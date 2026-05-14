/**
 * vs-parser.ts Unit Tests
 */

import { describe, it, expect } from 'vitest';
import { parseVsFile, validateImports, type VsParseResult } from './vs-parser.js';

describe('vs-parser', () => {
  // ===========================================================================
  // Import Statement Tests
  // ===========================================================================

  describe('import parsing', () => {
    it('parses single import', () => {
      const content = `import { clamp } from "./math.ts"`;
      const result = parseVsFile(content);

      expect(result.imports).toHaveLength(1);
      expect(result.imports[0]).toEqual({
        names: ['clamp'],
        modulePath: './math.ts',
        line: 1,
      });
    });

    it('parses multiple named imports', () => {
      const content = `import { clamp, lerp, smoothstep } from "./utils/math.ts"`;
      const result = parseVsFile(content);

      expect(result.imports).toHaveLength(1);
      expect(result.imports[0].names).toEqual(['clamp', 'lerp', 'smoothstep']);
      expect(result.imports[0].modulePath).toBe('./utils/math.ts');
    });

    it('handles import with alias (ignores alias)', () => {
      const content = `import { clamp as c, lerp } from "./math.ts"`;
      const result = parseVsFile(content);

      expect(result.imports[0].names).toEqual(['clamp', 'lerp']);
    });

    it('parses imports with single quotes', () => {
      const content = `import { notify } from './events.ts'`;
      const result = parseVsFile(content);

      expect(result.imports[0].modulePath).toBe('./events.ts');
    });

    it('parses multiple import statements', () => {
      const content = `
import { clamp } from "./math.ts"
import { notify, log } from "./events.ts"
      `;
      const result = parseVsFile(content);

      expect(result.imports).toHaveLength(2);
      expect(result.imports[0].names).toEqual(['clamp']);
      expect(result.imports[1].names).toEqual(['notify', 'log']);
    });
  });

  // ===========================================================================
  // Q Bind Tests
  // ===========================================================================

  describe('q bind parsing', () => {
    it('parses bind with single argument', () => {
      const content = `q bind opacity = lerp(hover_progress)`;
      const result = parseVsFile(content);

      expect(result.binds).toHaveLength(1);
      expect(result.binds[0]).toEqual({
        bindName: 'opacity',
        functionName: 'lerp',
        args: ['hover_progress'],
        line: 1,
      });
    });

    it('parses bind with multiple arguments', () => {
      const content = `q bind clamped = clamp(value, 0, 1)`;
      const result = parseVsFile(content);

      expect(result.binds[0].args).toEqual(['value', '0', '1']);
    });

    it('parses bind with no arguments', () => {
      const content = `q bind timestamp = now()`;
      const result = parseVsFile(content);

      expect(result.binds[0].args).toEqual([]);
    });

    it('handles whitespace variations', () => {
      const content = `q  bind   spaced  =  func(  a,  b  )`;
      const result = parseVsFile(content);

      expect(result.binds[0].bindName).toBe('spaced');
      expect(result.binds[0].functionName).toBe('func');
      expect(result.binds[0].args).toEqual(['a', 'b']);
    });
  });

  // ===========================================================================
  // Q Trigger Tests
  // ===========================================================================

  describe('q trigger parsing', () => {
    it('parses trigger with condition and action', () => {
      const content = `q trigger on_collision = bounds_overlap(rect_1, circle_1) -> notify(collision_data)`;
      const result = parseVsFile(content);

      expect(result.triggers).toHaveLength(1);
      expect(result.triggers[0]).toEqual({
        triggerName: 'on_collision',
        conditionKind: 'bounds_overlap',
        conditionArgs: ['rect_1', 'circle_1'],
        functionName: 'notify',
        functionArgs: ['collision_data'],
        line: 1,
      });
    });

    it('parses trigger with no action arguments', () => {
      const content = `q trigger on_click = bounds_overlap(button, cursor) -> handleClick()`;
      const result = parseVsFile(content);

      expect(result.triggers[0].functionArgs).toEqual([]);
    });

    it('parses trigger with multiple action arguments', () => {
      const content = `q trigger on_hover = bounds_overlap(a, b) -> log(entity_a, entity_b, timestamp)`;
      const result = parseVsFile(content);

      expect(result.triggers[0].functionArgs).toEqual(['entity_a', 'entity_b', 'timestamp']);
    });

    it('parses properties_equal condition with dot notation', () => {
      const content = `q trigger sync_x = properties_equal(player.x, target.x) -> onSync()`;
      const result = parseVsFile(content);

      expect(result.triggers).toHaveLength(1);
      expect(result.triggers[0].conditionKind).toBe('properties_equal');
      expect(result.triggers[0].conditionArgs).toEqual(['player.x', 'target.x']);
    });

    it('parses property_less_than condition', () => {
      const content = `q trigger below = property_less_than(ball.y, floor.y) -> onLand(ball)`;
      const result = parseVsFile(content);

      expect(result.triggers[0].conditionKind).toBe('property_less_than');
      expect(result.triggers[0].conditionArgs).toEqual(['ball.y', 'floor.y']);
      expect(result.triggers[0].functionArgs).toEqual(['ball']);
    });

    it('parses threshold_crossing condition with numeric threshold', () => {
      const content = `q trigger ground_hit = threshold_crossing(player.y, 100, falling) -> playSound(impact)`;
      const result = parseVsFile(content);

      expect(result.triggers[0].conditionKind).toBe('threshold_crossing');
      expect(result.triggers[0].conditionArgs).toEqual(['player.y', '100', 'falling']);
      expect(result.triggers[0].functionName).toBe('playSound');
    });

    it('parses threshold_crossing with decimal threshold', () => {
      const content = `q trigger opacity_fade = threshold_crossing(box.t, 0.5, rising) -> onFadeIn()`;
      const result = parseVsFile(content);

      expect(result.triggers[0].conditionArgs).toEqual(['box.t', '0.5', 'rising']);
    });

    it('parses threshold_crossing with rational string', () => {
      const content = `q trigger halfway = threshold_crossing(progress.x, 1/2, rising) -> onHalf()`;
      const result = parseVsFile(content);

      expect(result.triggers[0].conditionArgs).toEqual(['progress.x', '1/2', 'rising']);
    });
  });

  // ===========================================================================
  // Mixed Content Tests
  // ===========================================================================

  describe('mixed content parsing', () => {
    it('parses complete .vs file with all declaration types', () => {
      const content = `
// Math utilities
import { clamp, lerp } from "./utils/math.ts"
import { notify } from "./events.ts"

// Component definition
export component Button {
  param width: 100
  param height: 50

  // Q-dimension bindings
  q bind opacity = clamp(hover_progress, 0, 1)
  q bind scale = lerp(pressed, 1, 0.95)

  // Collision triggers
  q trigger on_hover = bounds_overlap(self, cursor) -> notify(hover_event)
}
      `;
      const result = parseVsFile(content);

      expect(result.imports).toHaveLength(2);
      expect(result.binds).toHaveLength(2);
      expect(result.triggers).toHaveLength(1);
      expect(result.errors).toHaveLength(0);
    });

    it('ignores comments', () => {
      const content = `
// This is a comment
import { clamp } from "./math.ts"
// q bind should_not_parse = fake()
      `;
      const result = parseVsFile(content);

      expect(result.imports).toHaveLength(1);
      expect(result.binds).toHaveLength(0);
    });

    it('ignores empty lines', () => {
      const content = `

import { a } from "./a.ts"

q bind x = a()

      `;
      const result = parseVsFile(content);

      expect(result.imports).toHaveLength(1);
      expect(result.binds).toHaveLength(1);
    });
  });

  // ===========================================================================
  // Error Handling Tests
  // ===========================================================================

  describe('error handling', () => {
    it('reports invalid q declaration', () => {
      const content = `q invalid syntax here`;
      const result = parseVsFile(content);

      expect(result.errors).toHaveLength(1);
      expect(result.errors[0].message).toContain('Invalid q declaration');
      expect(result.errors[0].line).toBe(1);
    });

    it('reports malformed q bind', () => {
      const content = `q bind missing_equals func()`;
      const result = parseVsFile(content);

      expect(result.errors).toHaveLength(1);
    });

    it('reports malformed q trigger', () => {
      const content = `q trigger missing_arrow = condition(a, b) notify()`;
      const result = parseVsFile(content);

      expect(result.errors).toHaveLength(1);
    });
  });

  // ===========================================================================
  // Import Validation Tests
  // ===========================================================================

  describe('validateImports', () => {
    it('returns no errors when all functions are imported', () => {
      const result: VsParseResult = {
        imports: [{ names: ['clamp', 'notify'], modulePath: './utils.ts', line: 1 }],
        binds: [{ bindName: 'x', functionName: 'clamp', args: [], line: 2 }],
        triggers: [
          {
            triggerName: 't',
            conditionKind: 'bounds_overlap',
            conditionArgs: [],
            functionName: 'notify',
            functionArgs: [],
            line: 3,
          },
        ],
        errors: [],
      };

      const errors = validateImports(result);
      expect(errors).toHaveLength(0);
    });

    it('reports missing import for bind function', () => {
      const result: VsParseResult = {
        imports: [],
        binds: [{ bindName: 'x', functionName: 'clamp', args: [], line: 2 }],
        triggers: [],
        errors: [],
      };

      const errors = validateImports(result);
      expect(errors).toHaveLength(1);
      expect(errors[0].message).toContain("'clamp'");
      expect(errors[0].message).toContain('not imported');
    });

    it('reports missing import for trigger function', () => {
      const result: VsParseResult = {
        imports: [],
        binds: [],
        triggers: [
          {
            triggerName: 't',
            conditionKind: 'bounds_overlap',
            conditionArgs: [],
            functionName: 'notify',
            functionArgs: [],
            line: 3,
          },
        ],
        errors: [],
      };

      const errors = validateImports(result);
      expect(errors).toHaveLength(1);
      expect(errors[0].message).toContain("'notify'");
    });
  });

  // ===========================================================================
  // Line Number Tests
  // ===========================================================================

  describe('line numbers', () => {
    it('tracks correct line numbers', () => {
      const content = `
import { a } from "./a.ts"

q bind x = a()

q trigger t = bounds_overlap(x, y) -> a()
      `;
      const result = parseVsFile(content);

      expect(result.imports[0].line).toBe(2);
      expect(result.binds[0].line).toBe(4);
      expect(result.triggers[0].line).toBe(6);
    });
  });

  // ===========================================================================
  // New TypeScript AST Parser Tests (function-call syntax)
  // ===========================================================================

  describe('TypeScript AST parsing', () => {
    describe('export default object literal', () => {
      it('parses rect component declaration', () => {
        const content = `
export default {
  bg: rect({ x: 100, y: 200, width: 320, height: 200 }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls).toHaveLength(1);
        expect(result.componentDecls[0].name).toBe('bg');
        expect(result.componentDecls[0].type).toBe('rect');
        expect(result.componentDecls[0].properties.x).toEqual({ kind: 'literal', value: 100 });
        expect(result.componentDecls[0].properties.y).toEqual({ kind: 'literal', value: 200 });
      });

      it('parses text component with string property', () => {
        const content = `
export default {
  label: text({ content: "Hello", fill: "#cdd6f4" }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls).toHaveLength(1);
        expect(result.componentDecls[0].type).toBe('text');
        expect(result.componentDecls[0].properties.content).toEqual({ kind: 'literal', value: 'Hello' });
        expect(result.componentDecls[0].properties.fill).toEqual({ kind: 'literal', value: '#cdd6f4' });
      });

      it('parses property reference (bg.x)', () => {
        const content = `
export default {
  bg: rect({ x: 100 }),
  label: text({ x: bg.x }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls).toHaveLength(2);
        expect(result.componentDecls[1].properties.x).toEqual({
          kind: 'reference',
          entity: 'bg',
          component: 'x',
        });
      });

      it('parses expression (bg.x + bg.width / 2)', () => {
        const content = `
export default {
  label: text({ x: bg.x + bg.width / 2 }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls).toHaveLength(1);
        const xProp = result.componentDecls[0].properties.x;
        expect(xProp.kind).toBe('expression');
        if (xProp.kind === 'expression') {
          expect(xProp.ast.type).toBe('binary');
          expect(xProp.source).toContain('bg.x + bg.width / 2');
        }
      });

      it('parses FFI function reference (content: getCount)', () => {
        const content = `
import { getCount } from "./logic"

export default {
  label: text({ content: getCount }),
}`;
        const result = parseVsFile(content);

        expect(result.imports).toHaveLength(1);
        expect(result.imports[0].names).toEqual(['getCount']);
        expect(result.componentDecls).toHaveLength(1);
        expect(result.componentDecls[0].properties.content).toEqual({
          kind: 'reference',
          entity: 'getCount',
        });
      });

      it('parses FFI function call (clamp(value, 0, 1))', () => {
        const content = `
export default {
  box: rect({ opacity: clamp(progress, 0, 1) }),
}`;
        const result = parseVsFile(content);

        const opacityProp = result.componentDecls[0].properties.opacity;
        expect(opacityProp.kind).toBe('ffiCall');
        if (opacityProp.kind === 'ffiCall') {
          expect(opacityProp.functionName).toBe('clamp');
          expect(opacityProp.args).toHaveLength(3);
        }
      });

      it('parses interactive: true flag', () => {
        const content = `
export default {
  btn: rect({ x: 100, interactive: true }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls[0].interactive).toBe(true);
      });

      it('parses onClick event binding', () => {
        const content = `
export default {
  btn: rect({
    x: 100,
    interactive: true,
    onClick: { type: 'increment', target: 'count', delta: 1 },
  }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls[0].eventBindings).toHaveLength(1);
        const binding = result.componentDecls[0].eventBindings[0];
        expect(binding.event).toBe('click');
        expect(binding.action).toEqual({
          type: 'increment',
          target: 'count',
          delta: 1,
        });
      });

      it('parses toggle event action', () => {
        const content = `
export default {
  toggle: rect({
    onClick: { type: 'toggle', target: 'visible', values: [0, 1] },
  }),
}`;
        const result = parseVsFile(content);

        const binding = result.componentDecls[0].eventBindings[0];
        expect(binding.action).toEqual({
          type: 'toggle',
          target: 'visible',
          values: [0, 1],
        });
      });

      it('parses call event action with handler', () => {
        const content = `
export default {
  btn: rect({
    onClick: { type: 'call', handler: 'onSubmit' },
  }),
}`;
        const result = parseVsFile(content);

        const binding = result.componentDecls[0].eventBindings[0];
        expect(binding.action).toEqual({
          type: 'call',
          handler: 'onSubmit',
        });
      });

      it('parses multiple components', () => {
        const content = `
export default {
  bg: rect({ x: 0, y: 0, width: 800, height: 600 }),
  panel: rect({ x: 100, y: 100, width: 300, height: 200 }),
  title: text({ x: 150, content: "Hello" }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls).toHaveLength(3);
        expect(result.componentDecls.map(c => c.name)).toEqual(['bg', 'panel', 'title']);
      });

      it('parses negative numbers', () => {
        const content = `
export default {
  box: rect({ x: -100, y: -50 }),
}`;
        const result = parseVsFile(content);

        expect(result.componentDecls[0].properties.x).toEqual({ kind: 'literal', value: -100 });
      });
    });

    describe('expression AST generation', () => {
      it('generates correct AST for simple addition', () => {
        const content = `
export default {
  box: rect({ x: a.x + 10 }),
}`;
        const result = parseVsFile(content);

        const xProp = result.componentDecls[0].properties.x;
        expect(xProp.kind).toBe('expression');
        if (xProp.kind === 'expression') {
          expect(xProp.ast).toEqual({
            type: 'binary',
            op: '+',
            left: { type: 'ref', entity: 'a', component: 'x' },
            right: { type: 'const', value: 10 },
          });
        }
      });

      it('generates correct AST for division', () => {
        const content = `
export default {
  box: rect({ x: width / 2 }),
}`;
        const result = parseVsFile(content);

        const xProp = result.componentDecls[0].properties.x;
        expect(xProp.kind).toBe('expression');
        if (xProp.kind === 'expression') {
          expect(xProp.ast.type).toBe('binary');
          if (xProp.ast.type === 'binary') {
            expect(xProp.ast.op).toBe('/');
          }
        }
      });

      it('generates correct AST for complex expression', () => {
        const content = `
export default {
  label: text({ x: (panel.x + panel.width) / 2 }),
}`;
        const result = parseVsFile(content);

        const xProp = result.componentDecls[0].properties.x;
        expect(xProp.kind).toBe('expression');
        if (xProp.kind === 'expression') {
          // (panel.x + panel.width) / 2
          expect(xProp.ast.type).toBe('binary');
          if (xProp.ast.type === 'binary') {
            expect(xProp.ast.op).toBe('/');
            expect(xProp.ast.left.type).toBe('binary'); // panel.x + panel.width
            expect(xProp.ast.right).toEqual({ type: 'const', value: 2 });
          }
        }
      });
    });
  });
});
