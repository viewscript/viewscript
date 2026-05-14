//! Rigidity analysis using Laman's Pebble Game algorithm.
//!
//! This module implements the Pebble Game algorithm for determining
//! whether a 2D constraint graph is rigid, flexible, or overconstrained.
//!
//! # Laman's Theorem
//!
//! A graph G = (V, E) is minimally rigid (isostatic) in 2D if and only if:
//! 1. |E| = 2|V| - 3
//! 2. For every subgraph H = (V', E') with |V'| >= 2: |E'| <= 2|V'| - 3
//!
//! # Pebble Game Algorithm
//!
//! Each vertex has 2 pebbles (degrees of freedom in 2D).
//! Adding an edge requires collecting 3 pebbles from its endpoints.
//! If 3 pebbles cannot be collected, the edge is redundant.

use std::collections::{HashMap, HashSet, VecDeque};

/// Unique identifier for a vertex in the constraint graph.
pub type VertexId = u64;

/// Unique identifier for an edge (constraint) in the graph.
pub type EdgeId = u64;

/// An edge in the constraint graph (renamed from Edge to avoid collision with types::Edge).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConstraintEdge {
    pub id: EdgeId,
    pub v1: VertexId,
    pub v2: VertexId,
}

/// The rigidity status of a constraint graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RigidityStatus {
    /// The graph is minimally rigid (isostatic).
    /// |E| = 2|V| - 3 and all edges are independent.
    Rigid,

    /// The graph is under-constrained (flexible).
    /// |E| < 2|V| - 3, has remaining degrees of freedom.
    Flexible {
        /// Number of remaining degrees of freedom.
        degrees_of_freedom: usize,
    },

    /// The graph is over-constrained.
    /// Contains redundant constraints that may cause conflicts.
    Overconstrained {
        /// IDs of redundant edges that could be removed.
        redundant_edges: Vec<EdgeId>,
    },
}

/// Result of rigidity analysis.
#[derive(Clone, Debug)]
pub struct RigidityAnalysis {
    /// The overall rigidity status.
    pub status: RigidityStatus,

    /// Number of vertices in the graph.
    pub vertex_count: usize,

    /// Number of edges in the graph.
    pub edge_count: usize,

    /// Number of independent edges (non-redundant).
    pub independent_edge_count: usize,

    /// The Laman number: 2|V| - 3 for 2D rigidity.
    pub laman_number: i64,
}

/// The Pebble Game state for rigidity analysis.
struct PebbleGame {
    /// Number of pebbles at each vertex (0, 1, or 2).
    pebbles: HashMap<VertexId, u8>,

    /// Directed edges in the pebble graph.
    /// For each vertex, stores the set of vertices it points to.
    directed_edges: HashMap<VertexId, HashSet<VertexId>>,

    /// Original edge information for tracking redundancy.
    edge_info: HashMap<(VertexId, VertexId), EdgeId>,

    /// All vertices in the graph.
    vertices: HashSet<VertexId>,
}

impl PebbleGame {
    /// Create a new Pebble Game with the given vertices.
    fn new(vertices: impl IntoIterator<Item = VertexId>) -> Self {
        let vertices: HashSet<_> = vertices.into_iter().collect();
        let pebbles: HashMap<_, _> = vertices.iter().map(|&v| (v, 2u8)).collect();
        let directed_edges: HashMap<_, _> = vertices.iter().map(|&v| (v, HashSet::new())).collect();

        Self {
            pebbles,
            directed_edges,
            edge_info: HashMap::new(),
            vertices,
        }
    }

    /// Get the number of pebbles at a vertex.
    fn pebble_count(&self, v: VertexId) -> u8 {
        *self.pebbles.get(&v).unwrap_or(&0)
    }

    /// Find a vertex with free pebbles reachable from `target` and redirect
    /// edges to move one pebble to `target`.
    ///
    /// In the pebble game, edge u->v means u used a pebble to cover edge (u,v).
    /// To free a pebble at u, we find a vertex w reachable via u->...->w that
    /// has a free pebble, then reverse edges along the path so w takes ownership.
    ///
    /// Returns true if a pebble was successfully moved to target.
    fn find_and_redirect_pebble(&mut self, target: VertexId, exclude: Option<VertexId>) -> bool {
        // BFS following OUTGOING edges to find a vertex with free pebbles
        let mut visited = HashSet::new();
        let mut parent: HashMap<VertexId, VertexId> = HashMap::new();
        let mut queue = VecDeque::new();

        visited.insert(target);
        if let Some(ex) = exclude {
            visited.insert(ex);
        }
        queue.push_back(target);

        let mut found_source: Option<VertexId> = None;

        while let Some(current) = queue.pop_front() {
            // Follow outgoing edges from current
            if let Some(neighbors) = self.directed_edges.get(&current) {
                for &next in neighbors.iter() {
                    if !visited.contains(&next) {
                        visited.insert(next);
                        parent.insert(next, current);

                        // Check if this vertex has free pebbles
                        if self.pebble_count(next) > 0 {
                            found_source = Some(next);
                            break;
                        }
                        queue.push_back(next);
                    }
                }
            }

            if found_source.is_some() {
                break;
            }
        }

        // If we found a source with pebbles, reverse edges along the path
        if let Some(source) = found_source {
            // Source gives up a pebble
            self.pebbles.entry(source).and_modify(|p| *p -= 1);

            // Reverse edges from source back to target
            // Path: target -> ... -> prev -> source
            // We reverse each edge: prev->source becomes source->prev
            let mut current = source;
            while let Some(&prev) = parent.get(&current) {
                // Edge was prev -> current, reverse to current -> prev
                self.directed_edges
                    .get_mut(&prev)
                    .map(|e| e.remove(&current));
                self.directed_edges
                    .get_mut(&current)
                    .map(|e| e.insert(prev));
                current = prev;
            }

            // Target gains a pebble
            self.pebbles.entry(target).and_modify(|p| *p += 1);

            return true;
        }

        false
    }

    /// Try to add an edge to the pebble graph.
    ///
    /// Returns true if the edge is independent (non-redundant),
    /// false if it is redundant.
    fn add_edge(&mut self, edge: &ConstraintEdge) -> bool {
        let v1 = edge.v1;
        let v2 = edge.v2;

        // Ensure both vertices exist
        if !self.vertices.contains(&v1) {
            self.vertices.insert(v1);
            self.pebbles.insert(v1, 2);
            self.directed_edges.insert(v1, HashSet::new());
        }
        if !self.vertices.contains(&v2) {
            self.vertices.insert(v2);
            self.pebbles.insert(v2, 2);
            self.directed_edges.insert(v2, HashSet::new());
        }

        // Store edge info
        let key = if v1 < v2 { (v1, v2) } else { (v2, v1) };
        self.edge_info.insert(key, edge.id);

        // Try to collect 3 pebbles total from v1 and v2
        // Must search to ensure pebbles are actually reachable
        let can_collect = self.try_collect_three_pebbles(v1, v2);

        if can_collect {
            // Use one pebble to cover this edge
            if self.pebble_count(v1) >= 1 {
                self.pebbles.entry(v1).and_modify(|p| *p -= 1);
                self.directed_edges.get_mut(&v1).map(|e| e.insert(v2));
            } else {
                self.pebbles.entry(v2).and_modify(|p| *p -= 1);
                self.directed_edges.get_mut(&v2).map(|e| e.insert(v1));
            }
            true
        } else {
            // Edge is redundant
            false
        }
    }

    /// Try to collect 3 pebbles for edge (v1, v2).
    ///
    /// For (2,3)-sparsity (2D rigidity), we gather pebbles at both endpoints
    /// (each excluding the other) and check if the total is >= 3.
    fn try_collect_three_pebbles(&mut self, v1: VertexId, v2: VertexId) -> bool {
        // Gather at v1 (excluding v2)
        while self.pebble_count(v1) < 2 {
            if !self.find_and_redirect_pebble(v1, Some(v2)) {
                break;
            }
        }
        let at_v1 = self.pebble_count(v1);

        // Gather at v2 (excluding v1)
        while self.pebble_count(v2) < 2 {
            if !self.find_and_redirect_pebble(v2, Some(v1)) {
                break;
            }
        }
        let at_v2 = self.pebble_count(v2);

        at_v1 + at_v2 >= 3
    }
}

/// Analyze the rigidity of a constraint graph.
///
/// # Arguments
///
/// * `vertices` - Iterator of vertex IDs in the graph
/// * `edges` - Iterator of edges (constraints) in the graph
///
/// # Returns
///
/// A `RigidityAnalysis` containing the rigidity status and statistics.
///
/// # Algorithm
///
/// Uses the Pebble Game algorithm to determine:
/// - Which edges are independent (non-redundant)
/// - Which edges are redundant (overconstrained)
/// - How many degrees of freedom remain (if flexible)
pub fn analyze_rigidity(
    vertices: impl IntoIterator<Item = VertexId>,
    edges: impl IntoIterator<Item = ConstraintEdge>,
) -> RigidityAnalysis {
    let vertices: Vec<_> = vertices.into_iter().collect();
    let edges: Vec<_> = edges.into_iter().collect();

    let vertex_count = vertices.len();
    let edge_count = edges.len();

    // Handle degenerate cases
    if vertex_count == 0 {
        return RigidityAnalysis {
            status: RigidityStatus::Rigid,
            vertex_count: 0,
            edge_count: 0,
            independent_edge_count: 0,
            laman_number: -3,
        };
    }

    if vertex_count == 1 {
        return RigidityAnalysis {
            status: RigidityStatus::Flexible {
                degrees_of_freedom: 2,
            },
            vertex_count: 1,
            edge_count: 0,
            independent_edge_count: 0,
            laman_number: -1,
        };
    }

    // Laman number for 2D rigidity: max independent edges = 2|V| - 3
    let laman_number = 2 * vertex_count as i64 - 3;
    let max_independent = laman_number.max(0) as usize;

    let mut game = PebbleGame::new(vertices);
    let mut redundant_edges = Vec::new();
    let mut independent_count = 0;

    for edge in &edges {
        // Use both pebble game AND Laman bound to determine independence
        // An edge is independent only if:
        // 1. Pebble game says it can be added, AND
        // 2. We haven't exceeded the Laman bound
        let pebble_ok = game.add_edge(edge);
        let within_laman = independent_count < max_independent;

        if pebble_ok && within_laman {
            independent_count += 1;
        } else {
            redundant_edges.push(edge.id);
        }
    }

    // Remaining degrees of freedom
    // DoF = max_independent - independent_count (edges needed to become rigid)
    let dof = max_independent.saturating_sub(independent_count);

    let status = if !redundant_edges.is_empty() {
        RigidityStatus::Overconstrained { redundant_edges }
    } else if dof > 0 {
        RigidityStatus::Flexible {
            degrees_of_freedom: dof,
        }
    } else {
        RigidityStatus::Rigid
    };

    RigidityAnalysis {
        status,
        vertex_count,
        edge_count,
        independent_edge_count: independent_count,
        laman_number,
    }
}

/// Convert constraint system entities to a rigidity graph.
///
/// Maps ViewScript entities to vertices and constraints to edges.
pub struct ConstraintGraphBuilder {
    vertices: HashSet<VertexId>,
    edges: Vec<ConstraintEdge>,
    next_edge_id: EdgeId,
}

impl ConstraintGraphBuilder {
    /// Create a new constraint graph builder.
    pub fn new() -> Self {
        Self {
            vertices: HashSet::new(),
            edges: Vec::new(),
            next_edge_id: 1,
        }
    }

    /// Add a vertex (entity) to the graph.
    pub fn add_vertex(&mut self, id: VertexId) {
        self.vertices.insert(id);
    }

    /// Add an edge (constraint) between two vertices.
    ///
    /// Returns the edge ID.
    pub fn add_edge(&mut self, v1: VertexId, v2: VertexId) -> EdgeId {
        let id = self.next_edge_id;
        self.next_edge_id += 1;

        self.vertices.insert(v1);
        self.vertices.insert(v2);
        self.edges.push(ConstraintEdge { id, v1, v2 });

        id
    }

    /// Add an edge with a specific ID.
    pub fn add_edge_with_id(&mut self, id: EdgeId, v1: VertexId, v2: VertexId) {
        self.vertices.insert(v1);
        self.vertices.insert(v2);
        self.edges.push(ConstraintEdge { id, v1, v2 });
    }

    /// Build and analyze the constraint graph.
    pub fn analyze(self) -> RigidityAnalysis {
        analyze_rigidity(self.vertices, self.edges)
    }
}

impl Default for ConstraintGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_edge() {
        // Two vertices, one edge
        // |E| = 1, |V| = 2, Laman = 2*2 - 3 = 1
        // Should be minimally rigid
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 2);
        assert_eq!(result.edge_count, 1);
        assert_eq!(result.laman_number, 1);
        assert_eq!(result.status, RigidityStatus::Rigid);
    }

    #[test]
    fn test_triangle_rigid() {
        // Triangle: 3 vertices, 3 edges
        // |E| = 3, |V| = 3, Laman = 2*3 - 3 = 3
        // Exactly minimal, should be rigid
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);
        builder.add_edge(2, 3);
        builder.add_edge(3, 1);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 3);
        assert_eq!(result.edge_count, 3);
        assert_eq!(result.laman_number, 3);
        assert_eq!(result.independent_edge_count, 3);
        assert_eq!(result.status, RigidityStatus::Rigid);
    }

    #[test]
    fn test_square_flexible() {
        // Square without diagonal: 4 vertices, 4 edges
        // |E| = 4, |V| = 4, Laman = 2*4 - 3 = 5
        // Under-constrained by 1 DoF (can shear)
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);
        builder.add_edge(2, 3);
        builder.add_edge(3, 4);
        builder.add_edge(4, 1);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.edge_count, 4);
        assert_eq!(result.laman_number, 5);
        assert!(matches!(
            result.status,
            RigidityStatus::Flexible {
                degrees_of_freedom: 1
            }
        ));
    }

    #[test]
    fn test_square_with_diagonal_rigid() {
        // Square with one diagonal: 4 vertices, 5 edges
        // |E| = 5, |V| = 4, Laman = 5
        // Minimally rigid
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);
        builder.add_edge(2, 3);
        builder.add_edge(3, 4);
        builder.add_edge(4, 1);
        builder.add_edge(1, 3); // diagonal

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.edge_count, 5);
        assert_eq!(result.independent_edge_count, 5);
        assert_eq!(result.status, RigidityStatus::Rigid);
    }

    #[test]
    fn test_square_with_both_diagonals_overconstrained() {
        // Square with both diagonals: 4 vertices, 6 edges
        // |E| = 6, |V| = 4, Laman = 5
        // Overconstrained by 1 edge
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge_with_id(1, 1, 2);
        builder.add_edge_with_id(2, 2, 3);
        builder.add_edge_with_id(3, 3, 4);
        builder.add_edge_with_id(4, 4, 1);
        builder.add_edge_with_id(5, 1, 3); // diagonal 1
        builder.add_edge_with_id(6, 2, 4); // diagonal 2

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.edge_count, 6);
        assert_eq!(result.independent_edge_count, 5);

        match &result.status {
            RigidityStatus::Overconstrained { redundant_edges } => {
                assert_eq!(redundant_edges.len(), 1);
            }
            _ => panic!("Expected Overconstrained status"),
        }
    }

    #[test]
    fn test_isolated_vertex() {
        // Single vertex with no edges
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_vertex(1);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 1);
        assert_eq!(result.edge_count, 0);
        assert!(matches!(
            result.status,
            RigidityStatus::Flexible {
                degrees_of_freedom: 2
            }
        ));
    }

    #[test]
    fn test_pentagon_flexible() {
        // Pentagon: 5 vertices, 5 edges
        // |E| = 5, |V| = 5, Laman = 2*5 - 3 = 7
        // Under-constrained by 2 DoF
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);
        builder.add_edge(2, 3);
        builder.add_edge(3, 4);
        builder.add_edge(4, 5);
        builder.add_edge(5, 1);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 5);
        assert_eq!(result.edge_count, 5);
        assert_eq!(result.laman_number, 7);
        assert!(matches!(
            result.status,
            RigidityStatus::Flexible {
                degrees_of_freedom: 2
            }
        ));
    }

    #[test]
    fn test_two_triangles_sharing_edge() {
        // Two triangles sharing an edge: 4 vertices, 5 edges
        // This forms a "butterfly" shape
        // |E| = 5, |V| = 4, Laman = 5
        // Should be rigid
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);
        builder.add_edge(2, 3);
        builder.add_edge(3, 1);
        builder.add_edge(2, 4);
        builder.add_edge(4, 3);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.edge_count, 5);
        assert_eq!(result.independent_edge_count, 5);
        assert_eq!(result.status, RigidityStatus::Rigid);
    }

    #[test]
    fn test_k4_complete_overconstrained() {
        // Complete graph K4: 4 vertices, 6 edges
        // Every pair connected
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge_with_id(1, 1, 2);
        builder.add_edge_with_id(2, 1, 3);
        builder.add_edge_with_id(3, 1, 4);
        builder.add_edge_with_id(4, 2, 3);
        builder.add_edge_with_id(5, 2, 4);
        builder.add_edge_with_id(6, 3, 4);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.edge_count, 6);
        // Laman number = 2*4 - 3 = 5, so max 5 independent edges
        assert_eq!(result.independent_edge_count, 5);

        match &result.status {
            RigidityStatus::Overconstrained { redundant_edges } => {
                assert_eq!(redundant_edges.len(), 1);
            }
            _ => panic!("Expected Overconstrained status"),
        }
    }

    #[test]
    fn test_chain_flexible() {
        // Chain of 4 vertices: v1--v2--v3--v4
        // 4 vertices, 3 edges
        // |E| = 3, Laman = 5
        // Under-constrained by 2 DoF
        let mut builder = ConstraintGraphBuilder::new();
        builder.add_edge(1, 2);
        builder.add_edge(2, 3);
        builder.add_edge(3, 4);

        let result = builder.analyze();
        assert_eq!(result.vertex_count, 4);
        assert_eq!(result.edge_count, 3);
        assert_eq!(result.laman_number, 5);
        assert!(matches!(
            result.status,
            RigidityStatus::Flexible {
                degrees_of_freedom: 2
            }
        ));
    }
}
