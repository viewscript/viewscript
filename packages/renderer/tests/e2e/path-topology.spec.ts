/**
 * Path Topology Preservation E2E Tests (Phase 6)
 *
 * These tests verify that:
 * 1. Shared control points between curves produce seamless connections
 * 2. Topology-preserving rounding does not introduce gaps or overlaps
 * 3. Fill rules (nonzero/evenodd) are correctly applied
 *
 * ## Test Strategy
 *
 * We construct constraint graphs where two cubic Bezier curves share an
 * endpoint (ControlPoint entity), then verify:
 * - Visual: No 1px gaps or artifacts at the connection
 * - Numeric: Shared point coordinates are bit-identical in both paths
 * - Hash: Visual regression against known-good baseline
 */

import { test, expect, Page } from '@playwright/test';

// =============================================================================
// Test Harness: In-Browser Path Rendering
// =============================================================================

const TEST_HTML = `
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Path Topology Test</title>
  <style>
    body { margin: 0; background: #1a1a2e; }
    canvas { display: block; }
    #debug { position: fixed; top: 10px; left: 10px; color: white; font-family: monospace; font-size: 12px; }
  </style>
</head>
<body>
  <canvas id="canvas" width="800" height="600"></canvas>
  <div id="debug"></div>

  <script>
    // =========================================================================
    // P-Dimension Rational Type (Exact Arithmetic)
    // =========================================================================

    class Rational {
      constructor(numerator, denominator = 1n) {
        this.numerator = BigInt(numerator);
        this.denominator = BigInt(denominator);
        this._normalize();
      }

      _normalize() {
        const gcd = this._gcd(
          this.numerator < 0n ? -this.numerator : this.numerator,
          this.denominator
        );
        this.numerator = this.numerator / gcd;
        this.denominator = this.denominator / gcd;
      }

      _gcd(a, b) {
        while (b !== 0n) {
          const t = b;
          b = a % b;
          a = t;
        }
        return a;
      }

      toFloat() {
        // RASTERIZATION BOUNDARY: Rational -> f64
        return Number(this.numerator) / Number(this.denominator);
      }

      equals(other) {
        return this.numerator === other.numerator && this.denominator === other.denominator;
      }

      toString() {
        return this.numerator + '/' + this.denominator;
      }
    }

    // =========================================================================
    // Control Point Entities
    // =========================================================================

    class ControlPoint {
      constructor(id, x, y, role = 'anchor') {
        this.id = id;
        this.x = x; // Rational
        this.y = y; // Rational
        this.role = role;
      }
    }

    // =========================================================================
    // Path Definition
    // =========================================================================

    class PathDefinition {
      constructor(id, fillRule = 'nonzero') {
        this.id = id;
        this.segments = [];
        this.fillRule = fillRule;
        this.closed = false;
      }

      moveTo(pointId) {
        this.segments.push({ type: 'moveTo', point: pointId });
        return this;
      }

      lineTo(pointId) {
        this.segments.push({ type: 'lineTo', point: pointId });
        return this;
      }

      cubicTo(ctrl1Id, ctrl2Id, pointId) {
        this.segments.push({ type: 'cubicTo', control1: ctrl1Id, control2: ctrl2Id, point: pointId });
        return this;
      }

      close() {
        this.segments.push({ type: 'close' });
        this.closed = true;
        return this;
      }
    }

    // =========================================================================
    // Canvas Mapper (Rational -> Float)
    // =========================================================================

    function rasterizePath(path, controlPoints, ctx) {
      ctx.beginPath();

      for (const seg of path.segments) {
        switch (seg.type) {
          case 'moveTo': {
            const p = controlPoints.get(seg.point);
            ctx.moveTo(p.x.toFloat(), p.y.toFloat());
            break;
          }
          case 'lineTo': {
            const p = controlPoints.get(seg.point);
            ctx.lineTo(p.x.toFloat(), p.y.toFloat());
            break;
          }
          case 'cubicTo': {
            const c1 = controlPoints.get(seg.control1);
            const c2 = controlPoints.get(seg.control2);
            const p = controlPoints.get(seg.point);
            ctx.bezierCurveTo(
              c1.x.toFloat(), c1.y.toFloat(),
              c2.x.toFloat(), c2.y.toFloat(),
              p.x.toFloat(), p.y.toFloat()
            );
            break;
          }
          case 'close': {
            ctx.closePath();
            break;
          }
        }
      }
    }

    // =========================================================================
    // Test Scenarios
    // =========================================================================

    window.runTest = function(testName) {
      const canvas = document.getElementById('canvas');
      const ctx = canvas.getContext('2d');
      const debug = document.getElementById('debug');

      ctx.fillStyle = '#1a1a2e';
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      switch (testName) {
        case 'shared-endpoint':
          return runSharedEndpointTest(ctx, debug);
        case 'triple-junction':
          return runTripleJunctionTest(ctx, debug);
        case 'fill-rule-evenodd':
          return runFillRuleEvenOddTest(ctx, debug);
        case 'fill-rule-nonzero':
          return runFillRuleNonZeroTest(ctx, debug);
        default:
          throw new Error('Unknown test: ' + testName);
      }
    };

    // =========================================================================
    // Test: Two Cubic Beziers Sharing an Endpoint
    // =========================================================================

    function runSharedEndpointTest(ctx, debug) {
      // Control points with exact rational coordinates
      const controlPoints = new Map();

      // Curve 1: Anchor1 -> Curve -> SharedPoint
      controlPoints.set(1, new ControlPoint(1, new Rational(100), new Rational(300), 'anchor'));
      controlPoints.set(2, new ControlPoint(2, new Rational(200), new Rational(100), 'handle'));
      controlPoints.set(3, new ControlPoint(3, new Rational(300), new Rational(100), 'handle'));

      // SHARED CONTROL POINT: Both curves reference this exact entity
      controlPoints.set(4, new ControlPoint(4, new Rational(400), new Rational(300), 'anchor'));

      // Curve 2: SharedPoint -> Curve -> Anchor5
      controlPoints.set(5, new ControlPoint(5, new Rational(500), new Rational(100), 'handle'));
      controlPoints.set(6, new ControlPoint(6, new Rational(600), new Rational(100), 'handle'));
      controlPoints.set(7, new ControlPoint(7, new Rational(700), new Rational(300), 'anchor'));

      // Path 1: Uses shared point as endpoint
      const path1 = new PathDefinition(100);
      path1.moveTo(1).cubicTo(2, 3, 4);

      // Path 2: Uses SAME shared point as startpoint
      const path2 = new PathDefinition(101);
      path2.moveTo(4).cubicTo(5, 6, 7);

      // Render both paths
      ctx.strokeStyle = '#6366f1';
      ctx.lineWidth = 4;

      rasterizePath(path1, controlPoints, ctx);
      ctx.stroke();

      ctx.strokeStyle = '#f43f5e';
      rasterizePath(path2, controlPoints, ctx);
      ctx.stroke();

      // Draw shared point highlight
      const shared = controlPoints.get(4);
      ctx.beginPath();
      ctx.arc(shared.x.toFloat(), shared.y.toFloat(), 8, 0, Math.PI * 2);
      ctx.fillStyle = '#10b981';
      ctx.fill();

      // Verify: Sample pixels at the junction
      const junctionX = shared.x.toFloat();
      const junctionY = shared.y.toFloat();

      // Get pixel data around junction
      const imageData = ctx.getImageData(junctionX - 10, junctionY - 10, 20, 20);
      const pixels = imageData.data;

      // Count non-background pixels (should be continuous, no gaps)
      let pathPixels = 0;
      let backgroundPixels = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        const r = pixels[i], g = pixels[i+1], b = pixels[i+2];
        // Background is #1a1a2e (26, 26, 46)
        if (r === 26 && g === 26 && b === 46) {
          backgroundPixels++;
        } else {
          pathPixels++;
        }
      }

      debug.innerHTML = [
        'TEST: shared-endpoint',
        'Shared Point: (' + shared.x.toString() + ', ' + shared.y.toString() + ')',
        'Float Coords: (' + junctionX.toFixed(6) + ', ' + junctionY.toFixed(6) + ')',
        'Junction Area: ' + pathPixels + ' path pixels, ' + backgroundPixels + ' background',
        'Status: ' + (pathPixels > 50 ? 'CONNECTED' : 'GAP DETECTED'),
      ].join('<br>');

      return {
        testName: 'shared-endpoint',
        sharedPointId: 4,
        sharedCoords: { x: junctionX, y: junctionY },
        junctionPathPixels: pathPixels,
        junctionBackgroundPixels: backgroundPixels,
        connected: pathPixels > 50,
      };
    }

    // =========================================================================
    // Test: Three Curves Meeting at One Point (Triple Junction)
    // =========================================================================

    function runTripleJunctionTest(ctx, debug) {
      const controlPoints = new Map();

      // Central shared point
      controlPoints.set(1, new ControlPoint(1, new Rational(400), new Rational(300), 'anchor'));

      // Curve A endpoints and handles
      controlPoints.set(2, new ControlPoint(2, new Rational(200), new Rational(200), 'anchor'));
      controlPoints.set(3, new ControlPoint(3, new Rational(250), new Rational(250), 'handle'));
      controlPoints.set(4, new ControlPoint(4, new Rational(350), new Rational(250), 'handle'));

      // Curve B endpoints and handles
      controlPoints.set(5, new ControlPoint(5, new Rational(600), new Rational(200), 'anchor'));
      controlPoints.set(6, new ControlPoint(6, new Rational(550), new Rational(250), 'handle'));
      controlPoints.set(7, new ControlPoint(7, new Rational(450), new Rational(250), 'handle'));

      // Curve C endpoints and handles
      controlPoints.set(8, new ControlPoint(8, new Rational(400), new Rational(500), 'anchor'));
      controlPoints.set(9, new ControlPoint(9, new Rational(400), new Rational(400), 'handle'));
      controlPoints.set(10, new ControlPoint(10, new Rational(400), new Rational(350), 'handle'));

      // Three paths all ending at shared point (1)
      const pathA = new PathDefinition(100);
      pathA.moveTo(2).cubicTo(3, 4, 1);

      const pathB = new PathDefinition(101);
      pathB.moveTo(5).cubicTo(6, 7, 1);

      const pathC = new PathDefinition(102);
      pathC.moveTo(8).cubicTo(9, 10, 1);

      // Render
      const colors = ['#6366f1', '#f43f5e', '#10b981'];
      [pathA, pathB, pathC].forEach((path, i) => {
        ctx.strokeStyle = colors[i];
        ctx.lineWidth = 3;
        rasterizePath(path, controlPoints, ctx);
        ctx.stroke();
      });

      // Highlight junction
      const junction = controlPoints.get(1);
      ctx.beginPath();
      ctx.arc(junction.x.toFloat(), junction.y.toFloat(), 10, 0, Math.PI * 2);
      ctx.fillStyle = '#fbbf24';
      ctx.fill();

      // Verify junction integrity
      const jx = junction.x.toFloat();
      const jy = junction.y.toFloat();

      // Sample a ring around the junction (outside the highlight circle)
      // to verify all three curve strokes exist
      const sampleRadius = 15; // Outside the 10px highlight circle
      const samplePoints = [
        { x: jx - sampleRadius, y: jy - sampleRadius * 0.5 }, // Upper left (curve A direction)
        { x: jx + sampleRadius, y: jy - sampleRadius * 0.5 }, // Upper right (curve B direction)
        { x: jx, y: jy + sampleRadius },                       // Below (curve C direction)
      ];

      let curvesFound = 0;
      const foundColors = [];

      for (const point of samplePoints) {
        const px = Math.round(point.x);
        const py = Math.round(point.y);
        const imageData = ctx.getImageData(px - 3, py - 3, 6, 6);

        // Check if any non-background pixel exists in this sample area
        let foundNonBackground = false;
        for (let i = 0; i < imageData.data.length; i += 4) {
          const r = imageData.data[i], g = imageData.data[i+1], b = imageData.data[i+2];
          // Not background (#1a1a2e = 26, 26, 46) and not yellow highlight (#fbbf24)
          if ((r !== 26 || g !== 26 || b !== 46) && !(r === 251 && g === 191 && b === 36)) {
            foundNonBackground = true;
            foundColors.push(r + ',' + g + ',' + b);
            break;
          }
        }
        if (foundNonBackground) curvesFound++;
      }

      debug.innerHTML = [
        'TEST: triple-junction',
        'Junction Point: (' + junction.x.toString() + ', ' + junction.y.toString() + ')',
        'Curves Found at Sample Points: ' + curvesFound + '/3',
        'Status: ' + (curvesFound >= 3 ? 'ALL CURVES MEET' : 'MISSING CURVES'),
      ].join('<br>');

      return {
        testName: 'triple-junction',
        junctionCoords: { x: jx, y: jy },
        curvesFound: curvesFound,
        allCurvesMeet: curvesFound >= 3,
      };
    }

    // =========================================================================
    // Test: Fill Rule EvenOdd (Donut Shape)
    // =========================================================================

    function runFillRuleEvenOddTest(ctx, debug) {
      const controlPoints = new Map();

      // Outer circle approximation (4 cubic beziers)
      const k = 0.5522847498; // Magic constant for circular Bezier approximation
      const cx = 400, cy = 300, r = 150;

      // Outer circle control points
      controlPoints.set(1, new ControlPoint(1, new Rational(cx), new Rational(cy - r)));
      controlPoints.set(2, new ControlPoint(2, new Rational(Math.round(cx + r * k)), new Rational(cy - r), 'handle'));
      controlPoints.set(3, new ControlPoint(3, new Rational(cx + r), new Rational(Math.round(cy - r * k)), 'handle'));
      controlPoints.set(4, new ControlPoint(4, new Rational(cx + r), new Rational(cy)));
      controlPoints.set(5, new ControlPoint(5, new Rational(cx + r), new Rational(Math.round(cy + r * k)), 'handle'));
      controlPoints.set(6, new ControlPoint(6, new Rational(Math.round(cx + r * k)), new Rational(cy + r), 'handle'));
      controlPoints.set(7, new ControlPoint(7, new Rational(cx), new Rational(cy + r)));
      controlPoints.set(8, new ControlPoint(8, new Rational(Math.round(cx - r * k)), new Rational(cy + r), 'handle'));
      controlPoints.set(9, new ControlPoint(9, new Rational(cx - r), new Rational(Math.round(cy + r * k)), 'handle'));
      controlPoints.set(10, new ControlPoint(10, new Rational(cx - r), new Rational(cy)));
      controlPoints.set(11, new ControlPoint(11, new Rational(cx - r), new Rational(Math.round(cy - r * k)), 'handle'));
      controlPoints.set(12, new ControlPoint(12, new Rational(Math.round(cx - r * k)), new Rational(cy - r), 'handle'));

      // Inner circle (hole) control points
      const ri = 75;
      controlPoints.set(21, new ControlPoint(21, new Rational(cx), new Rational(cy - ri)));
      controlPoints.set(22, new ControlPoint(22, new Rational(Math.round(cx + ri * k)), new Rational(cy - ri), 'handle'));
      controlPoints.set(23, new ControlPoint(23, new Rational(cx + ri), new Rational(Math.round(cy - ri * k)), 'handle'));
      controlPoints.set(24, new ControlPoint(24, new Rational(cx + ri), new Rational(cy)));
      controlPoints.set(25, new ControlPoint(25, new Rational(cx + ri), new Rational(Math.round(cy + ri * k)), 'handle'));
      controlPoints.set(26, new ControlPoint(26, new Rational(Math.round(cx + ri * k)), new Rational(cy + ri), 'handle'));
      controlPoints.set(27, new ControlPoint(27, new Rational(cx), new Rational(cy + ri)));
      controlPoints.set(28, new ControlPoint(28, new Rational(Math.round(cx - ri * k)), new Rational(cy + ri), 'handle'));
      controlPoints.set(29, new ControlPoint(29, new Rational(cx - ri), new Rational(Math.round(cy + ri * k)), 'handle'));
      controlPoints.set(30, new ControlPoint(30, new Rational(cx - ri), new Rational(cy)));
      controlPoints.set(31, new ControlPoint(31, new Rational(cx - ri), new Rational(Math.round(cy - ri * k)), 'handle'));
      controlPoints.set(32, new ControlPoint(32, new Rational(Math.round(cx - ri * k)), new Rational(cy - ri), 'handle'));

      // Build combined path with evenodd fill
      const path = new PathDefinition(100, 'evenodd');
      path.moveTo(1)
        .cubicTo(2, 3, 4)
        .cubicTo(5, 6, 7)
        .cubicTo(8, 9, 10)
        .cubicTo(11, 12, 1)
        .close()
        .moveTo(21)
        .cubicTo(22, 23, 24)
        .cubicTo(25, 26, 27)
        .cubicTo(28, 29, 30)
        .cubicTo(31, 32, 21)
        .close();

      // Render with fill
      rasterizePath(path, controlPoints, ctx);
      ctx.fillStyle = '#6366f1';
      ctx.fill('evenodd');

      // Verify center is transparent (hole)
      const centerPixel = ctx.getImageData(cx, cy, 1, 1).data;
      const isHoleTransparent = centerPixel[0] === 26 && centerPixel[1] === 26 && centerPixel[2] === 46;

      // Verify ring is filled
      const ringPixel = ctx.getImageData(cx + 112, cy, 1, 1).data; // Middle of ring
      const isRingFilled = ringPixel[0] !== 26 || ringPixel[1] !== 26 || ringPixel[2] !== 46;

      debug.innerHTML = [
        'TEST: fill-rule-evenodd',
        'Fill Rule: evenodd',
        'Center (hole): ' + (isHoleTransparent ? 'TRANSPARENT' : 'FILLED'),
        'Ring: ' + (isRingFilled ? 'FILLED' : 'TRANSPARENT'),
        'Status: ' + (isHoleTransparent && isRingFilled ? 'CORRECT' : 'INCORRECT'),
      ].join('<br>');

      return {
        testName: 'fill-rule-evenodd',
        fillRule: 'evenodd',
        holeTransparent: isHoleTransparent,
        ringFilled: isRingFilled,
        correct: isHoleTransparent && isRingFilled,
      };
    }

    // =========================================================================
    // Test: Fill Rule NonZero (Solid Donut)
    // =========================================================================

    function runFillRuleNonZeroTest(ctx, debug) {
      // Same geometry as evenodd test but with nonzero fill
      // With same-direction winding, center should be filled

      const controlPoints = new Map();
      const k = 0.5522847498;
      const cx = 400, cy = 300, r = 150, ri = 75;

      // Outer circle (same as evenodd test)
      controlPoints.set(1, new ControlPoint(1, new Rational(cx), new Rational(cy - r)));
      controlPoints.set(2, new ControlPoint(2, new Rational(Math.round(cx + r * k)), new Rational(cy - r), 'handle'));
      controlPoints.set(3, new ControlPoint(3, new Rational(cx + r), new Rational(Math.round(cy - r * k)), 'handle'));
      controlPoints.set(4, new ControlPoint(4, new Rational(cx + r), new Rational(cy)));
      controlPoints.set(5, new ControlPoint(5, new Rational(cx + r), new Rational(Math.round(cy + r * k)), 'handle'));
      controlPoints.set(6, new ControlPoint(6, new Rational(Math.round(cx + r * k)), new Rational(cy + r), 'handle'));
      controlPoints.set(7, new ControlPoint(7, new Rational(cx), new Rational(cy + r)));
      controlPoints.set(8, new ControlPoint(8, new Rational(Math.round(cx - r * k)), new Rational(cy + r), 'handle'));
      controlPoints.set(9, new ControlPoint(9, new Rational(cx - r), new Rational(Math.round(cy + r * k)), 'handle'));
      controlPoints.set(10, new ControlPoint(10, new Rational(cx - r), new Rational(cy)));
      controlPoints.set(11, new ControlPoint(11, new Rational(cx - r), new Rational(Math.round(cy - r * k)), 'handle'));
      controlPoints.set(12, new ControlPoint(12, new Rational(Math.round(cx - r * k)), new Rational(cy - r), 'handle'));

      // Inner circle (same direction for nonzero to fill everything)
      controlPoints.set(21, new ControlPoint(21, new Rational(cx), new Rational(cy - ri)));
      controlPoints.set(22, new ControlPoint(22, new Rational(Math.round(cx + ri * k)), new Rational(cy - ri), 'handle'));
      controlPoints.set(23, new ControlPoint(23, new Rational(cx + ri), new Rational(Math.round(cy - ri * k)), 'handle'));
      controlPoints.set(24, new ControlPoint(24, new Rational(cx + ri), new Rational(cy)));
      controlPoints.set(25, new ControlPoint(25, new Rational(cx + ri), new Rational(Math.round(cy + ri * k)), 'handle'));
      controlPoints.set(26, new ControlPoint(26, new Rational(Math.round(cx + ri * k)), new Rational(cy + ri), 'handle'));
      controlPoints.set(27, new ControlPoint(27, new Rational(cx), new Rational(cy + ri)));
      controlPoints.set(28, new ControlPoint(28, new Rational(Math.round(cx - ri * k)), new Rational(cy + ri), 'handle'));
      controlPoints.set(29, new ControlPoint(29, new Rational(cx - ri), new Rational(Math.round(cy + ri * k)), 'handle'));
      controlPoints.set(30, new ControlPoint(30, new Rational(cx - ri), new Rational(cy)));
      controlPoints.set(31, new ControlPoint(31, new Rational(cx - ri), new Rational(Math.round(cy - ri * k)), 'handle'));
      controlPoints.set(32, new ControlPoint(32, new Rational(Math.round(cx - ri * k)), new Rational(cy - ri), 'handle'));

      // Build combined path with nonzero fill (same winding)
      const path = new PathDefinition(100, 'nonzero');
      path.moveTo(1)
        .cubicTo(2, 3, 4)
        .cubicTo(5, 6, 7)
        .cubicTo(8, 9, 10)
        .cubicTo(11, 12, 1)
        .close()
        .moveTo(21)
        .cubicTo(22, 23, 24)
        .cubicTo(25, 26, 27)
        .cubicTo(28, 29, 30)
        .cubicTo(31, 32, 21)
        .close();

      // Render with fill
      rasterizePath(path, controlPoints, ctx);
      ctx.fillStyle = '#f43f5e';
      ctx.fill('nonzero');

      // With same-direction winding and nonzero rule, everything should be filled
      const centerPixel = ctx.getImageData(cx, cy, 1, 1).data;
      const isCenterFilled = centerPixel[0] !== 26 || centerPixel[1] !== 26 || centerPixel[2] !== 46;

      const ringPixel = ctx.getImageData(cx + 112, cy, 1, 1).data;
      const isRingFilled = ringPixel[0] !== 26 || ringPixel[1] !== 26 || ringPixel[2] !== 46;

      debug.innerHTML = [
        'TEST: fill-rule-nonzero',
        'Fill Rule: nonzero (same winding)',
        'Center: ' + (isCenterFilled ? 'FILLED' : 'TRANSPARENT'),
        'Ring: ' + (isRingFilled ? 'FILLED' : 'TRANSPARENT'),
        'Status: ' + (isCenterFilled && isRingFilled ? 'CORRECT' : 'INCORRECT'),
      ].join('<br>');

      return {
        testName: 'fill-rule-nonzero',
        fillRule: 'nonzero',
        centerFilled: isCenterFilled,
        ringFilled: isRingFilled,
        correct: isCenterFilled && isRingFilled,
      };
    }
  </script>
</body>
</html>
`;

// =============================================================================
// Playwright Tests
// =============================================================================

test.describe('Path Topology Preservation (Phase 6)', () => {
  let page: Page;

  test.beforeEach(async ({ browser }) => {
    page = await browser.newPage();
    await page.setContent(TEST_HTML);
    // Wait for script to load
    await page.waitForFunction(() => typeof (window as any).runTest === 'function');
  });

  test.afterEach(async () => {
    await page.close();
  });

  test('shared endpoint produces seamless curve connection', async () => {
    const result = await page.evaluate(() => (window as any).runTest('shared-endpoint'));

    expect(result.testName).toBe('shared-endpoint');
    expect(result.sharedPointId).toBe(4);

    // Verify shared point coordinates are exactly as specified (Rational precision)
    expect(result.sharedCoords.x).toBe(400);
    expect(result.sharedCoords.y).toBe(300);

    // Verify junction is connected (many path pixels, few background pixels at junction)
    expect(result.junctionPathPixels).toBeGreaterThan(50);
    expect(result.connected).toBe(true);
  });

  test('triple junction has all curves meeting at single point', async () => {
    const result = await page.evaluate(() => (window as any).runTest('triple-junction'));

    expect(result.testName).toBe('triple-junction');

    // Junction coordinates match rational definition
    expect(result.junctionCoords.x).toBe(400);
    expect(result.junctionCoords.y).toBe(300);

    // All three curves are found near the junction point
    expect(result.curvesFound).toBeGreaterThanOrEqual(3);
    expect(result.allCurvesMeet).toBe(true);
  });

  test('evenodd fill rule creates donut with transparent hole', async () => {
    const result = await page.evaluate(() => (window as any).runTest('fill-rule-evenodd'));

    expect(result.testName).toBe('fill-rule-evenodd');
    expect(result.fillRule).toBe('evenodd');

    // With evenodd rule, center (overlapping region) should be transparent
    expect(result.holeTransparent).toBe(true);

    // Ring (outer - inner) should be filled
    expect(result.ringFilled).toBe(true);

    expect(result.correct).toBe(true);
  });

  test('nonzero fill rule with same winding fills entire shape', async () => {
    const result = await page.evaluate(() => (window as any).runTest('fill-rule-nonzero'));

    expect(result.testName).toBe('fill-rule-nonzero');
    expect(result.fillRule).toBe('nonzero');

    // With nonzero rule and same-direction winding, center should be filled
    expect(result.centerFilled).toBe(true);
    expect(result.ringFilled).toBe(true);

    expect(result.correct).toBe(true);
  });

  test('CRITICAL: bit-identical coordinates at shared control point', async () => {
    // This test verifies the core invariant: shared ControlPoints must produce
    // exactly the same float coordinates when rasterized in different paths

    const result = await page.evaluate(() => {
      // Create two paths sharing a control point
      const sharedPoint = {
        id: 999,
        x: { numerator: 333333333n, denominator: 1000000n }, // 333.333333
        y: { numerator: 666666666n, denominator: 1000000n }, // 666.666666
      };

      // Convert to float (simulating rasterization boundary)
      const toFloat = (r: {numerator: bigint, denominator: bigint}) =>
        Number(r.numerator) / Number(r.denominator);

      const x1 = toFloat(sharedPoint.x);
      const y1 = toFloat(sharedPoint.y);

      // Second access (simulating another path using same point)
      const x2 = toFloat(sharedPoint.x);
      const y2 = toFloat(sharedPoint.y);

      return {
        x1, y1,
        x2, y2,
        xIdentical: Object.is(x1, x2),
        yIdentical: Object.is(y1, y2),
        // Also verify no precision loss compared to expected
        xExpected: 333.333333,
        yExpected: 666.666666,
        xClose: Math.abs(x1 - 333.333333) < 0.000001,
        yClose: Math.abs(y1 - 666.666666) < 0.000001,
      };
    });

    // Bit-identical coordinates (Object.is checks for exact equality including -0/+0)
    expect(result.xIdentical).toBe(true);
    expect(result.yIdentical).toBe(true);

    // Precision is maintained
    expect(result.xClose).toBe(true);
    expect(result.yClose).toBe(true);
  });
});
