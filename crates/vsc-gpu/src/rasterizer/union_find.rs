//! Union-Find Data Structure for Coordinate Equivalence Classes
//!
//! This module implements a Union-Find (disjoint set) data structure with
//! path compression for efficiently managing coordinate equivalence classes
//! in the topology-preserving rounding algorithm.
//!
//! ## Design Decision: Tuple Keys vs String Keys
//!
//! Unlike the TypeScript implementation which uses string keys like "entity_id:edge",
//! we use `(EntityId, Edge)` tuples directly as HashMap keys. This provides:
//! - Better performance (no string allocation/hashing)
//! - Type safety (compile-time edge validation)
//! - Clearer semantics

use std::collections::HashMap;
use vsc_core::{CoordRef, Edge};

/// Axis for coordinate grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Extension trait for Edge to get axis information.
pub trait EdgeAxis {
    /// Returns the axis this edge belongs to.
    fn axis(&self) -> Axis;
}

impl EdgeAxis for Edge {
    fn axis(&self) -> Axis {
        match self {
            Edge::Left | Edge::Right => Axis::Horizontal,
            Edge::Top | Edge::Bottom => Axis::Vertical,
        }
    }
}

/// Union-Find data structure with path compression.
///
/// Used to build equivalence classes of coordinates that must round to the
/// same pixel value. Two coordinates are in the same class if:
/// 1. They have the same rational value, OR
/// 2. They are connected by an 'equal' or 'adjacent' constraint
///
/// ## Example
///
/// ```
/// use vsc_gpu::rasterizer::union_find::UnionFind;
/// use vsc_core::{CoordRef, Edge, EntityId};
///
/// let mut uf = UnionFind::new();
///
/// let a_right: CoordRef = (EntityId(1), Edge::Right);
/// let b_left: CoordRef = (EntityId(2), Edge::Left);
///
/// // Initially, each coordinate is its own class
/// assert!(!uf.same_set(a_right, b_left));
///
/// // Union them (e.g., because they're adjacent)
/// uf.union(a_right, b_left);
///
/// // Now they're in the same equivalence class
/// assert!(uf.same_set(a_right, b_left));
/// ```
#[derive(Debug, Clone)]
pub struct UnionFind {
    parent: HashMap<CoordRef, CoordRef>,
    rank: HashMap<CoordRef, usize>,
}

impl UnionFind {
    /// Create a new empty Union-Find structure.
    pub fn new() -> Self {
        Self {
            parent: HashMap::new(),
            rank: HashMap::new(),
        }
    }

    /// Create a Union-Find with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            parent: HashMap::with_capacity(capacity),
            rank: HashMap::with_capacity(capacity),
        }
    }

    /// Find the representative (root) of the set containing `key`.
    ///
    /// Uses path compression: all nodes along the path to the root
    /// are updated to point directly to the root.
    pub fn find(&mut self, key: CoordRef) -> CoordRef {
        // Ensure the key exists
        if !self.parent.contains_key(&key) {
            self.parent.insert(key, key);
            self.rank.insert(key, 0);
            return key;
        }

        let parent = self.parent[&key];
        if parent != key {
            // Path compression: update to point directly to root
            let root = self.find(parent);
            self.parent.insert(key, root);
            root
        } else {
            key
        }
    }

    /// Union two sets containing `a` and `b`.
    ///
    /// Uses union by rank to keep trees balanced.
    pub fn union(&mut self, a: CoordRef, b: CoordRef) {
        let root_a = self.find(a);
        let root_b = self.find(b);

        if root_a == root_b {
            return; // Already in the same set
        }

        // Union by rank: attach smaller tree under larger tree
        let rank_a = *self.rank.get(&root_a).unwrap_or(&0);
        let rank_b = *self.rank.get(&root_b).unwrap_or(&0);

        if rank_a < rank_b {
            self.parent.insert(root_a, root_b);
        } else if rank_a > rank_b {
            self.parent.insert(root_b, root_a);
        } else {
            // Same rank: choose one as root and increment its rank
            self.parent.insert(root_b, root_a);
            self.rank.insert(root_a, rank_a + 1);
        }
    }

    /// Check if two keys are in the same set.
    pub fn same_set(&mut self, a: CoordRef, b: CoordRef) -> bool {
        self.find(a) == self.find(b)
    }

    /// Get the number of unique elements tracked.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    /// Check if the structure is empty.
    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// Collect all equivalence classes.
    ///
    /// Returns a map from root representative to all members of that class.
    pub fn collect_classes(&mut self) -> HashMap<CoordRef, Vec<CoordRef>> {
        let keys: Vec<CoordRef> = self.parent.keys().copied().collect();
        let mut classes: HashMap<CoordRef, Vec<CoordRef>> = HashMap::new();

        for key in keys {
            let root = self.find(key);
            classes.entry(root).or_default().push(key);
        }

        classes
    }
}

impl Default for UnionFind {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vsc_core::EntityId;

    #[test]
    fn test_find_creates_singleton() {
        let mut uf = UnionFind::new();
        let coord = (EntityId(1), Edge::Left);

        let root = uf.find(coord);
        assert_eq!(root, coord);
        assert_eq!(uf.len(), 1);
    }

    #[test]
    fn test_union_basic() {
        let mut uf = UnionFind::new();
        let a = (EntityId(1), Edge::Right);
        let b = (EntityId(2), Edge::Left);

        assert!(!uf.same_set(a, b));

        uf.union(a, b);

        assert!(uf.same_set(a, b));
    }

    #[test]
    fn test_union_transitive() {
        let mut uf = UnionFind::new();
        let a = (EntityId(1), Edge::Right);
        let b = (EntityId(2), Edge::Left);
        let c = (EntityId(3), Edge::Left);

        uf.union(a, b);
        uf.union(b, c);

        // Transitivity: a and c should be in the same set
        assert!(uf.same_set(a, c));
    }

    #[test]
    fn test_path_compression() {
        let mut uf = UnionFind::new();
        let a = (EntityId(1), Edge::Left);
        let b = (EntityId(2), Edge::Left);
        let c = (EntityId(3), Edge::Left);
        let d = (EntityId(4), Edge::Left);

        // Create a chain: a <- b <- c <- d
        uf.union(a, b);
        uf.union(b, c);
        uf.union(c, d);

        // Finding d should compress the path
        let root = uf.find(d);

        // All should now point to the same root
        assert_eq!(uf.find(a), root);
        assert_eq!(uf.find(b), root);
        assert_eq!(uf.find(c), root);
    }

    #[test]
    fn test_collect_classes() {
        let mut uf = UnionFind::new();

        // Class 1: A.right, B.left
        let a_right = (EntityId(1), Edge::Right);
        let b_left = (EntityId(2), Edge::Left);
        uf.union(a_right, b_left);

        // Class 2: C.right, D.left, E.left
        let c_right = (EntityId(3), Edge::Right);
        let d_left = (EntityId(4), Edge::Left);
        let e_left = (EntityId(5), Edge::Left);
        uf.union(c_right, d_left);
        uf.union(d_left, e_left);

        let classes = uf.collect_classes();

        // Should have exactly 2 classes
        assert_eq!(classes.len(), 2);

        // Check class sizes
        let sizes: Vec<usize> = classes.values().map(|v| v.len()).collect();
        assert!(sizes.contains(&2));
        assert!(sizes.contains(&3));
    }

    #[test]
    fn test_edge_axis() {
        assert_eq!(Edge::Left.axis(), Axis::Horizontal);
        assert_eq!(Edge::Right.axis(), Axis::Horizontal);
        assert_eq!(Edge::Top.axis(), Axis::Vertical);
        assert_eq!(Edge::Bottom.axis(), Axis::Vertical);
    }
}
