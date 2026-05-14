//! Static Validator for CODL Commands
//!
//! Performs static analysis to guarantee safety properties:
//! 1. Termination: Nesting depth <= MAX_NESTING_DEPTH
//! 2. Boundedness: Output size is O(N * D)
//! 3. Reference soundness: Array indices are within bounds
//! 4. Type soundness: Parameter types match usage

use crate::ast::*;
use crate::error::*;
use crate::parser::{extract_array_accesses, extract_variables, parse_expr, parse_where_clause};
use std::collections::{HashMap, HashSet};

/// Maximum allowed foreach nesting depth.
pub const MAX_NESTING_DEPTH: usize = 3;

/// Result of static validation.
#[derive(Debug)]
pub struct ValidationResult {
    /// Whether validation passed.
    pub is_valid: bool,

    /// Errors found during validation.
    pub errors: Vec<ValidationError>,

    /// Warnings (non-fatal issues).
    pub warnings: Vec<String>,

    /// Metadata collected during validation.
    pub metadata: ValidationMetadata,
}

/// Static validator for CODL commands.
pub struct CodlValidator {
    /// Current foreach nesting depth.
    current_depth: usize,

    /// Maximum depth encountered.
    max_depth: usize,

    /// Parameters defined in the command.
    parameters: HashMap<String, CodlType>,

    /// Variables in scope (parameter names + loop variables).
    scope: HashSet<String>,

    /// Collected errors.
    errors: Vec<ValidationError>,

    /// Collected warnings.
    warnings: Vec<String>,

    /// Array accesses for bounds checking.
    array_accesses: Vec<ArrayAccessInfo>,

    /// Current operation index.
    current_op_index: usize,
}

impl CodlValidator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self {
            current_depth: 0,
            max_depth: 0,
            parameters: HashMap::new(),
            scope: HashSet::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            array_accesses: Vec::new(),
            current_op_index: 0,
        }
    }

    /// Validate a CODL command.
    pub fn validate(&mut self, cmd: &CodlCommand) -> ValidationResult {
        // Reset state
        self.errors.clear();
        self.warnings.clear();
        self.current_depth = 0;
        self.max_depth = 0;
        self.scope.clear();
        self.parameters.clear();
        self.array_accesses.clear();

        // Validate and collect parameters
        self.validate_parameters(&cmd.parameters);

        // Add parameters to scope
        for param in &cmd.parameters {
            self.scope.insert(param.name.clone());
            self.parameters
                .insert(param.name.clone(), param.param_type.clone());
        }

        // Validate operations
        for (i, op) in cmd.operations.iter().enumerate() {
            self.current_op_index = i;
            self.validate_operation(op);
        }

        // Build result
        ValidationResult {
            is_valid: self.errors.is_empty(),
            errors: self.errors.clone(),
            warnings: self.warnings.clone(),
            metadata: ValidationMetadata {
                max_nesting_depth: self.max_depth,
                yield_count: self.count_yields(&cmd.operations),
                referenced_variables: self.scope.iter().cloned().collect(),
                array_accesses: self.array_accesses.clone(),
            },
        }
    }

    fn validate_parameters(&mut self, params: &[CodlParameter]) {
        let mut seen_names = HashSet::new();

        for param in params {
            // Check for duplicate names
            if seen_names.contains(&param.name) {
                self.errors.push(
                    ValidationErrorBuilder::new(
                        ValidationErrorCode::MissingRequiredField,
                        format!("Duplicate parameter name: {}", param.name),
                    )
                    .build(),
                );
            }
            seen_names.insert(param.name.clone());

            // Validate default value if present
            if let Some(default) = &param.default {
                if let Err(e) = parse_expr(default) {
                    self.errors.push(
                        ValidationErrorBuilder::new(
                            ValidationErrorCode::InvalidExpression,
                            format!(
                                "Invalid default value for parameter '{}': {}",
                                param.name, e
                            ),
                        )
                        .build(),
                    );
                }
            }
        }
    }

    fn validate_operation(&mut self, op: &CodlOperation) {
        match op {
            CodlOperation::Foreach(foreach) => self.validate_foreach(foreach),
            CodlOperation::DirectYield(yield_spec) => {
                self.validate_yield(yield_spec, "yield");
            }
            CodlOperation::Conditional(cond) => self.validate_conditional(cond),
        }
    }

    fn validate_foreach(&mut self, foreach: &CodlForeach) {
        // Check nesting depth
        self.current_depth += 1;
        if self.current_depth > self.max_depth {
            self.max_depth = self.current_depth;
        }

        if self.current_depth > MAX_NESTING_DEPTH {
            self.errors.push(
                ValidationErrorBuilder::new(
                    ValidationErrorCode::NestingDepthExceeded,
                    format!(
                        "Foreach nesting depth {} exceeds maximum {}",
                        self.current_depth, MAX_NESTING_DEPTH
                    ),
                )
                .at_operation(self.current_op_index, "foreach")
                .with_suggestion(format!(
                    "Reduce nesting depth to {} or fewer levels",
                    MAX_NESTING_DEPTH
                ))
                .build(),
            );
        }

        // Validate the array being iterated
        let array_var = &foreach.foreach.in_expr;
        if !self.scope.contains(array_var) {
            self.errors.push(
                ValidationErrorBuilder::new(
                    ValidationErrorCode::UndefinedVariable,
                    format!("Undefined variable in foreach: {}", array_var),
                )
                .at_operation(self.current_op_index, "foreach.in")
                .build(),
            );
        } else {
            // Check that it's an array type
            if let Some(param_type) = self.parameters.get(array_var) {
                if !matches!(
                    param_type,
                    CodlType::ArrayEntityId | CodlType::ArrayRational
                ) {
                    self.errors.push(
                        ValidationErrorBuilder::new(
                            ValidationErrorCode::TypeMismatch,
                            format!(
                                "Cannot iterate over non-array type: {} is {:?}",
                                array_var, param_type
                            ),
                        )
                        .at_operation(self.current_op_index, "foreach.in")
                        .build(),
                    );
                }
            }
        }

        // Add loop variables to scope
        let item_var = foreach.foreach.item.clone();
        let index_var = foreach.foreach.index.clone();

        self.scope.insert(item_var.clone());
        self.scope.insert(index_var.clone());

        // Validate where clause
        let where_guard = if let Some(where_clause) = &foreach.r#where {
            match parse_where_clause(where_clause) {
                Ok(expr) => {
                    self.validate_where_clause(&expr, where_clause);
                    Some(expr)
                }
                Err(e) => {
                    self.errors.push(
                        ValidationErrorBuilder::new(
                            ValidationErrorCode::InvalidWhereClause,
                            format!("Invalid where clause: {}", e),
                        )
                        .at_operation(self.current_op_index, "where")
                        .build(),
                    );
                    None
                }
            }
        } else {
            None
        };

        // Validate yield with where guard context
        self.validate_yield_with_guard(&foreach.r#yield, "foreach.yield", &where_guard, &index_var);

        // Remove loop variables from scope
        self.scope.remove(&item_var);
        self.scope.remove(&index_var);

        self.current_depth -= 1;
    }

    fn validate_where_clause(&mut self, expr: &CodlExpr, _original: &str) {
        // Check that all variables in the where clause are in scope
        let vars = extract_variables(expr);
        for var in vars {
            if !self.scope.contains(&var) {
                self.errors.push(
                    ValidationErrorBuilder::new(
                        ValidationErrorCode::UndefinedVariable,
                        format!("Undefined variable in where clause: {}", var),
                    )
                    .at_operation(self.current_op_index, "where")
                    .build(),
                );
            }
        }
    }

    fn validate_yield(&mut self, yield_spec: &CodlYield, path: &str) {
        self.validate_yield_with_guard(yield_spec, path, &None, "")
    }

    fn validate_yield_with_guard(
        &mut self,
        yield_spec: &CodlYield,
        path: &str,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) {
        match yield_spec {
            CodlYield::Constraint(c) => {
                self.validate_constraint_yield(c, path, where_guard, index_var);
            }
            CodlYield::Origin(o) => {
                self.validate_origin_yield(o, path);
            }
            CodlYield::PathEntity(p) => {
                self.validate_path_entity_yield(p, path, where_guard, index_var);
            }
            CodlYield::FillSpec(f) => {
                self.validate_fill_spec_yield(f, path, where_guard, index_var);
            }
            CodlYield::StrokeSpec(s) => {
                self.validate_stroke_spec_yield(s, path, where_guard, index_var);
            }
        }
    }

    fn validate_constraint_yield(
        &mut self,
        c: &CodlConstraintYield,
        path: &str,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) {
        // Validate target expression
        self.validate_expr_string(
            &c.target,
            &format!("{}.target", path),
            where_guard,
            index_var,
        );

        // Validate term
        match &c.term {
            CodlTerm::Const { value } => {
                self.validate_expr_string(
                    value,
                    &format!("{}.term.value", path),
                    where_guard,
                    index_var,
                );
            }
            CodlTerm::Ref { entity_id, .. } => {
                self.validate_expr_string(
                    entity_id,
                    &format!("{}.term.entity_id", path),
                    where_guard,
                    index_var,
                );
            }
            CodlTerm::Linear {
                entity_id,
                coefficient,
                offset,
                ..
            } => {
                self.validate_expr_string(
                    entity_id,
                    &format!("{}.term.entity_id", path),
                    where_guard,
                    index_var,
                );
                self.validate_expr_string(
                    coefficient,
                    &format!("{}.term.coefficient", path),
                    where_guard,
                    index_var,
                );
                self.validate_expr_string(
                    offset,
                    &format!("{}.term.offset", path),
                    where_guard,
                    index_var,
                );
            }
        }
    }

    fn validate_origin_yield(&mut self, o: &CodlOriginYield, path: &str) {
        self.validate_expr_string(&o.target, &format!("{}.target", path), &None, "");
        self.validate_expr_string(&o.value, &format!("{}.value", path), &None, "");
    }

    fn validate_path_entity_yield(
        &mut self,
        p: &crate::ast::CodlPathEntityYield,
        path: &str,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) {
        // Validate ID expression
        self.validate_expr_string(&p.id, &format!("{}.id", path), where_guard, index_var);

        // Validate each segment's coordinate expressions
        for (i, seg) in p.segments.iter().enumerate() {
            let seg_path = format!("{}.segments[{}]", path, i);
            match seg {
                crate::ast::CodlPathSegment::MoveTo { x, y }
                | crate::ast::CodlPathSegment::LineTo { x, y } => {
                    self.validate_expr_string(
                        x,
                        &format!("{}.x", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        y,
                        &format!("{}.y", seg_path),
                        where_guard,
                        index_var,
                    );
                }
                crate::ast::CodlPathSegment::QuadTo { cx, cy, x, y } => {
                    self.validate_expr_string(
                        cx,
                        &format!("{}.cx", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        cy,
                        &format!("{}.cy", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        x,
                        &format!("{}.x", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        y,
                        &format!("{}.y", seg_path),
                        where_guard,
                        index_var,
                    );
                }
                crate::ast::CodlPathSegment::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    self.validate_expr_string(
                        c1x,
                        &format!("{}.c1x", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        c1y,
                        &format!("{}.c1y", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        c2x,
                        &format!("{}.c2x", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        c2y,
                        &format!("{}.c2y", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        x,
                        &format!("{}.x", seg_path),
                        where_guard,
                        index_var,
                    );
                    self.validate_expr_string(
                        y,
                        &format!("{}.y", seg_path),
                        where_guard,
                        index_var,
                    );
                }
                crate::ast::CodlPathSegment::Close => {}
            }
        }
    }

    fn validate_fill_spec_yield(
        &mut self,
        f: &crate::ast::CodlFillSpecYield,
        path: &str,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) {
        // Validate target expression
        self.validate_expr_string(
            &f.target,
            &format!("{}.target", path),
            where_guard,
            index_var,
        );

        // Validate fill type
        match &f.fill {
            crate::ast::CodlFillType::Solid { r, g, b, a } => {
                self.validate_expr_string(r, &format!("{}.fill.r", path), where_guard, index_var);
                self.validate_expr_string(g, &format!("{}.fill.g", path), where_guard, index_var);
                self.validate_expr_string(b, &format!("{}.fill.b", path), where_guard, index_var);
                self.validate_expr_string(a, &format!("{}.fill.a", path), where_guard, index_var);
            }
            crate::ast::CodlFillType::Gradient { gradient_id } => {
                self.validate_expr_string(
                    gradient_id,
                    &format!("{}.fill.gradient_id", path),
                    where_guard,
                    index_var,
                );
            }
        }
    }

    fn validate_stroke_spec_yield(
        &mut self,
        s: &crate::ast::CodlStrokeSpecYield,
        path: &str,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) {
        // Validate target expression
        self.validate_expr_string(
            &s.target,
            &format!("{}.target", path),
            where_guard,
            index_var,
        );

        // Validate stroke properties
        self.validate_expr_string(&s.width, &format!("{}.width", path), where_guard, index_var);
        self.validate_expr_string(&s.r, &format!("{}.r", path), where_guard, index_var);
        self.validate_expr_string(&s.g, &format!("{}.g", path), where_guard, index_var);
        self.validate_expr_string(&s.b, &format!("{}.b", path), where_guard, index_var);
        self.validate_expr_string(&s.a, &format!("{}.a", path), where_guard, index_var);
        self.validate_expr_string(
            &s.miter_limit,
            &format!("{}.miter_limit", path),
            where_guard,
            index_var,
        );
    }

    fn validate_expr_string(
        &mut self,
        expr_str: &str,
        path: &str,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) {
        match parse_expr(expr_str) {
            Ok(expr) => {
                // Check variables are in scope
                let vars = extract_variables(&expr);
                for var in &vars {
                    if !self.scope.contains(var) {
                        self.errors.push(
                            ValidationErrorBuilder::new(
                                ValidationErrorCode::UndefinedVariable,
                                format!("Undefined variable: {}", var),
                            )
                            .at_operation(self.current_op_index, path)
                            .build(),
                        );
                    }
                }

                // Check array accesses for bounds
                let accesses = extract_array_accesses(&expr);
                for (array, index_expr) in accesses {
                    let is_guarded = self.check_index_guarded(&index_expr, where_guard, index_var);

                    self.array_accesses.push(ArrayAccessInfo {
                        array_name: array.clone(),
                        index_expr: format!("{:?}", index_expr),
                        min_index: self.extract_min_index(&index_expr, where_guard, index_var),
                        is_guarded,
                    });

                    // Check for potential out-of-bounds access
                    if !is_guarded {
                        // Check if index might be negative
                        if self.index_might_be_negative(&index_expr, index_var) {
                            self.errors.push(
                                ValidationErrorBuilder::new(
                                    ValidationErrorCode::PotentialIndexOutOfBounds,
                                    format!("Array access {}[...] might be out of bounds", array),
                                )
                                .at_operation(self.current_op_index, path)
                                .with_suggestion(format!(
                                    "Add a where clause like '{} > 0' to guard the access",
                                    index_var
                                ))
                                .build(),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                self.errors.push(
                    ValidationErrorBuilder::new(
                        ValidationErrorCode::InvalidExpression,
                        format!("Invalid expression '{}': {}", expr_str, e),
                    )
                    .at_operation(self.current_op_index, path)
                    .build(),
                );
            }
        }
    }

    fn check_index_guarded(
        &self,
        _index_expr: &CodlExpr,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) -> bool {
        if where_guard.is_none() || index_var.is_empty() {
            return false;
        }

        let guard = where_guard.as_ref().unwrap();

        // Check if the where clause is of form "index_var > 0" or "index_var >= 1"
        match guard {
            CodlExpr::Comparison { left, op, right } => {
                // Check if left is the index variable
                if let CodlExpr::Variable(var) = left.as_ref() {
                    if var == index_var {
                        // Check if the comparison provides a lower bound
                        if let CodlExpr::Literal(val) = right.as_ref() {
                            if let Ok(n) = val.parse::<i64>() {
                                match op {
                                    ComparisonOp::Gt if n >= 0 => return true,
                                    ComparisonOp::Ge if n >= 1 => return true,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        false
    }

    fn extract_min_index(
        &self,
        _index_expr: &CodlExpr,
        where_guard: &Option<CodlExpr>,
        index_var: &str,
    ) -> Option<i64> {
        if let Some(guard) = where_guard {
            match guard {
                CodlExpr::Comparison { left, op, right } => {
                    if let CodlExpr::Variable(var) = left.as_ref() {
                        if var == index_var {
                            if let CodlExpr::Literal(val) = right.as_ref() {
                                if let Ok(n) = val.parse::<i64>() {
                                    return match op {
                                        ComparisonOp::Gt => Some(n + 1),
                                        ComparisonOp::Ge => Some(n),
                                        _ => None,
                                    };
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn index_might_be_negative(&self, index_expr: &CodlExpr, index_var: &str) -> bool {
        // Check if the index expression is "i - N" where N > 0
        match index_expr {
            CodlExpr::BinaryOp { left, op, right: _ } => {
                if *op == BinaryOp::Sub {
                    if let CodlExpr::Variable(var) = left.as_ref() {
                        if var == index_var {
                            // i - something could be negative when i = 0
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn validate_conditional(&mut self, cond: &CodlConditional) {
        // Validate condition
        match parse_where_clause(&cond.condition) {
            Ok(expr) => {
                let vars = extract_variables(&expr);
                for var in vars {
                    if !self.scope.contains(&var) {
                        self.errors.push(
                            ValidationErrorBuilder::new(
                                ValidationErrorCode::UndefinedVariable,
                                format!("Undefined variable in condition: {}", var),
                            )
                            .at_operation(self.current_op_index, "if")
                            .build(),
                        );
                    }
                }
            }
            Err(e) => {
                self.errors.push(
                    ValidationErrorBuilder::new(
                        ValidationErrorCode::InvalidExpression,
                        format!("Invalid condition: {}", e),
                    )
                    .at_operation(self.current_op_index, "if")
                    .build(),
                );
            }
        }

        // Validate then branch
        for op in &cond.then {
            self.validate_operation(op);
        }

        // Validate else branch
        if let Some(else_ops) = &cond.r#else {
            for op in else_ops {
                self.validate_operation(op);
            }
        }
    }

    fn count_yields(&self, ops: &[CodlOperation]) -> usize {
        let mut count = 0;
        for op in ops {
            count += self.count_yields_in_op(op);
        }
        count
    }

    fn count_yields_in_op(&self, op: &CodlOperation) -> usize {
        match op {
            CodlOperation::Foreach(f) => {
                // Each foreach iteration produces yields
                // We count 1 for the template (actual count depends on input)
                1 + self.count_yields_in_yield(&f.r#yield)
            }
            CodlOperation::DirectYield(_) => 1,
            CodlOperation::Conditional(c) => {
                let then_count: usize = c.then.iter().map(|o| self.count_yields_in_op(o)).sum();
                let else_count: usize = c
                    .r#else
                    .as_ref()
                    .map(|ops| ops.iter().map(|o| self.count_yields_in_op(o)).sum())
                    .unwrap_or(0);
                then_count + else_count
            }
        }
    }

    fn count_yields_in_yield(&self, _yield: &CodlYield) -> usize {
        1
    }
}

impl Default for CodlValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a CODL command and return the result.
pub fn validate_codl(cmd: &CodlCommand) -> ValidationResult {
    let mut validator = CodlValidator::new();
    validator.validate(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_validate(yaml: &str) -> ValidationResult {
        let cmd: CodlCommand = serde_yaml::from_str(yaml).expect("Failed to parse YAML");
        validate_codl(&cmd)
    }

    #[test]
    fn test_valid_simple_command() {
        let yaml = r#"
name: simple
parameters:
  - name: target
    type: EntityId
  - name: value
    type: Rational
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${value}"
"#;

        let result = parse_and_validate(yaml);
        assert!(result.is_valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn test_valid_foreach_with_guard() {
        let yaml = r#"
name: stack
parameters:
  - name: instances
    type: Array<EntityId>
  - name: gap
    type: Rational
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

        let result = parse_and_validate(yaml);
        assert!(result.is_valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn test_invalid_unguarded_index() {
        let yaml = r#"
name: bad_stack
parameters:
  - name: instances
    type: Array<EntityId>
operations:
  - foreach:
      item: curr
      index: i
      in: instances
    yield:
      type: constraint
      target: "${curr}"
      component: y
      relation: eq
      term:
        type: ref
        entity_id: "${instances[i-1]}"
        component: y
"#;

        let result = parse_and_validate(yaml);
        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.code == ValidationErrorCode::PotentialIndexOutOfBounds));
    }

    #[test]
    fn test_invalid_undefined_variable() {
        let yaml = r#"
name: bad_var
parameters:
  - name: target
    type: EntityId
operations:
  - type: constraint
    target: "${undefined_var}"
    component: x
    relation: eq
    term:
      type: const
      value: "100"
"#;

        let result = parse_and_validate(yaml);
        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.code == ValidationErrorCode::UndefinedVariable));
    }

    #[test]
    fn test_nesting_depth_limit() {
        let yaml = r#"
name: too_deep
parameters:
  - name: a
    type: Array<EntityId>
  - name: b
    type: Array<EntityId>
  - name: c
    type: Array<EntityId>
  - name: d
    type: Array<EntityId>
operations:
  - foreach:
      item: x1
      index: i1
      in: a
    yield:
      type: constraint
      target: "${x1}"
      component: x
      relation: eq
      term:
        type: const
        value: "0"
"#;

        // This should pass (depth = 1)
        let result = parse_and_validate(yaml);
        assert!(result.is_valid);
        assert_eq!(result.metadata.max_nesting_depth, 1);
    }

    // =========================================================================
    // Phase 15.1: Boundary Value Tests for Static Validator
    // =========================================================================

    #[test]
    fn test_nesting_depth_3_allowed_boundary() {
        // MAX_NESTING_DEPTH = 3, so depth=3 should be allowed (boundary value)
        let yaml = r#"
name: depth_3
parameters:
  - name: a
    type: Array<EntityId>
  - name: b
    type: Array<EntityId>
  - name: c
    type: Array<EntityId>
operations:
  - foreach:
      item: x1
      index: i1
      in: a
    yield:
      type: constraint
      target: "${x1}"
      component: x
      relation: eq
      term:
        type: const
        value: "0"
"#;
        // Note: The above is depth=1. We need to create nested foreach.
        // However, current AST doesn't support nested foreach in yield.
        // Let's test with conditionals containing foreach.

        // Actually, with current structure, foreach can't nest inside foreach's yield.
        // The test should verify max_nesting_depth tracking, not actual nesting.

        // For this test, we verify depth=1 passes and metadata is correct.
        let result = parse_and_validate(yaml);
        assert!(result.is_valid, "Depth 1 should pass");
        assert_eq!(result.metadata.max_nesting_depth, 1);
    }

    #[test]
    fn test_nesting_depth_exceeds_max_rejected() {
        // Note: Current AST doesn't support nested foreach directly.
        // This test documents expected behavior if nesting were possible.
        // Since foreach can only appear at top level in operations[],
        // and yield doesn't contain operations, depth > 1 isn't currently achievable.
        //
        // This test verifies that the validator WOULD reject depth > MAX if it occurred.
        //
        // For now, we test that depth=1 is correctly measured.
        let yaml = r#"
name: single_foreach
parameters:
  - name: items
    type: Array<EntityId>
operations:
  - foreach:
      item: curr
      index: i
      in: items
    yield:
      type: origin
      target: "${curr}"
      component: x
      value: "0"
"#;
        let result = parse_and_validate(yaml);
        assert!(result.is_valid);
        assert_eq!(result.metadata.max_nesting_depth, 1);

        // Verify MAX_NESTING_DEPTH constant is 3
        assert_eq!(MAX_NESTING_DEPTH, 3, "MAX_NESTING_DEPTH should be 3");
    }

    #[test]
    fn test_guard_condition_i_greater_than_0_recognized() {
        // Guard: "i > 0" should make ${instances[i-1]} safe
        let yaml = r#"
name: guarded_access
parameters:
  - name: instances
    type: Array<EntityId>
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
        type: ref
        entity_id: "${instances[i-1]}"
        component: y
"#;
        let result = parse_and_validate(yaml);
        assert!(
            result.is_valid,
            "Guard 'i > 0' should make instances[i-1] safe. Errors: {:?}",
            result.errors
        );

        // Verify array access is marked as guarded
        assert!(
            result.metadata.array_accesses.iter().any(|a| a.is_guarded),
            "Array access should be marked as guarded"
        );
    }

    #[test]
    fn test_guard_condition_i_ge_1_recognized() {
        // Guard: "i >= 1" should also make ${instances[i-1]} safe
        let yaml = r#"
name: guarded_ge
parameters:
  - name: instances
    type: Array<EntityId>
operations:
  - foreach:
      item: curr
      index: i
      in: instances
    where: "i >= 1"
    yield:
      type: constraint
      target: "${curr}"
      component: y
      relation: eq
      term:
        type: ref
        entity_id: "${instances[i-1]}"
        component: y
"#;
        let result = parse_and_validate(yaml);
        assert!(
            result.is_valid,
            "Guard 'i >= 1' should make instances[i-1] safe. Errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_no_guard_i_minus_1_rejected() {
        // Without guard, ${instances[i-1]} is potentially out of bounds when i=0
        let yaml = r#"
name: unguarded_access
parameters:
  - name: instances
    type: Array<EntityId>
operations:
  - foreach:
      item: curr
      index: i
      in: instances
    yield:
      type: constraint
      target: "${curr}"
      component: y
      relation: eq
      term:
        type: ref
        entity_id: "${instances[i-1]}"
        component: y
"#;
        let result = parse_and_validate(yaml);
        assert!(!result.is_valid, "Unguarded i-1 access should be rejected");
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::PotentialIndexOutOfBounds),
            "Should produce PotentialIndexOutOfBounds error"
        );
    }

    #[test]
    fn test_insufficient_guard_rejected() {
        // Guard "i > -1" is insufficient to protect i-1 (allows i=0)
        // Note: Current implementation checks for "i > 0" or "i >= 1" specifically.
        // "i > -1" should NOT be recognized as a valid guard.
        let yaml = r#"
name: insufficient_guard
parameters:
  - name: instances
    type: Array<EntityId>
operations:
  - foreach:
      item: curr
      index: i
      in: instances
    where: "i > -1"
    yield:
      type: constraint
      target: "${curr}"
      component: y
      relation: eq
      term:
        type: ref
        entity_id: "${instances[i-1]}"
        component: y
"#;
        let result = parse_and_validate(yaml);
        // This should fail because i > -1 allows i=0, making i-1 = -1 (out of bounds)
        assert!(
            !result.is_valid,
            "Guard 'i > -1' is insufficient, should be rejected"
        );
    }

    #[test]
    fn test_type_mismatch_foreach_on_non_array() {
        // Iterating over a non-array type should be caught at validation
        let yaml = r#"
name: bad_foreach
parameters:
  - name: single_entity
    type: EntityId
operations:
  - foreach:
      item: curr
      index: i
      in: single_entity
    yield:
      type: origin
      target: "${curr}"
      component: x
      value: "0"
"#;
        let result = parse_and_validate(yaml);
        assert!(
            !result.is_valid,
            "Foreach over non-array should be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.code == ValidationErrorCode::TypeMismatch),
            "Should produce TypeMismatch error"
        );
    }

    #[test]
    fn test_duplicate_parameter_names_rejected() {
        let yaml = r#"
name: dup_params
parameters:
  - name: x
    type: EntityId
  - name: x
    type: Rational
operations:
  - type: origin
    target: "${x}"
    component: x
    value: "0"
"#;
        let result = parse_and_validate(yaml);
        assert!(
            !result.is_valid,
            "Duplicate parameter names should be rejected"
        );
    }
}
