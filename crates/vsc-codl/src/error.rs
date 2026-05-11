//! Error Types for CODL Processing
//!
//! Defines errors that can occur during parsing, validation, and interpretation.

use thiserror::Error;

/// Result type for CODL operations.
pub type CodlResult<T> = Result<T, CodlError>;

/// Errors that can occur during CODL processing.
#[derive(Debug, Error)]
pub enum CodlError {
    /// Error parsing YAML structure.
    #[error("YAML parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    /// Error parsing expression.
    #[error("Expression parse error: {0}")]
    ParseError(String),

    /// Static validation error.
    #[error("Validation error: {0}")]
    ValidationError(ValidationError),

    /// Error during interpretation.
    #[error("Interpretation error: {0}")]
    InterpretationError(String),

    /// Type mismatch error.
    #[error("Type error: expected {expected}, got {actual}")]
    TypeError { expected: String, actual: String },

    /// Missing required parameter.
    #[error("Missing required parameter: {0}")]
    MissingParameter(String),

    /// Array index out of bounds.
    #[error("Array index out of bounds: {array}[{index}] (length: {length})")]
    IndexOutOfBounds {
        array: String,
        index: i64,
        length: usize,
    },

    /// Unknown variable reference.
    #[error("Unknown variable: {0}")]
    UnknownVariable(String),

    /// Division by zero.
    #[error("Division by zero in expression")]
    DivisionByZero,
}

/// Specific validation errors with location information.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Error code for programmatic handling.
    pub code: ValidationErrorCode,

    /// Human-readable message.
    pub message: String,

    /// Location in the CODL file (if available).
    pub location: Option<ValidationLocation>,

    /// Suggestions for fixing the error.
    pub suggestions: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(loc) = &self.location {
            write!(f, " at {}", loc)?;
        }
        Ok(())
    }
}

/// Validation error codes for precise error handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationErrorCode {
    /// Foreach nesting depth exceeds limit.
    NestingDepthExceeded,

    /// Type mismatch in parameter usage.
    TypeMismatch,

    /// Array index may be out of bounds.
    PotentialIndexOutOfBounds,

    /// Undefined variable reference.
    UndefinedVariable,

    /// Undefined parameter reference.
    UndefinedParameter,

    /// Invalid where clause expression.
    InvalidWhereClause,

    /// Circular reference detected.
    CircularReference,

    /// Missing required field.
    MissingRequiredField,

    /// Invalid expression syntax.
    InvalidExpression,
}

impl std::fmt::Display for ValidationErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::NestingDepthExceeded => "E001",
            Self::TypeMismatch => "E002",
            Self::PotentialIndexOutOfBounds => "E003",
            Self::UndefinedVariable => "E004",
            Self::UndefinedParameter => "E005",
            Self::InvalidWhereClause => "E006",
            Self::CircularReference => "E007",
            Self::MissingRequiredField => "E008",
            Self::InvalidExpression => "E009",
        };
        write!(f, "{}", code)
    }
}

/// Location in a CODL file.
#[derive(Debug, Clone)]
pub struct ValidationLocation {
    /// Operation index (0-based).
    pub operation_index: usize,

    /// Path within the operation (e.g., "foreach.yield.term").
    pub path: String,
}

impl std::fmt::Display for ValidationLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "operations[{}].{}", self.operation_index, self.path)
    }
}

/// Builder for validation errors.
pub struct ValidationErrorBuilder {
    code: ValidationErrorCode,
    message: String,
    location: Option<ValidationLocation>,
    suggestions: Vec<String>,
}

impl ValidationErrorBuilder {
    pub fn new(code: ValidationErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            location: None,
            suggestions: Vec::new(),
        }
    }

    pub fn at_operation(mut self, index: usize, path: impl Into<String>) -> Self {
        self.location = Some(ValidationLocation {
            operation_index: index,
            path: path.into(),
        });
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestions.push(suggestion.into());
        self
    }

    pub fn build(self) -> ValidationError {
        ValidationError {
            code: self.code,
            message: self.message,
            location: self.location,
            suggestions: self.suggestions,
        }
    }
}

impl From<ValidationError> for CodlError {
    fn from(err: ValidationError) -> Self {
        CodlError::ValidationError(err)
    }
}
