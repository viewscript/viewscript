/**
 * Chunk Splitting Strategy for Progressive Loading
 *
 * This module implements static analysis to partition the constraint graph
 * into Initial and Lazy chunks, enabling Qwik/Wiz-style progressive hydration.
 *
 * ## Core Principle: T-Vector Reachability Analysis
 *
 * An entity belongs to the Initial Chunk if and only if:
 * 1. It is visible at T=0 (initial render time), AND
 * 2. It is reachable from the viewport bounds at T=0
 *
 * An entity belongs to a Lazy Chunk if:
 * 1. It becomes visible only after T changes (user interaction), OR
 * 2. It is outside the initial viewport (below the fold), OR
 * 3. It depends on Q-dimension data that hasn't loaded yet
 *
 * ## Algorithm Overview
 *
 * ```
 * INPUT: Constraint Graph G = (Entities, Constraints)
 *        Viewport V = (width, height)
 *        Initial Time T₀ = 0
 *
 * OUTPUT: Partition P = { Chunk₀ (initial), Chunk₁, Chunk₂, ... }
 *
 * ALGORITHM:
 * 1. Evaluate all constraints at T=T₀ to get initial positions
 * 2. Mark entities intersecting V as "initially visible"
 * 3. For each Q-dimension binding (event handler):
 *    a. Trace which constraints are affected when event fires
 *    b. Group affected entities into event-specific lazy chunks
 * 4. For entities below the fold:
 *    a. Create viewport-intersection lazy chunks
 * 5. Compute chunk dependency DAG
 * ```
 */

import type {
  EntityId,
  ConstraintId,
  ChunkId,
  Chunk,
  LoadTrigger,
  RenderableEntity,
  Rational,
  PVectorBounds,
} from '../ast/types';

// =============================================================================
// Input Types (from IR)
// =============================================================================

interface IRConstraint {
  id: ConstraintId;
  target: EntityId;
  component: 'x' | 'y' | 'z' | 't';
  relation: 'eq' | 'lt' | 'le' | 'gt' | 'ge';
  term: IRConstraintTerm;
}

type IRConstraintTerm =
  | { type: 'const'; value: Rational }
  | { type: 'ref'; entityId: EntityId; component: 'x' | 'y' | 'z' | 't' }
  | { type: 'linear'; coefficient: Rational; entityId: EntityId; component: 'x' | 'y' | 'z' | 't'; offset: Rational };

interface IRModule {
  entities: EntityId[];
  constraints: IRConstraint[];
  eventBindings: EventBinding[];
  imports: IRImport[];
}

interface EventBinding {
  sourceEntity: EntityId;
  eventType: string;
  targetConstraint: ConstraintId;
}

interface IRImport {
  path: string;
  exportedEntities: EntityId[];
}

interface ViewportBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

// =============================================================================
// Chunk Splitting Algorithm
// =============================================================================

/**
 * Static analysis result for chunk splitting.
 */
export interface ChunkSplitResult {
  /** The initial chunk (loaded immediately) */
  initialChunk: Chunk;

  /** Lazy chunks (loaded on demand) */
  lazyChunks: Chunk[];

  /** Mapping from entity to chunk */
  entityToChunk: Map<EntityId, ChunkId>;

  /** Chunk dependency DAG */
  chunkDependencies: Map<ChunkId, Set<ChunkId>>;
}

/**
 * Rule 1: Initial Visibility Rule
 *
 * An entity E is initially visible iff:
 *   ∃ constraint C where C.target = E ∧ C.component ∈ {x, y} ∧
 *   eval(C, T=0) produces a position within viewport bounds
 *
 * Pseudocode:
 * ```
 * function isInitiallyVisible(entity, constraints, viewport):
 *   position = evaluatePosition(entity, constraints, T=0)
 *   return intersects(position.bounds, viewport)
 * ```
 */
function computeInitiallyVisibleEntities(
  ir: IRModule,
  viewport: ViewportBounds,
): Set<EntityId> {
  const visible = new Set<EntityId>();

  // Step 1: Evaluate all constraints at T=0
  const positions = evaluateConstraintsAtT0(ir.constraints);

  // Step 2: Check intersection with viewport
  for (const entityId of ir.entities) {
    const pos = positions.get(entityId);
    if (pos && intersectsViewport(pos, viewport)) {
      visible.add(entityId);
    }
  }

  // Step 3: Include entities referenced by visible entities (transitive closure)
  let changed = true;
  while (changed) {
    changed = false;
    for (const entityId of visible) {
      const refs = getReferencedEntities(entityId, ir.constraints);
      for (const ref of refs) {
        if (!visible.has(ref)) {
          visible.add(ref);
          changed = true;
        }
      }
    }
  }

  return visible;
}

/**
 * Rule 2: Event-Triggered Lazy Chunk Rule
 *
 * When an event E triggers constraint C, all entities affected by C
 * (directly or transitively) form a lazy chunk.
 *
 * Affected entities are computed via constraint dependency analysis:
 * ```
 * function getAffectedEntities(constraint):
 *   affected = {constraint.target}
 *   for each constraint C' that references constraint.target:
 *     affected = affected ∪ getAffectedEntities(C')
 *   return affected
 * ```
 *
 * Q-dimension involvement:
 * - Events ARE Q-dimension (user input is unpredictable)
 * - Event handlers define the boundary between Initial and Lazy
 * - Each unique event source creates a potential chunk boundary
 */
function computeEventTriggeredChunks(
  ir: IRModule,
  initiallyVisible: Set<EntityId>,
): Map<string, Set<EntityId>> {
  const eventChunks = new Map<string, Set<EntityId>>();

  for (const binding of ir.eventBindings) {
    const chunkKey = `event:${binding.sourceEntity}:${binding.eventType}`;

    // Find the constraint being modified
    const targetConstraint = ir.constraints.find(c => c.id === binding.targetConstraint);
    if (!targetConstraint) continue;

    // Compute transitively affected entities
    const affected = computeAffectedEntities(targetConstraint.target, ir.constraints);

    // Filter to entities NOT in initial chunk
    const lazyEntities = new Set<EntityId>();
    for (const entityId of affected) {
      if (!initiallyVisible.has(entityId)) {
        lazyEntities.add(entityId);
      }
    }

    if (lazyEntities.size > 0) {
      eventChunks.set(chunkKey, lazyEntities);
    }
  }

  return eventChunks;
}

/**
 * Rule 3: Below-the-Fold Lazy Chunk Rule
 *
 * Entities outside the initial viewport form viewport-intersection chunks.
 * These are loaded when the user scrolls them into view.
 *
 * Chunking strategy:
 * - Divide the below-fold area into horizontal bands
 * - Each band forms a separate chunk
 * - Chunk loading is triggered by Intersection Observer
 */
function computeViewportIntersectionChunks(
  ir: IRModule,
  viewport: ViewportBounds,
  initiallyVisible: Set<EntityId>,
): Map<string, Set<EntityId>> {
  const viewportChunks = new Map<string, Set<EntityId>>();

  // Evaluate positions
  const positions = evaluateConstraintsAtT0(ir.constraints);

  // Band height (configurable, default to viewport height)
  const bandHeight = viewport.height;

  for (const entityId of ir.entities) {
    if (initiallyVisible.has(entityId)) continue;

    const pos = positions.get(entityId);
    if (!pos) continue;

    // Determine which band this entity falls into
    const bandIndex = Math.floor(pos.y / bandHeight);
    const chunkKey = `viewport-band:${bandIndex}`;

    if (!viewportChunks.has(chunkKey)) {
      viewportChunks.set(chunkKey, new Set());
    }
    viewportChunks.get(chunkKey)!.add(entityId);
  }

  return viewportChunks;
}

/**
 * Rule 4: Import-Based Chunk Boundary Rule
 *
 * Each imported module can define a chunk boundary.
 * This enables code splitting at the component level.
 *
 * ```
 * import { Button } from "./components/button.vs"
 * // Button's entities MAY be in a separate chunk if:
 * // - They are not initially visible, OR
 * // - They are marked with `lazy: true` in the import
 * ```
 */
function computeImportChunks(
  ir: IRModule,
  initiallyVisible: Set<EntityId>,
): Map<string, Set<EntityId>> {
  const importChunks = new Map<string, Set<EntityId>>();

  for (const imp of ir.imports) {
    const lazyEntities = new Set<EntityId>();

    for (const entityId of imp.exportedEntities) {
      if (!initiallyVisible.has(entityId)) {
        lazyEntities.add(entityId);
      }
    }

    if (lazyEntities.size > 0) {
      const chunkKey = `import:${imp.path}`;
      importChunks.set(chunkKey, lazyEntities);
    }
  }

  return importChunks;
}

/**
 * Main chunk splitting function.
 */
export function splitIntoChunks(
  ir: IRModule,
  viewport: ViewportBounds,
): ChunkSplitResult {
  // Step 1: Compute initially visible entities
  const initiallyVisible = computeInitiallyVisibleEntities(ir, viewport);

  // Step 2: Compute event-triggered chunks
  const eventChunks = computeEventTriggeredChunks(ir, initiallyVisible);

  // Step 3: Compute viewport-intersection chunks
  const viewportChunks = computeViewportIntersectionChunks(ir, viewport, initiallyVisible);

  // Step 4: Compute import-based chunks
  const importChunks = computeImportChunks(ir, initiallyVisible);

  // Step 5: Merge and deduplicate chunks
  const allLazyChunks = new Map<string, Set<EntityId>>();

  for (const [key, entities] of eventChunks) {
    allLazyChunks.set(key, entities);
  }
  for (const [key, entities] of viewportChunks) {
    if (!allLazyChunks.has(key)) {
      allLazyChunks.set(key, entities);
    } else {
      // Merge
      for (const e of entities) {
        allLazyChunks.get(key)!.add(e);
      }
    }
  }
  for (const [key, entities] of importChunks) {
    if (!allLazyChunks.has(key)) {
      allLazyChunks.set(key, entities);
    }
  }

  // Step 6: Build chunk objects
  const initialChunk: Chunk = {
    id: 'initial',
    entityIds: Array.from(initiallyVisible),
    dependsOn: [],
    isInitial: true,
    loadTriggers: [{ type: 'immediate' }],
  };

  const lazyChunks: Chunk[] = [];
  const entityToChunk = new Map<EntityId, ChunkId>();

  for (const entityId of initiallyVisible) {
    entityToChunk.set(entityId, 'initial');
  }

  for (const [chunkId, entities] of allLazyChunks) {
    const triggers = computeLoadTriggers(chunkId, entities, ir);

    const chunk: Chunk = {
      id: chunkId,
      entityIds: Array.from(entities),
      dependsOn: ['initial'], // All lazy chunks depend on initial
      isInitial: false,
      loadTriggers: triggers,
    };

    lazyChunks.push(chunk);

    for (const entityId of entities) {
      entityToChunk.set(entityId, chunkId);
    }
  }

  // Step 7: Compute chunk dependencies
  const chunkDependencies = computeChunkDependencies(
    initialChunk,
    lazyChunks,
    ir.constraints,
    entityToChunk,
  );

  return {
    initialChunk,
    lazyChunks,
    entityToChunk,
    chunkDependencies,
  };
}

// =============================================================================
// Helper Functions
// =============================================================================

function evaluateConstraintsAtT0(
  constraints: IRConstraint[],
): Map<EntityId, { x: number; y: number; z: number }> {
  // Simplified evaluation - in production, this would use
  // the full constraint solver with T=0
  const positions = new Map<EntityId, { x: number; y: number; z: number }>();

  for (const c of constraints) {
    if (!positions.has(c.target)) {
      positions.set(c.target, { x: 0, y: 0, z: 0 });
    }

    const pos = positions.get(c.target)!;

    if (c.term.type === 'const') {
      const value = rationalToNumber(c.term.value);
      if (c.component === 'x') pos.x = value;
      if (c.component === 'y') pos.y = value;
      if (c.component === 'z') pos.z = value;
    }
  }

  return positions;
}

function rationalToNumber(r: Rational): number {
  return Number(r.numerator) / Number(r.denominator);
}

function intersectsViewport(
  pos: { x: number; y: number },
  viewport: ViewportBounds,
): boolean {
  // Simplified - assumes point intersection
  return (
    pos.x >= viewport.x &&
    pos.x <= viewport.x + viewport.width &&
    pos.y >= viewport.y &&
    pos.y <= viewport.y + viewport.height
  );
}

function getReferencedEntities(
  entityId: EntityId,
  constraints: IRConstraint[],
): Set<EntityId> {
  const refs = new Set<EntityId>();

  for (const c of constraints) {
    if (c.target !== entityId) continue;

    if (c.term.type === 'ref') {
      refs.add(c.term.entityId);
    } else if (c.term.type === 'linear') {
      refs.add(c.term.entityId);
    }
  }

  return refs;
}

function computeAffectedEntities(
  startEntity: EntityId,
  constraints: IRConstraint[],
): Set<EntityId> {
  const affected = new Set<EntityId>();
  const queue = [startEntity];

  while (queue.length > 0) {
    const current = queue.shift()!;
    if (affected.has(current)) continue;
    affected.add(current);

    // Find constraints that reference this entity
    for (const c of constraints) {
      const refEntity =
        c.term.type === 'ref' ? c.term.entityId :
        c.term.type === 'linear' ? c.term.entityId :
        null;

      if (refEntity === current && !affected.has(c.target)) {
        queue.push(c.target);
      }
    }
  }

  return affected;
}

function computeLoadTriggers(
  chunkId: string,
  entities: Set<EntityId>,
  ir: IRModule,
): LoadTrigger[] {
  const triggers: LoadTrigger[] = [];

  if (chunkId.startsWith('event:')) {
    const [, entityIdStr, eventType] = chunkId.split(':');
    triggers.push({
      type: 'event',
      eventType,
      targetEntity: parseInt(entityIdStr, 10),
    });
  } else if (chunkId.startsWith('viewport-band:')) {
    // Use first entity as intersection target
    const firstEntity = entities.values().next().value;
    if (firstEntity !== undefined) {
      triggers.push({
        type: 'viewport-intersect',
        entityId: firstEntity,
      });
    }
  }

  return triggers;
}

function computeChunkDependencies(
  initial: Chunk,
  lazy: Chunk[],
  constraints: IRConstraint[],
  entityToChunk: Map<EntityId, ChunkId>,
): Map<ChunkId, Set<ChunkId>> {
  const deps = new Map<ChunkId, Set<ChunkId>>();

  deps.set(initial.id, new Set());

  for (const chunk of lazy) {
    const chunkDeps = new Set<ChunkId>();

    // Check if any entity in this chunk references an entity in another chunk
    for (const entityId of chunk.entityIds) {
      const refs = getReferencedEntities(entityId, constraints);
      for (const ref of refs) {
        const refChunk = entityToChunk.get(ref);
        if (refChunk && refChunk !== chunk.id) {
          chunkDeps.add(refChunk);
        }
      }
    }

    deps.set(chunk.id, chunkDeps);
  }

  return deps;
}
