/**
 * SVG Path Parser for ViewScript
 *
 * Parses SVG `d` attribute strings and converts them to the JSON format
 * expected by WASM FontRegistry.tessellate_svg_path().
 *
 * Supports all SVG path commands:
 * - M/m (moveto)
 * - L/l (lineto)
 * - H/h (horizontal lineto)
 * - V/v (vertical lineto)
 * - Q/q (quadratic bezier)
 * - T/t (smooth quadratic bezier)
 * - C/c (cubic bezier)
 * - S/s (smooth cubic bezier)
 * - A/a (arc)
 * - Z/z (closepath)
 */

export interface PathCommand {
  type: 'M' | 'L' | 'Q' | 'C' | 'A' | 'Z';
  x?: number;
  y?: number;
  x1?: number;
  y1?: number;
  x2?: number;
  y2?: number;
  rx?: number;
  ry?: number;
  rotation?: number;
  large_arc?: boolean;
  sweep?: boolean;
}

/**
 * Parse an SVG path `d` attribute into an array of path commands.
 *
 * @param d - The SVG path data string
 * @returns Array of PathCommand objects suitable for WASM tessellation
 */
export function parseSvgPath(d: string): PathCommand[] {
  const commands: PathCommand[] = [];

  // Current position
  let cx = 0;
  let cy = 0;

  // Start of current subpath (for Z command)
  let sx = 0;
  let sy = 0;

  // Previous control point (for smooth curves)
  let prevQx = 0;
  let prevQy = 0;
  let prevCx2 = 0;
  let prevCy2 = 0;
  let prevCmd = '';

  // Tokenize the path data
  const tokens = tokenize(d);
  let i = 0;

  const getNumber = (): number => {
    if (i >= tokens.length) throw new Error('Unexpected end of path data');
    const num = parseFloat(tokens[i++]);
    if (isNaN(num)) throw new Error(`Expected number, got: ${tokens[i - 1]}`);
    return num;
  };

  const getFlag = (): boolean => {
    const num = getNumber();
    return num !== 0;
  };

  while (i < tokens.length) {
    const cmd = tokens[i];

    // If it's not a command letter, repeat the previous command
    if (!/^[MmLlHhVvQqTtCcSsAaZz]$/.test(cmd)) {
      if (!prevCmd) throw new Error(`Invalid path command: ${cmd}`);
      // For M/m, subsequent coordinates are treated as L/l
      if (prevCmd === 'M') {
        processCommand('L');
      } else if (prevCmd === 'm') {
        processCommand('l');
      } else {
        processCommand(prevCmd);
      }
      continue;
    }

    i++; // consume the command letter
    processCommand(cmd);
    prevCmd = cmd;
  }

  function processCommand(cmd: string): void {
    const isRelative = cmd === cmd.toLowerCase();

    switch (cmd.toUpperCase()) {
      case 'M': {
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x += cx;
          y += cy;
        }
        commands.push({ type: 'M', x, y });
        cx = x;
        cy = y;
        sx = x;
        sy = y;
        break;
      }

      case 'L': {
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x += cx;
          y += cy;
        }
        commands.push({ type: 'L', x, y });
        cx = x;
        cy = y;
        break;
      }

      case 'H': {
        let x = getNumber();
        if (isRelative) {
          x += cx;
        }
        commands.push({ type: 'L', x, y: cy });
        cx = x;
        break;
      }

      case 'V': {
        let y = getNumber();
        if (isRelative) {
          y += cy;
        }
        commands.push({ type: 'L', x: cx, y });
        cy = y;
        break;
      }

      case 'Q': {
        let x1 = getNumber();
        let y1 = getNumber();
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x1 += cx;
          y1 += cy;
          x += cx;
          y += cy;
        }
        commands.push({ type: 'Q', x1, y1, x, y });
        prevQx = x1;
        prevQy = y1;
        cx = x;
        cy = y;
        break;
      }

      case 'T': {
        // Smooth quadratic: reflect previous control point
        let x1: number;
        let y1: number;
        if (prevCmd === 'Q' || prevCmd === 'q' || prevCmd === 'T' || prevCmd === 't') {
          x1 = 2 * cx - prevQx;
          y1 = 2 * cy - prevQy;
        } else {
          x1 = cx;
          y1 = cy;
        }
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x += cx;
          y += cy;
        }
        commands.push({ type: 'Q', x1, y1, x, y });
        prevQx = x1;
        prevQy = y1;
        cx = x;
        cy = y;
        break;
      }

      case 'C': {
        let x1 = getNumber();
        let y1 = getNumber();
        let x2 = getNumber();
        let y2 = getNumber();
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x1 += cx;
          y1 += cy;
          x2 += cx;
          y2 += cy;
          x += cx;
          y += cy;
        }
        commands.push({ type: 'C', x1, y1, x2, y2, x, y });
        prevCx2 = x2;
        prevCy2 = y2;
        cx = x;
        cy = y;
        break;
      }

      case 'S': {
        // Smooth cubic: reflect previous control point
        let x1: number;
        let y1: number;
        if (prevCmd === 'C' || prevCmd === 'c' || prevCmd === 'S' || prevCmd === 's') {
          x1 = 2 * cx - prevCx2;
          y1 = 2 * cy - prevCy2;
        } else {
          x1 = cx;
          y1 = cy;
        }
        let x2 = getNumber();
        let y2 = getNumber();
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x2 += cx;
          y2 += cy;
          x += cx;
          y += cy;
        }
        commands.push({ type: 'C', x1, y1, x2, y2, x, y });
        prevCx2 = x2;
        prevCy2 = y2;
        cx = x;
        cy = y;
        break;
      }

      case 'A': {
        const rx = getNumber();
        const ry = getNumber();
        const rotation = getNumber();
        const largeArc = getFlag();
        const sweep = getFlag();
        let x = getNumber();
        let y = getNumber();
        if (isRelative) {
          x += cx;
          y += cy;
        }
        commands.push({
          type: 'A',
          rx,
          ry,
          rotation,
          large_arc: largeArc,
          sweep,
          x,
          y,
        });
        cx = x;
        cy = y;
        break;
      }

      case 'Z': {
        commands.push({ type: 'Z' });
        cx = sx;
        cy = sy;
        break;
      }
    }
  }

  return commands;
}

/**
 * Tokenize SVG path data into commands and numbers.
 */
function tokenize(d: string): string[] {
  const tokens: string[] = [];

  // Regex to match:
  // - Command letters: [MmLlHhVvQqTtCcSsAaZz]
  // - Numbers: optional sign, digits, optional decimal, optional exponent
  const regex = /([MmLlHhVvQqTtCcSsAaZz])|([+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)/g;

  let match: RegExpExecArray | null;
  while ((match = regex.exec(d)) !== null) {
    tokens.push(match[0]);
  }

  return tokens;
}

/**
 * Convert parsed path commands to JSON string for WASM tessellation.
 *
 * @param commands - Array of PathCommand objects
 * @returns JSON string suitable for FontRegistry.tessellate_svg_path()
 */
export function pathCommandsToJson(commands: PathCommand[]): string {
  return JSON.stringify(commands);
}

/**
 * Convenience function: parse SVG path and return JSON for WASM.
 *
 * @param d - The SVG path data string
 * @returns JSON string suitable for FontRegistry.tessellate_svg_path()
 */
export function svgPathToJson(d: string): string {
  return pathCommandsToJson(parseSvgPath(d));
}
