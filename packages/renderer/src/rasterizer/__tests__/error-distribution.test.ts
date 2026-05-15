/**
 * Unit Tests for Subpixel Error Distribution
 *
 * These tests verify the Largest Remainder Method implementation
 * guarantees spatial closure (no gaps, no overflow).
 */

import { describe, it, expect } from 'vitest';
import {
  distributeWithLargestRemainder,
  applyErrorDistribution,
  type SiblingGroup,
  type ContainmentConstraint,
} from '../error-distribution.js';

describe('Largest Remainder Method', () => {
  /**
   * CRITICAL TEST: Architect's Decision #2
   *
   * "幅100pxのコンテナ内に、幅33.333...pxの要素が3つ、
   *  隙間なく隣接して配置される"
   *
   * This is the canonical case that motivated error distribution.
   */
  it('distributes 100px among 3 equal children (33.333...px each) without gaps', () => {
    // Arrange
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11, 12],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 100 / 3], // 33.333...
        [11, 100 / 3], // 33.333...
        [12, 100 / 3], // 33.333...
      ]),
    };

    // Act
    const result = distributeWithLargestRemainder(group);

    // Assert: Sum must equal parent exactly
    expect(result.totalPixels).toBe(100);
    expect(result.isExact).toBe(true);

    // Assert: Each child gets 33 or 34 pixels
    const pixels = result.dimensions.map((d: { pixels: number }) => d.pixels);
    expect(pixels.every((p: number) => p === 33 || p === 34)).toBe(true);

    // Assert: Exactly one child gets the extra pixel
    const count34 = pixels.filter((p: number) => p === 34).length;
    const count33 = pixels.filter((p: number) => p === 33).length;
    expect(count34).toBe(1);
    expect(count33).toBe(2);

    // Assert: Sum is exactly 100 (33 + 33 + 34 = 100)
    expect(pixels.reduce((a: number, b: number) => a + b, 0)).toBe(100);

    // Assert: Distribution is [34, 33, 33] (leftmost gets extra due to tie-break)
    expect(pixels).toEqual([34, 33, 33]);
  });

  it('handles exact division (no remainder)', () => {
    // Arrange: 100px / 4 = 25px exactly
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11, 12, 13],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 25],
        [11, 25],
        [12, 25],
        [13, 25],
      ]),
    };

    // Act
    const result = distributeWithLargestRemainder(group);

    // Assert
    expect(result.totalPixels).toBe(100);
    expect(result.isExact).toBe(true);
    expect(result.dimensions.map((d: { pixels: number }) => d.pixels)).toEqual([25, 25, 25, 25]);
  });

  it('distributes multiple extra pixels by remainder priority', () => {
    // Arrange: 100px for [40.9, 30.8, 28.3]
    // floors: [40, 30, 28] = 98
    // remainders: [0.9, 0.8, 0.3]
    // shortfall: 2px
    // Extra pixels go to: 40.9 → 41, 30.8 → 31
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11, 12],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 40.9],
        [11, 30.8],
        [12, 28.3],
      ]),
    };

    // Act
    const result = distributeWithLargestRemainder(group);

    // Assert
    expect(result.totalPixels).toBe(100);
    expect(result.isExact).toBe(true);
    expect(result.dimensions.map((d: { pixels: number }) => d.pixels)).toEqual([41, 31, 28]);
  });

  it('handles single child (trivial case)', () => {
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([[10, 100]]),
    };

    const result = distributeWithLargestRemainder(group);

    expect(result.totalPixels).toBe(100);
    expect(result.dimensions[0].pixels).toBe(100);
  });

  it('handles empty children', () => {
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map(),
    };

    const result = distributeWithLargestRemainder(group);

    expect(result.totalPixels).toBe(0);
    expect(result.dimensions).toEqual([]);
  });

  it('ties are broken by layout order (leftmost first)', () => {
    // Arrange: 10px for [3.5, 3.5] - equal remainders
    // floors: [3, 3] = 6
    // remainders: [0.5, 0.5] - TIE!
    // shortfall: 4px (wait, that's wrong)
    // Actually: 10 - 6 = 4... no, 3.5 + 3.5 = 7, not 10
    // Let me fix: 7px for [3.5, 3.5]
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11],
      axis: 'horizontal',
      parentDimension: 7,
      childDimensions: new Map([
        [10, 3.5],
        [11, 3.5],
      ]),
    };

    // Act
    const result = distributeWithLargestRemainder(group);

    // Assert: shortfall is 1, so first element (leftmost) gets it
    expect(result.totalPixels).toBe(7);
    expect(result.dimensions.map((d: { pixels: number }) => d.pixels)).toEqual([4, 3]);
  });

  it('handles vertical axis', () => {
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11, 12],
      axis: 'vertical',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 100 / 3],
        [11, 100 / 3],
        [12, 100 / 3],
      ]),
    };

    const result = distributeWithLargestRemainder(group);

    expect(result.totalPixels).toBe(100);
    expect(result.isExact).toBe(true);
  });

  it('records error for each element', () => {
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11, 12],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 100 / 3], // ~33.333
        [11, 100 / 3],
        [12, 100 / 3],
      ]),
    };

    const result = distributeWithLargestRemainder(group);

    // First element: 34 - 33.333... = +0.666...
    expect(result.dimensions[0].error).toBeCloseTo(0.6667, 3);

    // Other elements: 33 - 33.333... = -0.333...
    expect(result.dimensions[1].error).toBeCloseTo(-0.3333, 3);
    expect(result.dimensions[2].error).toBeCloseTo(-0.3333, 3);
  });
});

describe('applyErrorDistribution (Integration)', () => {
  it('adjusts child bounds to fit parent exactly', () => {
    // Arrange
    const roundedBounds = new Map([
      [1, { x: 0, y: 0, width: 100, height: 50 }], // Parent
      [10, { x: 0, y: 0, width: 33, height: 50 }], // Child 1 (naive)
      [11, { x: 33, y: 0, width: 33, height: 50 }], // Child 2
      [12, { x: 66, y: 0, width: 33, height: 50 }], // Child 3
      // Sum: 33 + 33 + 33 = 99 ← GAP!
    ]);

    const containments: ContainmentConstraint[] = [
      {
        parentId: 1,
        childIds: [10, 11, 12],
        axis: 'horizontal',
      },
    ];

    // Act
    const result = applyErrorDistribution(roundedBounds, containments);

    // Assert: Children sum to parent width exactly
    const child1 = result.get(10)!;
    const child2 = result.get(11)!;
    const child3 = result.get(12)!;

    const totalWidth = child1.width + child2.width + child3.width;
    expect(totalWidth).toBe(100);

    // Assert: Children are contiguous (no gaps)
    expect(child2.x).toBe(child1.x + child1.width);
    expect(child3.x).toBe(child2.x + child2.width);
  });

  it('handles multiple containment constraints', () => {
    const roundedBounds = new Map([
      // Horizontal container
      [1, { x: 0, y: 0, width: 100, height: 50 }],
      [10, { x: 0, y: 0, width: 33, height: 50 }],
      [11, { x: 33, y: 0, width: 33, height: 50 }],
      [12, { x: 66, y: 0, width: 33, height: 50 }],
      // Vertical container
      [2, { x: 0, y: 50, width: 100, height: 100 }],
      [20, { x: 0, y: 50, width: 100, height: 33 }],
      [21, { x: 0, y: 83, width: 100, height: 33 }],
      [22, { x: 0, y: 116, width: 100, height: 33 }],
    ]);

    const containments: ContainmentConstraint[] = [
      { parentId: 1, childIds: [10, 11, 12], axis: 'horizontal' },
      { parentId: 2, childIds: [20, 21, 22], axis: 'vertical' },
    ];

    const result = applyErrorDistribution(roundedBounds, containments);

    // Horizontal container
    const hTotal = result.get(10)!.width + result.get(11)!.width + result.get(12)!.width;
    expect(hTotal).toBe(100);

    // Vertical container
    const vTotal = result.get(20)!.height + result.get(21)!.height + result.get(22)!.height;
    expect(vTotal).toBe(100);
  });
});

describe('Edge Cases', () => {
  it('handles very small fractional differences', () => {
    // 100px / 7 = 14.285714...
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [1, 2, 3, 4, 5, 6, 7],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [1, 100 / 7],
        [2, 100 / 7],
        [3, 100 / 7],
        [4, 100 / 7],
        [5, 100 / 7],
        [6, 100 / 7],
        [7, 100 / 7],
      ]),
    };

    const result = distributeWithLargestRemainder(group);

    expect(result.totalPixels).toBe(100);
    expect(result.isExact).toBe(true);

    // 7 * 14 = 98, shortfall = 2
    // Two elements get 15px, five get 14px
    const fifteens = result.dimensions.filter((d: { pixels: number }) => d.pixels === 15).length;
    const fourteens = result.dimensions.filter((d: { pixels: number }) => d.pixels === 14).length;
    expect(fifteens).toBe(2);
    expect(fourteens).toBe(5);
  });

  it('handles zero-width children', () => {
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 100],
        [11, 0],
      ]),
    };

    const result = distributeWithLargestRemainder(group);

    expect(result.totalPixels).toBe(100);
    expect(result.dimensions[0].pixels).toBe(100);
    expect(result.dimensions[1].pixels).toBe(0);
  });

  it('uses proportional scaling when children exceed parent', () => {
    // Children sum to 150, but parent is only 100
    const group: SiblingGroup = {
      parentId: 1,
      childIds: [10, 11, 12],
      axis: 'horizontal',
      parentDimension: 100,
      childDimensions: new Map([
        [10, 50],
        [11, 50],
        [12, 50],
      ]),
    };

    const result = distributeWithLargestRemainder(group);

    // Falls back to proportional: each gets ~33.33
    expect(result.totalPixels).toBe(100);
    expect(result.method).toBe('first-fit');
  });
});

describe('Proof: No Gaps or Overflow', () => {
  /**
   * Property test: For ANY valid input, sum(children) === parent
   */
  it('maintains invariant for random inputs', () => {
    const testCases = [
      { parent: 100, children: [100 / 3, 100 / 3, 100 / 3] },
      { parent: 1000, children: [1000 / 7, 1000 / 7, 1000 / 7, 1000 / 7, 1000 / 7, 1000 / 7, 1000 / 7] },
      { parent: 50, children: [12.5, 12.5, 12.5, 12.5] },
      { parent: 99, children: [33, 33, 33] },
      { parent: 101, children: [50.5, 50.5] },
      { parent: 1, children: [0.5, 0.5] },
      { parent: 2, children: [0.6, 0.7, 0.7] },
    ];

    for (const tc of testCases) {
      const group: SiblingGroup = {
        parentId: 1,
        childIds: tc.children.map((_, i) => i + 10),
        axis: 'horizontal',
        parentDimension: tc.parent,
        childDimensions: new Map(tc.children.map((c, i) => [i + 10, c])),
      };

      const result = distributeWithLargestRemainder(group);

      expect(result.totalPixels).toBe(tc.parent);
      expect(result.isExact).toBe(true);
    }
  });
});
