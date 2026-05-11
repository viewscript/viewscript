//! CODL Abstract Syntax Tree (Phase 15)
//!
//! Defines the structure for Constraint Operation Description Language files.
//! CODL is a declarative YAML/JSON DSL for defining constraint generation macros.
//!
//! ## Example CODL File
//!
//! ```yaml
//! name: stack_vertical
//! version: "1.0.0"
//! description: "Stacks instances vertically with a specified gap"
//! parameters:
//!   - name: instances
//!     type: Array<EntityId>
//!   - name: gap
//!     type: Rational
//!     default: "0"
//! operations:
//!   - foreach:
//!       item: curr
//!       index: i
//!       in: instances
//!     where: "i > 0"
//!     yield:
//!       type: constraint
//!       target: "${curr}"
//!       component: y
//!       relation: eq
//!       term:
//!         type: linear
//!         entity_id: "${instances[i-1]}"
//!         component: y
//!         offset: "${gap}"
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root structure of a CODL command file (.vscmd.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodlCommand {
    /// Unique name of this command (e.g., "stack_vertical").
    pub name: String,

    /// SemVer version string.
    #[serde(default = "default_version")]
    pub version: String,

    /// Human-readable description.
    #[serde(default)]
    pub description: String,

    /// Input parameters for this command.
    #[serde(default)]
    pub parameters: Vec<CodlParameter>,

    /// Operations to execute (constraint generation rules).
    pub operations: Vec<CodlOperation>,

    /// Optional metadata (author, license, etc.).
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// A parameter definition for a CODL command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodlParameter {
    /// Parameter name (used in template expressions).
    pub name: String,

    /// Parameter type.
    #[serde(rename = "type")]
    pub param_type: CodlType,

    /// Optional default value (as expression string).
    #[serde(default)]
    pub default: Option<String>,

    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Supported types in CODL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodlType {
    /// Exact rational number.
    Rational,
    /// Entity identifier.
    EntityId,
    /// Array of entity identifiers.
    #[serde(rename = "Array<EntityId>")]
    ArrayEntityId,
    /// Array of rationals.
    #[serde(rename = "Array<Rational>")]
    ArrayRational,
    /// String value.
    String,
    /// Boolean value.
    Bool,
}

/// An operation in a CODL command.
///
/// Operations can be:
/// - Direct constraint yields
/// - Foreach loops with yields
/// - Conditional blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CodlOperation {
    /// A foreach loop that iterates over an array.
    Foreach(CodlForeach),
    /// A direct yield (single constraint or origin).
    DirectYield(CodlYield),
    /// Conditional operation.
    Conditional(CodlConditional),
}

/// A foreach loop operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodlForeach {
    /// Loop specification.
    pub foreach: ForeachSpec,

    /// Optional filter condition (e.g., "i > 0").
    #[serde(default)]
    pub r#where: Option<String>,

    /// What to yield for each iteration.
    pub r#yield: CodlYield,
}

/// Specification for a foreach loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeachSpec {
    /// Variable name for current item.
    pub item: String,

    /// Variable name for current index.
    pub index: String,

    /// Expression evaluating to the array to iterate.
    #[serde(rename = "in")]
    pub in_expr: String,
}

/// A conditional operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodlConditional {
    /// Condition expression (e.g., "origin_y != null").
    #[serde(rename = "if")]
    pub condition: String,

    /// Operations to execute if condition is true.
    pub then: Vec<CodlOperation>,

    /// Optional else branch.
    #[serde(default)]
    pub r#else: Option<Vec<CodlOperation>>,
}

/// A yield specification (what to generate).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodlYield {
    /// Generate a constraint.
    Constraint(CodlConstraintYield),
    /// Generate an origin constraint (shorthand).
    Origin(CodlOriginYield),
}

/// Yield specification for a constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodlConstraintYield {
    /// Target entity expression (e.g., "${curr}" or "${instances[0]}").
    pub target: String,

    /// Component to constrain.
    pub component: CodlComponent,

    /// Relation type.
    pub relation: CodlRelation,

    /// Constraint term.
    pub term: CodlTerm,

    /// Priority (default: soft for CODL-generated constraints).
    #[serde(default = "default_priority")]
    pub priority: CodlPriority,

    /// Optional intent description.
    #[serde(default)]
    pub intent: Option<String>,
}

fn default_priority() -> CodlPriority {
    CodlPriority::Soft
}

/// Yield specification for an origin constraint (shorthand).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodlOriginYield {
    /// Target entity expression.
    pub target: String,

    /// Component (x or y).
    pub component: CodlComponent,

    /// Value expression.
    pub value: String,

    /// Priority.
    #[serde(default = "default_priority")]
    pub priority: CodlPriority,
}

/// Vector component in CODL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodlComponent {
    X,
    Y,
    Z,
    T,
}

/// Relation type in CODL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodlRelation {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Constraint priority in CODL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodlPriority {
    Hard,
    Soft,
}

/// A constraint term in CODL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CodlTerm {
    /// Constant value.
    Const {
        /// Value expression (e.g., "${gap}" or "100").
        value: String,
    },
    /// Reference to another entity.
    Ref {
        /// Entity ID expression.
        entity_id: String,
        /// Component to reference.
        component: CodlComponent,
    },
    /// Linear transformation.
    Linear {
        /// Entity ID expression.
        entity_id: String,
        /// Component to reference.
        component: CodlComponent,
        /// Coefficient expression (default: "1").
        #[serde(default = "default_coefficient")]
        coefficient: String,
        /// Offset expression.
        offset: String,
    },
}

fn default_coefficient() -> String {
    "1".to_string()
}

// =============================================================================
// Expression AST for Template Variables
// =============================================================================

/// A parsed expression from template strings like "${instances[i-1]}".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodlExpr {
    /// Literal rational value.
    Literal(String),

    /// Variable reference (e.g., "gap", "curr").
    Variable(String),

    /// Array indexing (e.g., "instances[i-1]").
    ArrayIndex {
        array: String,
        index: Box<CodlExpr>,
    },

    /// Binary arithmetic operation.
    BinaryOp {
        left: Box<CodlExpr>,
        op: BinaryOp,
        right: Box<CodlExpr>,
    },

    /// Comparison for conditions.
    Comparison {
        left: Box<CodlExpr>,
        op: ComparisonOp,
        right: Box<CodlExpr>,
    },

    /// Null check (for optional parameters).
    IsNull(String),
    IsNotNull(String),
}

/// Binary arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

// =============================================================================
// Validation Metadata
// =============================================================================

/// Metadata collected during static validation.
#[derive(Debug, Clone, Default)]
pub struct ValidationMetadata {
    /// Maximum foreach nesting depth encountered.
    pub max_nesting_depth: usize,

    /// Total number of yields (upper bound on constraints).
    pub yield_count: usize,

    /// Variables referenced in expressions.
    pub referenced_variables: Vec<String>,

    /// Array index expressions that need bounds checking.
    pub array_accesses: Vec<ArrayAccessInfo>,
}

/// Information about an array access for bounds checking.
#[derive(Debug, Clone)]
pub struct ArrayAccessInfo {
    /// The array variable being accessed.
    pub array_name: String,

    /// The index expression (for error reporting).
    pub index_expr: String,

    /// Minimum index value (computed from where clauses).
    pub min_index: Option<i64>,

    /// Whether this access is guarded by a where clause.
    pub is_guarded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_codl() {
        let yaml = r#"
name: test_command
version: "1.0.0"
description: "Test command"
parameters:
  - name: target
    type: EntityId
  - name: value
    type: Rational
    default: "0"
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${value}"
"#;

        let cmd: CodlCommand = serde_yaml::from_str(yaml).expect("Failed to parse CODL");
        assert_eq!(cmd.name, "test_command");
        assert_eq!(cmd.parameters.len(), 2);
        assert_eq!(cmd.operations.len(), 1);
    }

    #[test]
    fn test_parse_foreach_codl() {
        let yaml = r#"
name: stack_vertical
version: "1.0.0"
parameters:
  - name: instances
    type: Array<EntityId>
  - name: gap
    type: Rational
    default: "0"
operations:
  - foreach:
      item: curr
      index: i
      in: instances
    where: "i > 0"
    yield:
      type: constraint
      target: "${curr}"
      component: y
      relation: eq
      term:
        type: linear
        entity_id: "${instances[i-1]}"
        component: y
        offset: "${gap}"
"#;

        let cmd: CodlCommand = serde_yaml::from_str(yaml).expect("Failed to parse CODL");
        assert_eq!(cmd.name, "stack_vertical");
        assert_eq!(cmd.parameters.len(), 2);

        match &cmd.operations[0] {
            CodlOperation::Foreach(foreach) => {
                assert_eq!(foreach.foreach.item, "curr");
                assert_eq!(foreach.foreach.index, "i");
                assert_eq!(foreach.r#where, Some("i > 0".to_string()));
            }
            _ => panic!("Expected Foreach operation"),
        }
    }

    #[test]
    fn test_codl_type_serde() {
        let yaml = r#"type: Array<EntityId>"#;

        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(rename = "type")]
            t: CodlType,
        }

        let w: Wrapper = serde_yaml::from_str(yaml).expect("Failed to parse type");
        assert_eq!(w.t, CodlType::ArrayEntityId);
    }
}
