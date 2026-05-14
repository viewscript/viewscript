/**
 * Semantic Translator for P/Q Dimension Bridge (Phase 7: Task 20)
 *
 * This module converts raw solver output (VarId -> Rational maps) into
 * entity-level coordinate descriptions suitable for LLM comprehension.
 *
 * ## Purpose
 *
 * The P-dimension solver produces solutions in the form:
 *   { "1:x": "100/1", "1:y": "200/1", "2:x": "300/1", ... }
 *
 * This is not LLM-friendly. The SemanticTranslator converts this to:
 *   {
 *     entities: [
 *       { entityId: 1, coordinates: { x: 100, y: 200 }, description: "Point at (100, 200)" },
 *       { entityId: 2, coordinates: { x: 300, y: 150 }, description: "Point at (300, 150)" }
 *     ],
 *     relationships: [
 *       { type: "distance", from: 1, to: 2, value: "~212 units" }
 *     ]
 *   }
 *
 * ## Axiom 2: Ouroboros Binding
 *
 * This module sits at the P/Q dimension boundary. It translates deterministic
 * solver output (P) into natural language suitable for non-deterministic
 * oracles (Q/LLM).
 *
 * ## Usage
 *
 * ```typescript
 * const translator = new SemanticTranslator(entityRegistry);
 * const semanticSolution = translator.translateSolution(rawSolution);
 * const diff = translator.compareSolutions(solution1, solution2);
 * ```
 */

import type { EntityId, Rational } from '../ast/types';

// =============================================================================
// Types for Raw Solver Output
// =============================================================================

/**
 * Raw solution from solver: "entity_id:component" -> "numerator/denominator"
 */
export type RawSolution = Map<string, string>;

/**
 * Parsed VarId key.
 */
export interface ParsedVarId {
  entityId: EntityId;
  component: VectorComponent;
}

/**
 * Vector components matching Rust VectorComponent enum.
 */
export type VectorComponent =
  | 'x'
  | 'y'
  | 'z'
  | 't'
  | 'value'
  | 'r'
  | 'g'
  | 'b'
  | 'alpha'
  | 'position';

// =============================================================================
// Entity Metadata Registry
// =============================================================================

/**
 * Entity type classification for semantic descriptions.
 */
export type EntityType =
  | 'point'
  | 'control_point'
  | 'rect'
  | 'circle'
  | 'text'
  | 'color_stop'
  | 'scalar'
  | 'unknown';

/**
 * Metadata about an entity (provided by the constraint graph).
 */
export interface EntityMetadata {
  entityId: EntityId;
  type: EntityType;
  name?: string;
  parentId?: EntityId;
}

/**
 * Registry of entity metadata for semantic translation.
 */
export interface EntityRegistry {
  get(entityId: EntityId): EntityMetadata | undefined;
  getAll(): EntityMetadata[];
}

// =============================================================================
// Semantic Output Types
// =============================================================================

/**
 * Coordinates for a spatial entity.
 */
export interface EntityCoordinates {
  x?: number;
  y?: number;
  z?: number;
  t?: number;
}

/**
 * Color values for a color stop entity.
 */
export interface EntityColor {
  r?: number;
  g?: number;
  b?: number;
  alpha?: number;
  position?: number;
}

/**
 * Scalar value for value-only entities.
 */
export interface EntityScalar {
  value?: number;
}

/**
 * A semantic entity description.
 */
export interface SemanticEntity {
  entityId: EntityId;
  type: EntityType;
  name?: string;

  /** Spatial coordinates (for points, rects, etc.) */
  coordinates?: EntityCoordinates;

  /** Color values (for color stops) */
  color?: EntityColor;

  /** Scalar value (for radius, angle, etc.) */
  scalar?: EntityScalar;

  /** Human-readable description for LLM */
  description: string;
}

/**
 * Relationship between entities derived from geometry.
 */
export interface SemanticRelationship {
  type: 'distance' | 'alignment' | 'containment' | 'collinear';
  fromEntityId: EntityId;
  toEntityId: EntityId;
  description: string;
  value?: string;
}

/**
 * Complete semantic solution.
 */
export interface SemanticSolution {
  /** Unique identifier for this solution (index in MultipleSolutions) */
  solutionIndex: number;

  /** All entities with resolved coordinates */
  entities: SemanticEntity[];

  /** Derived relationships between entities */
  relationships: SemanticRelationship[];

  /** High-level summary for LLM consumption */
  summary: string;
}

/**
 * Difference between two solutions.
 */
export interface SolutionDiff {
  /** Entities that differ between solutions */
  differingEntities: EntityDiff[];

  /** Human-readable diff summary */
  summary: string;
}

export interface EntityDiff {
  entityId: EntityId;
  name?: string;
  solution1: SemanticEntity;
  solution2: SemanticEntity;
  description: string;
}

// =============================================================================
// SemanticTranslator Implementation
// =============================================================================

/**
 * Translates raw solver output to LLM-friendly semantic descriptions.
 */
export class SemanticTranslator {
  constructor(private readonly registry: EntityRegistry) {}

  /**
   * Parse a raw VarId key (e.g., "5:x") into its components.
   */
  parseVarId(key: string): ParsedVarId | null {
    const [entityStr, component] = key.split(':');
    if (!entityStr || !component) return null;

    const entityId = parseInt(entityStr, 10);
    if (isNaN(entityId)) return null;

    const validComponents: VectorComponent[] = [
      'x', 'y', 'z', 't', 'value', 'r', 'g', 'b', 'alpha', 'position',
    ];
    if (!validComponents.includes(component as VectorComponent)) return null;

    return { entityId, component: component as VectorComponent };
  }

  /**
   * Parse a rational string (e.g., "100/1" or "100") to a number.
   *
   * WARNING: This converts P-dimension exact rationals to Q-dimension floats.
   * Only use for display/LLM consumption, NEVER for constraint solving.
   */
  parseRationalToFloat(value: string): number {
    const parts = value.split('/');
    if (parts.length === 2) {
      const num = parseFloat(parts[0]);
      const denom = parseFloat(parts[1]);
      if (denom === 0) return NaN;
      return num / denom;
    }
    return parseFloat(value);
  }

  /**
   * Translate a raw solution to a semantic solution.
   */
  translateSolution(rawSolution: RawSolution, solutionIndex: number = 0): SemanticSolution {
    // Group by entity
    const entityMap = new Map<EntityId, Map<VectorComponent, number>>();

    for (const [key, value] of rawSolution) {
      const parsed = this.parseVarId(key);
      if (!parsed) continue;

      let components = entityMap.get(parsed.entityId);
      if (!components) {
        components = new Map();
        entityMap.set(parsed.entityId, components);
      }
      components.set(parsed.component, this.parseRationalToFloat(value));
    }

    // Build semantic entities
    const entities: SemanticEntity[] = [];
    for (const [entityId, components] of entityMap) {
      const metadata = this.registry.get(entityId);
      const entity = this.buildSemanticEntity(
        entityId,
        metadata?.type ?? 'unknown',
        metadata?.name,
        components,
      );
      entities.push(entity);
    }

    // Sort by entity ID for consistent output
    entities.sort((a, b) => a.entityId - b.entityId);

    // Derive relationships
    const relationships = this.deriveRelationships(entities);

    // Generate summary
    const summary = this.generateSummary(entities, relationships);

    return {
      solutionIndex,
      entities,
      relationships,
      summary,
    };
  }

  /**
   * Build a semantic entity from raw components.
   */
  private buildSemanticEntity(
    entityId: EntityId,
    type: EntityType,
    name: string | undefined,
    components: Map<VectorComponent, number>,
  ): SemanticEntity {
    const entity: SemanticEntity = {
      entityId,
      type,
      name,
      description: '',
    };

    // Build coordinates/color/scalar based on entity type
    if (type === 'color_stop') {
      entity.color = {
        r: components.get('r'),
        g: components.get('g'),
        b: components.get('b'),
        alpha: components.get('alpha'),
        position: components.get('position'),
      };
      entity.description = this.describeColorStop(entity.color, name);
    } else if (type === 'scalar') {
      entity.scalar = {
        value: components.get('value'),
      };
      entity.description = this.describeScalar(entity.scalar, name);
    } else {
      // Spatial entity
      entity.coordinates = {
        x: components.get('x'),
        y: components.get('y'),
        z: components.get('z'),
        t: components.get('t'),
      };
      entity.description = this.describeSpatialEntity(type, entity.coordinates, name);
    }

    return entity;
  }

  /**
   * Generate description for a spatial entity.
   */
  private describeSpatialEntity(
    type: EntityType,
    coords: EntityCoordinates,
    name?: string,
  ): string {
    const nameStr = name ? `"${name}" ` : '';
    const x = coords.x !== undefined ? this.formatCoord(coords.x) : '?';
    const y = coords.y !== undefined ? this.formatCoord(coords.y) : '?';

    switch (type) {
      case 'point':
      case 'control_point':
        return `${nameStr}${type === 'control_point' ? 'Control point' : 'Point'} at (${x}, ${y})`;
      case 'rect':
        return `${nameStr}Rectangle at (${x}, ${y})`;
      case 'circle':
        return `${nameStr}Circle centered at (${x}, ${y})`;
      case 'text':
        return `${nameStr}Text at (${x}, ${y})`;
      default:
        return `${nameStr}Entity ${type} at (${x}, ${y})`;
    }
  }

  /**
   * Generate description for a color stop.
   */
  private describeColorStop(color: EntityColor, name?: string): string {
    const nameStr = name ? `"${name}" ` : '';
    const r = color.r !== undefined ? Math.round(color.r) : '?';
    const g = color.g !== undefined ? Math.round(color.g) : '?';
    const b = color.b !== undefined ? Math.round(color.b) : '?';
    const pos = color.position !== undefined ? `${(color.position * 100).toFixed(0)}%` : '?';

    return `${nameStr}Color stop at ${pos}: rgb(${r}, ${g}, ${b})`;
  }

  /**
   * Generate description for a scalar entity.
   */
  private describeScalar(scalar: EntityScalar, name?: string): string {
    const nameStr = name ? `"${name}"` : 'Scalar';
    const val = scalar.value !== undefined ? this.formatCoord(scalar.value) : '?';

    return `${nameStr} = ${val}`;
  }

  /**
   * Format a coordinate value for display.
   */
  private formatCoord(value: number): string {
    // Round to reasonable precision for display
    if (Number.isInteger(value)) {
      return value.toString();
    }
    return value.toFixed(2).replace(/\.?0+$/, '');
  }

  /**
   * Derive geometric relationships between entities.
   */
  private deriveRelationships(entities: SemanticEntity[]): SemanticRelationship[] {
    const relationships: SemanticRelationship[] = [];

    // Find horizontally aligned entities (same Y)
    const byY = new Map<number, SemanticEntity[]>();
    for (const entity of entities) {
      if (entity.coordinates?.y !== undefined) {
        const y = Math.round(entity.coordinates.y);
        let list = byY.get(y);
        if (!list) {
          list = [];
          byY.set(y, list);
        }
        list.push(entity);
      }
    }

    for (const [y, aligned] of byY) {
      if (aligned.length >= 2) {
        const ids = aligned.map(e => e.entityId).join(', ');
        relationships.push({
          type: 'alignment',
          fromEntityId: aligned[0].entityId,
          toEntityId: aligned[aligned.length - 1].entityId,
          description: `Entities ${ids} are horizontally aligned at y=${y}`,
        });
      }
    }

    // Find vertically aligned entities (same X)
    const byX = new Map<number, SemanticEntity[]>();
    for (const entity of entities) {
      if (entity.coordinates?.x !== undefined) {
        const x = Math.round(entity.coordinates.x);
        let list = byX.get(x);
        if (!list) {
          list = [];
          byX.set(x, list);
        }
        list.push(entity);
      }
    }

    for (const [x, aligned] of byX) {
      if (aligned.length >= 2) {
        const ids = aligned.map(e => e.entityId).join(', ');
        relationships.push({
          type: 'alignment',
          fromEntityId: aligned[0].entityId,
          toEntityId: aligned[aligned.length - 1].entityId,
          description: `Entities ${ids} are vertically aligned at x=${x}`,
        });
      }
    }

    return relationships;
  }

  /**
   * Generate a high-level summary of the solution.
   */
  private generateSummary(
    entities: SemanticEntity[],
    relationships: SemanticRelationship[],
  ): string {
    const entityCounts = new Map<EntityType, number>();
    for (const entity of entities) {
      entityCounts.set(entity.type, (entityCounts.get(entity.type) ?? 0) + 1);
    }

    const parts: string[] = [];
    for (const [type, count] of entityCounts) {
      parts.push(`${count} ${type}${count > 1 ? 's' : ''}`);
    }

    let summary = `Solution with ${entities.length} entities: ${parts.join(', ')}.`;

    if (relationships.length > 0) {
      summary += ` ${relationships.length} geometric relationship${relationships.length > 1 ? 's' : ''} detected.`;
    }

    return summary;
  }

  /**
   * Compare two solutions and generate a diff.
   */
  compareSolutions(
    solution1: SemanticSolution,
    solution2: SemanticSolution,
  ): SolutionDiff {
    const diffs: EntityDiff[] = [];

    const entities1 = new Map(solution1.entities.map(e => [e.entityId, e]));
    const entities2 = new Map(solution2.entities.map(e => [e.entityId, e]));

    // Find entities that differ
    for (const [entityId, e1] of entities1) {
      const e2 = entities2.get(entityId);
      if (!e2) continue;

      if (!this.entitiesEqual(e1, e2)) {
        diffs.push({
          entityId,
          name: e1.name ?? e2.name,
          solution1: e1,
          solution2: e2,
          description: this.describeDiff(e1, e2),
        });
      }
    }

    const summary = diffs.length === 0
      ? 'Solutions are identical'
      : `${diffs.length} entities differ: ${diffs.map(d => d.entityId).join(', ')}`;

    return { differingEntities: diffs, summary };
  }

  /**
   * Check if two semantic entities are equal.
   */
  private entitiesEqual(e1: SemanticEntity, e2: SemanticEntity): boolean {
    const tolerance = 1e-6;

    if (e1.coordinates && e2.coordinates) {
      for (const key of ['x', 'y', 'z', 't'] as const) {
        const v1 = e1.coordinates[key];
        const v2 = e2.coordinates[key];
        if (v1 !== undefined && v2 !== undefined && Math.abs(v1 - v2) > tolerance) {
          return false;
        }
      }
    }

    if (e1.color && e2.color) {
      for (const key of ['r', 'g', 'b', 'alpha', 'position'] as const) {
        const v1 = e1.color[key];
        const v2 = e2.color[key];
        if (v1 !== undefined && v2 !== undefined && Math.abs(v1 - v2) > tolerance) {
          return false;
        }
      }
    }

    if (e1.scalar && e2.scalar) {
      const v1 = e1.scalar.value;
      const v2 = e2.scalar.value;
      if (v1 !== undefined && v2 !== undefined && Math.abs(v1 - v2) > tolerance) {
        return false;
      }
    }

    return true;
  }

  /**
   * Describe the difference between two entities.
   */
  private describeDiff(e1: SemanticEntity, e2: SemanticEntity): string {
    if (e1.coordinates && e2.coordinates) {
      const changes: string[] = [];
      if (e1.coordinates.x !== e2.coordinates.x) {
        changes.push(`x: ${this.formatCoord(e1.coordinates.x ?? 0)} -> ${this.formatCoord(e2.coordinates.x ?? 0)}`);
      }
      if (e1.coordinates.y !== e2.coordinates.y) {
        changes.push(`y: ${this.formatCoord(e1.coordinates.y ?? 0)} -> ${this.formatCoord(e2.coordinates.y ?? 0)}`);
      }
      return `Position changes: ${changes.join(', ')}`;
    }

    return 'Values differ';
  }

  /**
   * Translate multiple solutions and generate comparative descriptions.
   */
  translateMultipleSolutions(rawSolutions: RawSolution[]): {
    solutions: SemanticSolution[];
    comparison: string;
  } {
    const solutions = rawSolutions.map((raw, idx) => this.translateSolution(raw, idx));

    if (solutions.length < 2) {
      return {
        solutions,
        comparison: 'Only one solution available.',
      };
    }

    // Generate pairwise comparison for first two solutions
    const diff = this.compareSolutions(solutions[0], solutions[1]);

    let comparison = `${solutions.length} solutions found.\n\n`;
    comparison += `Solution 0 vs Solution 1:\n${diff.summary}\n\n`;

    for (const entityDiff of diff.differingEntities) {
      comparison += `  Entity ${entityDiff.entityId}: ${entityDiff.description}\n`;
    }

    return { solutions, comparison };
  }
}

// =============================================================================
// Utility: Create Entity Registry from Array
// =============================================================================

/**
 * Create an EntityRegistry from an array of metadata.
 */
export function createEntityRegistry(entities: EntityMetadata[]): EntityRegistry {
  const map = new Map(entities.map(e => [e.entityId, e]));
  return {
    get: (id: EntityId) => map.get(id),
    getAll: () => entities,
  };
}

/**
 * Create an empty EntityRegistry (all entities will be "unknown" type).
 */
export function createEmptyRegistry(): EntityRegistry {
  return {
    get: () => undefined,
    getAll: () => [],
  };
}
