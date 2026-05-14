/**
 * Gradient Shader Mapper: P-Dimension to GPU Shaders
 *
 * This module maps P-dimension gradient entities to GPU shader objects.
 * It handles the critical transition from exact rational arithmetic to GPU-compatible
 * floating-point representation while preserving visual fidelity.
 *
 * ## Architecture
 *
 * ```
 *   P-Dimension (Exact)                        GPU (Float)
 *   ─────────────────────────────────────────────────────────────
 *
 *   LinearGradient {                           GpuShaderBackend.Shader
 *     start: Rational(1, 3)    ────────────▶   MakeLinearGradient(
 *     end: Rational(2, 3)                        [0.333..., 0.666...],
 *     stops: [                                   colors: Float32Array,
 *       ColorStop(r=255, ...)                    positions: Float32Array
 *     ]                                        )
 *   }
 *
 *   ┌─────────────────────────────────────────────────────────────┐
 *   │  CRITICAL: Topology-preserving rounding at this boundary   │
 *   │                                                             │
 *   │  - Color channels [0, 255] → [0.0, 1.0] with clamping      │
 *   │  - Position values [0, 1] stay exact, no interpolation     │
 *   │  - Control points use same rounding as other coordinates   │
 *   └─────────────────────────────────────────────────────────────┘
 * ```
 *
 * ## Usage
 *
 * ```typescript
 * const shader = mapLinearGradientToShader(ck, gradient, bounds, dpr);
 * paint.setShader(shader);
 * canvas.drawRect(bounds, paint);
 * ```
 */

import type { Rational, RasterBounds } from '../ast/types';

// =============================================================================
// Types
// =============================================================================

/** GPU shader backend interface (minimal subset needed for gradients) */
export interface GpuShaderBackend {
  Shader: {
    MakeLinearGradient(
      start: Float32Array,
      end: Float32Array,
      colors: Float32Array,
      positions: Float32Array | null,
      mode: number,
      localMatrix?: Float32Array,
    ): ShaderInstance;

    MakeTwoPointConicalGradient(
      start: Float32Array,
      startRadius: number,
      end: Float32Array,
      endRadius: number,
      colors: Float32Array,
      positions: Float32Array | null,
      mode: number,
      localMatrix?: Float32Array,
    ): ShaderInstance;

    MakeSweepGradient(
      cx: number,
      cy: number,
      colors: Float32Array,
      positions: Float32Array | null,
      mode: number,
      startAngle: number,
      endAngle: number,
      localMatrix?: Float32Array,
    ): ShaderInstance;
  };

  TileMode: {
    Clamp: number;
    Repeat: number;
    Mirror: number;
    Decal: number;
  };
}

export interface ShaderInstance {
  delete(): void;
}

/**
 * P-dimension color stop with exact rational values.
 */
export interface PColorStop {
  /** Entity ID */
  id: number;
  /** Red channel [0, 255] (rational) */
  r: Rational;
  /** Green channel [0, 255] (rational) */
  g: Rational;
  /** Blue channel [0, 255] (rational) */
  b: Rational;
  /** Alpha channel [0, 1] (rational) */
  a: Rational;
  /** Position along gradient [0, 1] (rational) */
  position: Rational;
}

/**
 * P-dimension control point with exact coordinates.
 */
export interface PControlPoint {
  id: number;
  x: Rational;
  y: Rational;
}

/**
 * P-dimension linear gradient definition.
 */
export interface PLinearGradient {
  id: number;
  start: PControlPoint;
  end: PControlPoint;
  stops: PColorStop[];
  tileMode: 'clamp' | 'repeat' | 'mirror' | 'decal';
}

/**
 * P-dimension radial gradient definition.
 */
export interface PRadialGradient {
  id: number;
  center: PControlPoint;
  radiusX: Rational;
  radiusY: Rational;
  focalPoint?: PControlPoint;
  focalRadius?: Rational;
  stops: PColorStop[];
  tileMode: 'clamp' | 'repeat' | 'mirror' | 'decal';
}

/**
 * P-dimension conic (sweep) gradient definition.
 */
export interface PConicGradient {
  id: number;
  center: PControlPoint;
  /** Rotation offset in degrees (rational) */
  rotation: Rational;
  /** Start angle in degrees (rational) */
  startAngle: Rational;
  /** End angle in degrees (rational) */
  endAngle: Rational;
  stops: PColorStop[];
}

// =============================================================================
// Core Mapping Functions
// =============================================================================

/**
 * Map a P-dimension linear gradient to a GPU shader.
 *
 * @param ck - GPU shader backend instance
 * @param gradient - P-dimension linear gradient definition
 * @param bounds - Rasterized bounds for coordinate transformation
 * @param devicePixelRatio - Device pixel ratio for coordinate scaling
 * @returns GPU shader instance (caller must call delete() when done)
 */
export function mapLinearGradientToShader(
  ck: GpuShaderBackend,
  gradient: PLinearGradient,
  bounds: RasterBounds,
  devicePixelRatio: number,
): ShaderInstance {
  // Convert control points to device coordinates
  const startX = rationalToFloat(gradient.start.x) * devicePixelRatio;
  const startY = rationalToFloat(gradient.start.y) * devicePixelRatio;
  const endX = rationalToFloat(gradient.end.x) * devicePixelRatio;
  const endY = rationalToFloat(gradient.end.y) * devicePixelRatio;

  // Sort stops by position (required by GPU shader backend)
  const sortedStops = [...gradient.stops].sort(
    (a, b) => rationalToFloat(a.position) - rationalToFloat(b.position),
  );

  // Convert colors to Float32Array (RGBA format, each channel 0-1)
  const colors = colorStopsToFloat32Array(sortedStops);
  const positions = positionsToFloat32Array(sortedStops);

  // Map tile mode
  const tileMode = mapTileMode(ck, gradient.tileMode);

  return ck.Shader.MakeLinearGradient(
    new Float32Array([startX, startY]),
    new Float32Array([endX, endY]),
    colors,
    positions,
    tileMode,
  );
}

/**
 * Map a P-dimension radial gradient to a GPU shader.
 *
 * The GPU backend uses two-point conical gradients which can express:
 * - Circle gradients (same center, different radii)
 * - Focal gradients (different centers)
 *
 * @param ck - GPU shader backend instance
 * @param gradient - P-dimension radial gradient definition
 * @param bounds - Rasterized bounds for coordinate transformation
 * @param devicePixelRatio - Device pixel ratio for coordinate scaling
 * @returns GPU shader instance
 */
export function mapRadialGradientToShader(
  ck: GpuShaderBackend,
  gradient: PRadialGradient,
  bounds: RasterBounds,
  devicePixelRatio: number,
): ShaderInstance {
  // Convert center to device coordinates
  const centerX = rationalToFloat(gradient.center.x) * devicePixelRatio;
  const centerY = rationalToFloat(gradient.center.y) * devicePixelRatio;

  // For elliptical gradients, we use the larger radius and scale
  // For now, use average of radiusX and radiusY as the radius
  const radiusX = rationalToFloat(gradient.radiusX) * devicePixelRatio;
  const radiusY = rationalToFloat(gradient.radiusY) * devicePixelRatio;
  const radius = Math.max(radiusX, radiusY);

  // Handle focal point (if specified)
  let focalX = centerX;
  let focalY = centerY;
  let focalR = 0;

  if (gradient.focalPoint) {
    focalX = rationalToFloat(gradient.focalPoint.x) * devicePixelRatio;
    focalY = rationalToFloat(gradient.focalPoint.y) * devicePixelRatio;
  }
  if (gradient.focalRadius) {
    focalR = rationalToFloat(gradient.focalRadius) * devicePixelRatio;
  }

  // Sort and convert stops
  const sortedStops = [...gradient.stops].sort(
    (a, b) => rationalToFloat(a.position) - rationalToFloat(b.position),
  );
  const colors = colorStopsToFloat32Array(sortedStops);
  const positions = positionsToFloat32Array(sortedStops);

  const tileMode = mapTileMode(ck, gradient.tileMode);

  // Two-point conical: inner circle to outer circle
  return ck.Shader.MakeTwoPointConicalGradient(
    new Float32Array([focalX, focalY]),
    focalR,
    new Float32Array([centerX, centerY]),
    radius,
    colors,
    positions,
    tileMode,
  );
}

/**
 * Map a P-dimension conic (sweep) gradient to a GPU shader.
 *
 * @param ck - GPU shader backend instance
 * @param gradient - P-dimension conic gradient definition
 * @param bounds - Rasterized bounds for coordinate transformation
 * @param devicePixelRatio - Device pixel ratio for coordinate scaling
 * @returns GPU shader instance
 */
export function mapConicGradientToShader(
  ck: GpuShaderBackend,
  gradient: PConicGradient,
  bounds: RasterBounds,
  devicePixelRatio: number,
): ShaderInstance {
  // Convert center to device coordinates
  const centerX = rationalToFloat(gradient.center.x) * devicePixelRatio;
  const centerY = rationalToFloat(gradient.center.y) * devicePixelRatio;

  // Convert angles (GPU shader expects degrees)
  const startAngle = rationalToFloat(gradient.startAngle) + rationalToFloat(gradient.rotation);
  const endAngle = rationalToFloat(gradient.endAngle) + rationalToFloat(gradient.rotation);

  // Sort and convert stops
  const sortedStops = [...gradient.stops].sort(
    (a, b) => rationalToFloat(a.position) - rationalToFloat(b.position),
  );
  const colors = colorStopsToFloat32Array(sortedStops);
  const positions = positionsToFloat32Array(sortedStops);

  // Sweep gradient always uses Clamp-like behavior
  return ck.Shader.MakeSweepGradient(
    centerX,
    centerY,
    colors,
    positions,
    ck.TileMode.Clamp,
    startAngle,
    endAngle,
  );
}

// =============================================================================
// Conversion Utilities
// =============================================================================

/**
 * Convert a rational number to a floating-point number.
 *
 * This is the critical boundary where exact arithmetic meets GPU floats.
 * The conversion is straightforward division, but precision loss is
 * unavoidable and acceptable at this layer.
 */
export function rationalToFloat(r: Rational): number {
  // Handle bigint to number conversion
  // For very large rationals, this may lose precision
  return Number(r.numerator) / Number(r.denominator);
}

/**
 * Convert P-dimension color stops to a Float32Array of RGBA values.
 *
 * GPU shader expects colors in RGBA order, with each channel in [0, 1].
 * P-dimension stores RGB in [0, 255] and Alpha in [0, 1].
 *
 * ## Topology-Preserving Rounding (Clamping)
 *
 * Color values are clamped to [0, 1] to ensure GPU-valid input.
 * This preserves the topological ordering of colors even if the
 * original rational values were slightly out of range.
 */
export function colorStopsToFloat32Array(stops: PColorStop[]): Float32Array {
  const array = new Float32Array(stops.length * 4);

  for (let i = 0; i < stops.length; i++) {
    const stop = stops[i];
    const offset = i * 4;

    // Convert [0, 255] rational to [0, 1] float with clamping
    array[offset + 0] = clamp01(rationalToFloat(stop.r) / 255);
    array[offset + 1] = clamp01(rationalToFloat(stop.g) / 255);
    array[offset + 2] = clamp01(rationalToFloat(stop.b) / 255);

    // Alpha is already in [0, 1] in P-dimension
    array[offset + 3] = clamp01(rationalToFloat(stop.a));
  }

  return array;
}

/**
 * Convert P-dimension color stop positions to a Float32Array.
 *
 * Positions are kept as-is (already in [0, 1] in P-dimension),
 * with clamping for safety.
 */
export function positionsToFloat32Array(stops: PColorStop[]): Float32Array {
  const array = new Float32Array(stops.length);

  for (let i = 0; i < stops.length; i++) {
    array[i] = clamp01(rationalToFloat(stops[i].position));
  }

  return array;
}

/**
 * Clamp a value to [0, 1] range.
 *
 * This is the "topology-preserving rounding" for color values:
 * it ensures the value is valid for GPU while preserving ordering.
 */
function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

/**
 * Map P-dimension tile mode to GPU tile mode constant.
 */
function mapTileMode(
  ck: GpuShaderBackend,
  mode: 'clamp' | 'repeat' | 'mirror' | 'decal',
): number {
  switch (mode) {
    case 'clamp':
      return ck.TileMode.Clamp;
    case 'repeat':
      return ck.TileMode.Repeat;
    case 'mirror':
      return ck.TileMode.Mirror;
    case 'decal':
      return ck.TileMode.Decal;
    default:
      return ck.TileMode.Clamp;
  }
}

// =============================================================================
// Factory Function for Gradient FillStyle
// =============================================================================

/**
 * Create a GPU shader from a FillStyle gradient definition.
 *
 * This is a higher-level factory that integrates with the existing
 * FillStyle type from the AST.
 */
export function createGradientShader(
  ck: GpuShaderBackend,
  fillType: 'linear-gradient' | 'radial-gradient',
  stops: Array<{ offset: Rational; color: string }>,
  bounds: RasterBounds,
  devicePixelRatio: number,
): ShaderInstance | null {
  // Convert simplified FillStyle stops to PColorStop format
  const pStops: PColorStop[] = stops.map((stop, index) => {
    const rgba = parseColorString(stop.color);
    return {
      id: index,
      r: { numerator: BigInt(rgba.r), denominator: 1n },
      g: { numerator: BigInt(rgba.g), denominator: 1n },
      b: { numerator: BigInt(rgba.b), denominator: 1n },
      a: { numerator: BigInt(Math.round(rgba.a * 1000)), denominator: 1000n },
      position: stop.offset,
    };
  });

  if (fillType === 'linear-gradient') {
    // Default linear gradient: top to bottom
    const linearGradient: PLinearGradient = {
      id: 0,
      start: {
        id: 0,
        x: { numerator: BigInt(Math.round(bounds.x)), denominator: 1n },
        y: { numerator: BigInt(Math.round(bounds.y)), denominator: 1n },
      },
      end: {
        id: 0,
        x: { numerator: BigInt(Math.round(bounds.x)), denominator: 1n },
        y: { numerator: BigInt(Math.round(bounds.y + bounds.height)), denominator: 1n },
      },
      stops: pStops,
      tileMode: 'clamp',
    };
    return mapLinearGradientToShader(ck, linearGradient, bounds, devicePixelRatio);
  } else if (fillType === 'radial-gradient') {
    // Default radial gradient: center of bounds, radius to edge
    const cx = bounds.x + bounds.width / 2;
    const cy = bounds.y + bounds.height / 2;
    const radius = Math.max(bounds.width, bounds.height) / 2;

    const radialGradient: PRadialGradient = {
      id: 0,
      center: {
        id: 0,
        x: { numerator: BigInt(Math.round(cx * 1000)), denominator: 1000n },
        y: { numerator: BigInt(Math.round(cy * 1000)), denominator: 1000n },
      },
      radiusX: { numerator: BigInt(Math.round(radius * 1000)), denominator: 1000n },
      radiusY: { numerator: BigInt(Math.round(radius * 1000)), denominator: 1000n },
      stops: pStops,
      tileMode: 'clamp',
    };
    return mapRadialGradientToShader(ck, radialGradient, bounds, devicePixelRatio);
  }

  return null;
}

/**
 * Parse a CSS color string to RGBA values.
 *
 * Supports:
 * - Hex: #RGB, #RGBA, #RRGGBB, #RRGGBBAA
 * - Named colors (basic set)
 */
function parseColorString(color: string): { r: number; g: number; b: number; a: number } {
  // Named colors (basic set)
  const namedColors: Record<string, { r: number; g: number; b: number }> = {
    black: { r: 0, g: 0, b: 0 },
    white: { r: 255, g: 255, b: 255 },
    red: { r: 255, g: 0, b: 0 },
    green: { r: 0, g: 128, b: 0 },
    blue: { r: 0, g: 0, b: 255 },
    yellow: { r: 255, g: 255, b: 0 },
    cyan: { r: 0, g: 255, b: 255 },
    magenta: { r: 255, g: 0, b: 255 },
    transparent: { r: 0, g: 0, b: 0 },
  };

  const lower = color.toLowerCase().trim();

  if (lower === 'transparent') {
    return { r: 0, g: 0, b: 0, a: 0 };
  }

  if (namedColors[lower]) {
    return { ...namedColors[lower], a: 1 };
  }

  // Hex parsing
  if (lower.startsWith('#')) {
    const hex = lower.slice(1);

    if (hex.length === 3) {
      // #RGB
      return {
        r: parseInt(hex[0] + hex[0], 16),
        g: parseInt(hex[1] + hex[1], 16),
        b: parseInt(hex[2] + hex[2], 16),
        a: 1,
      };
    } else if (hex.length === 4) {
      // #RGBA
      return {
        r: parseInt(hex[0] + hex[0], 16),
        g: parseInt(hex[1] + hex[1], 16),
        b: parseInt(hex[2] + hex[2], 16),
        a: parseInt(hex[3] + hex[3], 16) / 255,
      };
    } else if (hex.length === 6) {
      // #RRGGBB
      return {
        r: parseInt(hex.slice(0, 2), 16),
        g: parseInt(hex.slice(2, 4), 16),
        b: parseInt(hex.slice(4, 6), 16),
        a: 1,
      };
    } else if (hex.length === 8) {
      // #RRGGBBAA
      return {
        r: parseInt(hex.slice(0, 2), 16),
        g: parseInt(hex.slice(2, 4), 16),
        b: parseInt(hex.slice(4, 6), 16),
        a: parseInt(hex.slice(6, 8), 16) / 255,
      };
    }
  }

  // Fallback: black
  return { r: 0, g: 0, b: 0, a: 1 };
}

// =============================================================================
// Exports for Testing
// =============================================================================

export const _internals = {
  rationalToFloat,
  clamp01,
  colorStopsToFloat32Array,
  positionsToFloat32Array,
  mapTileMode,
  parseColorString,
};
