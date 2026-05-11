//! ViewScript Constraint Operation Description Language (CODL)
//!
//! CODL is a declarative YAML/JSON DSL for defining constraint generation macros.
//! It enables LLMs and users to define reusable layout patterns without modifying
//! the core ViewScript binary.
//!
//! ## Design Philosophy
//!
//! ### Non-Turing-Complete by Design
//! CODL deliberately restricts expressiveness to guarantee:
//! - **Termination**: All CODL programs halt (no unbounded recursion)
//! - **Boundedness**: Output size is O(N * D) where N = input array length, D = nesting depth
//! - **Safety**: No side effects, pure constraint generation
//!
//! ### Static Verification
//! Before execution, CODL files undergo static analysis to verify:
//! 1. Nesting depth <= 3 (termination guarantee)
//! 2. Type correctness (parameters match usage)
//! 3. Array bounds (index expressions are within bounds given where clauses)
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                      CODL Processing Pipeline                           │
//! └─────────────────────────────────────────────────────────────────────────┘
//!
//!   ┌──────────────┐     ┌──────────────────┐     ┌──────────────────────┐
//!   │ YAML File    │     │  Parser          │     │  AST                 │
//!   │ (.vscmd.yaml)│────▶│  (serde_yaml)    │────▶│  (CodlCommand)       │
//!   └──────────────┘     └──────────────────┘     └──────────────────────┘
//!          │
//!          ▼
//!   ┌──────────────┐     ┌──────────────────┐     ┌──────────────────────┐
//!   │ Static       │     │  Validation      │     │  ValidationResult    │
//!   │ Validator    │────▶│  (depth, types,  │────▶│  (Ok or Errors)      │
//!   │              │     │   bounds)        │     │                      │
//!   └──────────────┘     └──────────────────┘     └──────────────────────┘
//!          │
//!          ▼
//!   ┌──────────────┐     ┌──────────────────┐     ┌──────────────────────┐
//!   │ Arguments    │     │  Interpreter     │     │  Vec<Constraint>     │
//!   │ (JSON)       │────▶│  (evaluate +     │────▶│  (P-dimension)       │
//!   │              │     │   bind)          │     │                      │
//!   └──────────────┘     └──────────────────┘     └──────────────────────┘
//! ```

pub mod ast;
pub mod parser;
pub mod validator;
pub mod interpreter;
pub mod error;

pub use ast::*;
pub use parser::*;
pub use validator::*;
pub use interpreter::*;
pub use error::*;
