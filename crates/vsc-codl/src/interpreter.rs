//! CODL Interpreter
//!
//! Evaluates validated CODL commands with actual JSON arguments to produce
//! P-dimension constraints.
//!
//! ## Safety Guarantees
//!
//! The interpreter assumes the CODL has been validated by `CodlValidator`.
//! This means:
//! - All variables are defined
//! - Array accesses are properly guarded
//! - Nesting depth is within limits
//!
//! The interpreter does NOT re-validate; it trusts the validator.

use crate::ast::*;
use crate::error::{CodlError, CodlResult};
use crate::parser::{parse_expr, parse_where_clause};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use vsc_core::types::{
    Constraint, ConstraintPriority, ConstraintTerm, EntityId, Rational, RelationType,
    VectorComponent,
};

/// Runtime value in CODL evaluation.
#[derive(Debug, Clone)]
pub enum CodlValue {
    /// A rational number.
    Rational(Rational),
    /// An entity ID.
    EntityId(EntityId),
    /// An array of entity IDs.
    ArrayEntityId(Vec<EntityId>),
    /// An array of rationals.
    ArrayRational(Vec<Rational>),
    /// A string value.
    String(String),
    /// A boolean value.
    Bool(bool),
    /// Null (for optional parameters).
    Null,
}

impl CodlValue {
    /// Try to get as EntityId.
    pub fn as_entity_id(&self) -> Option<EntityId> {
        match self {
            CodlValue::EntityId(id) => Some(*id),
            _ => None,
        }
    }

    /// Try to get as Rational.
    pub fn as_rational(&self) -> Option<&Rational> {
        match self {
            CodlValue::Rational(r) => Some(r),
            _ => None,
        }
    }

    /// Try to get as array of EntityIds.
    pub fn as_array_entity_id(&self) -> Option<&[EntityId]> {
        match self {
            CodlValue::ArrayEntityId(arr) => Some(arr),
            _ => None,
        }
    }

    /// Try to get as i64 (for indices).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            CodlValue::Rational(r) => {
                // Use the rational's string representation to check if it's an integer
                // Rational prints as "numer/denom", so we check if it simplifies to "n/1"
                let s = format!("{}", r);
                if let Some((numer_str, denom_str)) = s.split_once('/') {
                    if denom_str == "1" {
                        return numer_str.parse().ok();
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, CodlValue::Null)
    }
}

/// CODL interpreter that evaluates commands with arguments.
pub struct CodlInterpreter {
    /// Variable scope (parameters + loop variables).
    scope: HashMap<String, CodlValue>,
    /// Parameter type information.
    param_types: HashMap<String, CodlType>,
    /// Generated constraints.
    constraints: Vec<Constraint>,
    /// Next constraint ID.
    next_id: u64,
    /// Source scope for generated constraints.
    source_scope: Option<String>,
}

impl CodlInterpreter {
    /// Create a new interpreter.
    pub fn new() -> Self {
        Self {
            scope: HashMap::new(),
            param_types: HashMap::new(),
            constraints: Vec::new(),
            next_id: 1,
            source_scope: None,
        }
    }

    /// Set the starting constraint ID.
    pub fn with_start_id(mut self, id: u64) -> Self {
        self.next_id = id;
        self
    }

    /// Set the source scope for generated constraints.
    pub fn with_source_scope(mut self, scope: impl Into<String>) -> Self {
        self.source_scope = Some(scope.into());
        self
    }

    /// Execute a CODL command with JSON arguments.
    ///
    /// Returns the generated constraints.
    pub fn execute(
        &mut self,
        cmd: &CodlCommand,
        args: &JsonValue,
    ) -> CodlResult<Vec<Constraint>> {
        // Reset state
        self.scope.clear();
        self.param_types.clear();
        self.constraints.clear();

        // Bind parameters
        self.bind_parameters(&cmd.parameters, args)?;

        // Execute operations
        for op in &cmd.operations {
            self.execute_operation(op)?;
        }

        Ok(std::mem::take(&mut self.constraints))
    }

    /// Bind JSON arguments to CODL parameters.
    fn bind_parameters(
        &mut self,
        params: &[CodlParameter],
        args: &JsonValue,
    ) -> CodlResult<()> {
        let args_obj = args.as_object().ok_or_else(|| {
            CodlError::InterpretationError("Arguments must be a JSON object".to_string())
        })?;

        for param in params {
            self.param_types
                .insert(param.name.clone(), param.param_type.clone());

            let value = if let Some(json_val) = args_obj.get(&param.name) {
                self.json_to_codl_value(json_val, &param.param_type)?
            } else if let Some(default_str) = &param.default {
                // Use default value
                self.parse_default_value(default_str, &param.param_type)?
            } else {
                // Optional parameter with no default
                CodlValue::Null
            };

            self.scope.insert(param.name.clone(), value);
        }

        Ok(())
    }

    /// Convert JSON value to CODL value based on expected type.
    fn json_to_codl_value(
        &self,
        json: &JsonValue,
        expected_type: &CodlType,
    ) -> CodlResult<CodlValue> {
        match expected_type {
            CodlType::Rational => {
                let r = self.parse_rational_json(json)?;
                Ok(CodlValue::Rational(r))
            }
            CodlType::EntityId => {
                let id = json
                    .as_u64()
                    .ok_or_else(|| CodlError::TypeError {
                        expected: "EntityId (u64)".to_string(),
                        actual: format!("{:?}", json),
                    })?;
                Ok(CodlValue::EntityId(EntityId(id)))
            }
            CodlType::ArrayEntityId => {
                let arr = json.as_array().ok_or_else(|| CodlError::TypeError {
                    expected: "Array<EntityId>".to_string(),
                    actual: format!("{:?}", json),
                })?;
                let ids: Result<Vec<_>, _> = arr
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .map(EntityId)
                            .ok_or_else(|| CodlError::TypeError {
                                expected: "EntityId (u64)".to_string(),
                                actual: format!("{:?}", v),
                            })
                    })
                    .collect();
                Ok(CodlValue::ArrayEntityId(ids?))
            }
            CodlType::ArrayRational => {
                let arr = json.as_array().ok_or_else(|| CodlError::TypeError {
                    expected: "Array<Rational>".to_string(),
                    actual: format!("{:?}", json),
                })?;
                let rationals: Result<Vec<_>, _> =
                    arr.iter().map(|v| self.parse_rational_json(v)).collect();
                Ok(CodlValue::ArrayRational(rationals?))
            }
            CodlType::String => {
                let s = json.as_str().ok_or_else(|| CodlError::TypeError {
                    expected: "String".to_string(),
                    actual: format!("{:?}", json),
                })?;
                Ok(CodlValue::String(s.to_string()))
            }
            CodlType::Bool => {
                let b = json.as_bool().ok_or_else(|| CodlError::TypeError {
                    expected: "Bool".to_string(),
                    actual: format!("{:?}", json),
                })?;
                Ok(CodlValue::Bool(b))
            }
        }
    }

    /// Parse a rational from JSON (supports "n/d" string or integer).
    fn parse_rational_json(&self, json: &JsonValue) -> CodlResult<Rational> {
        match json {
            JsonValue::String(s) => self.parse_rational_str(s),
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Rational::from_int(i))
                } else {
                    Err(CodlError::TypeError {
                        expected: "Rational (integer or \"n/d\")".to_string(),
                        actual: format!("{}", n),
                    })
                }
            }
            _ => Err(CodlError::TypeError {
                expected: "Rational".to_string(),
                actual: format!("{:?}", json),
            }),
        }
    }

    /// Parse a rational from string "n/d" or integer string.
    fn parse_rational_str(&self, s: &str) -> CodlResult<Rational> {
        if s.contains('/') {
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() != 2 {
                return Err(CodlError::ParseError(format!(
                    "Invalid rational format: {}",
                    s
                )));
            }
            let numer: i64 = parts[0].trim().parse().map_err(|_| {
                CodlError::ParseError(format!("Invalid numerator: {}", parts[0]))
            })?;
            let denom: i64 = parts[1].trim().parse().map_err(|_| {
                CodlError::ParseError(format!("Invalid denominator: {}", parts[1]))
            })?;
            if denom == 0 {
                return Err(CodlError::DivisionByZero);
            }
            Ok(Rational::new(numer, denom))
        } else {
            let n: i64 = s.trim().parse().map_err(|_| {
                CodlError::ParseError(format!("Invalid integer: {}", s))
            })?;
            Ok(Rational::from_int(n))
        }
    }

    /// Parse default value string to CODL value.
    fn parse_default_value(
        &self,
        default_str: &str,
        expected_type: &CodlType,
    ) -> CodlResult<CodlValue> {
        match expected_type {
            CodlType::Rational => {
                let r = self.parse_rational_str(default_str)?;
                Ok(CodlValue::Rational(r))
            }
            CodlType::String => Ok(CodlValue::String(default_str.to_string())),
            CodlType::Bool => {
                let b = default_str.parse::<bool>().map_err(|_| {
                    CodlError::ParseError(format!("Invalid boolean: {}", default_str))
                })?;
                Ok(CodlValue::Bool(b))
            }
            _ => Err(CodlError::InterpretationError(format!(
                "Default values not supported for type {:?}",
                expected_type
            ))),
        }
    }

    /// Execute a single operation.
    fn execute_operation(&mut self, op: &CodlOperation) -> CodlResult<()> {
        match op {
            CodlOperation::Foreach(foreach) => self.execute_foreach(foreach),
            CodlOperation::DirectYield(yield_spec) => self.execute_yield(yield_spec),
            CodlOperation::Conditional(cond) => self.execute_conditional(cond),
        }
    }

    /// Execute a foreach loop.
    fn execute_foreach(&mut self, foreach: &CodlForeach) -> CodlResult<()> {
        let array_name = &foreach.foreach.in_expr;
        let item_var = &foreach.foreach.item;
        let index_var = &foreach.foreach.index;

        // Get the array to iterate
        let array = self
            .scope
            .get(array_name)
            .ok_or_else(|| CodlError::UnknownVariable(array_name.clone()))?
            .clone();

        let items = match &array {
            CodlValue::ArrayEntityId(arr) => arr
                .iter()
                .map(|id| CodlValue::EntityId(*id))
                .collect::<Vec<_>>(),
            CodlValue::ArrayRational(arr) => arr
                .iter()
                .map(|r| CodlValue::Rational(r.clone()))
                .collect::<Vec<_>>(),
            _ => {
                return Err(CodlError::TypeError {
                    expected: "Array".to_string(),
                    actual: format!("{:?}", array),
                })
            }
        };

        // Parse where clause once
        let where_expr = if let Some(where_str) = &foreach.r#where {
            Some(parse_where_clause(where_str)?)
        } else {
            None
        };

        // Iterate
        for (i, item) in items.iter().enumerate() {
            // Set loop variables
            self.scope.insert(item_var.clone(), item.clone());
            self.scope
                .insert(index_var.clone(), CodlValue::Rational(Rational::from_int(i as i64)));

            // Check where clause
            if let Some(ref expr) = where_expr {
                if !self.evaluate_condition(expr)? {
                    continue;
                }
            }

            // Execute yield
            self.execute_yield(&foreach.r#yield)?;
        }

        // Clean up loop variables
        self.scope.remove(item_var);
        self.scope.remove(index_var);

        Ok(())
    }

    /// Execute a conditional operation.
    fn execute_conditional(&mut self, cond: &CodlConditional) -> CodlResult<()> {
        let condition_expr = parse_where_clause(&cond.condition)?;
        let result = self.evaluate_condition(&condition_expr)?;

        if result {
            for op in &cond.then {
                self.execute_operation(op)?;
            }
        } else if let Some(else_ops) = &cond.r#else {
            for op in else_ops {
                self.execute_operation(op)?;
            }
        }

        Ok(())
    }

    /// Execute a yield to generate a constraint.
    fn execute_yield(&mut self, yield_spec: &CodlYield) -> CodlResult<()> {
        match yield_spec {
            CodlYield::Constraint(c) => self.yield_constraint(c),
            CodlYield::Origin(o) => self.yield_origin(o),
        }
    }

    /// Generate a constraint from yield specification.
    fn yield_constraint(&mut self, c: &CodlConstraintYield) -> CodlResult<()> {
        let target = self.evaluate_entity_id(&c.target)?;
        let component = self.codl_to_core_component(c.component);
        let relation = self.codl_to_core_relation(c.relation);
        let term = self.evaluate_term(&c.term)?;
        let priority = self.codl_to_core_priority(c.priority);

        let constraint = Constraint {
            id: self.next_id,
            target,
            component,
            relation,
            term,
            priority,
            source_scope: self.source_scope.clone(),
        };

        self.next_id += 1;
        self.constraints.push(constraint);

        Ok(())
    }

    /// Generate an origin constraint (shorthand for Const term).
    fn yield_origin(&mut self, o: &CodlOriginYield) -> CodlResult<()> {
        let target = self.evaluate_entity_id(&o.target)?;
        let component = self.codl_to_core_component(o.component);
        let value = self.evaluate_rational(&o.value)?;
        let priority = self.codl_to_core_priority(o.priority);

        let constraint = Constraint {
            id: self.next_id,
            target,
            component,
            relation: RelationType::Eq,
            term: ConstraintTerm::Const { value },
            priority,
            source_scope: self.source_scope.clone(),
        };

        self.next_id += 1;
        self.constraints.push(constraint);

        Ok(())
    }

    /// Evaluate a term specification.
    fn evaluate_term(&mut self, term: &CodlTerm) -> CodlResult<ConstraintTerm> {
        match term {
            CodlTerm::Const { value } => {
                let r = self.evaluate_rational(value)?;
                Ok(ConstraintTerm::Const { value: r })
            }
            CodlTerm::Ref {
                entity_id,
                component,
            } => {
                let id = self.evaluate_entity_id(entity_id)?;
                let comp = self.codl_to_core_component(*component);
                Ok(ConstraintTerm::Ref {
                    entity_id: id,
                    component: comp,
                })
            }
            CodlTerm::Linear {
                entity_id,
                component,
                coefficient,
                offset,
            } => {
                let id = self.evaluate_entity_id(entity_id)?;
                let comp = self.codl_to_core_component(*component);
                let coef = self.evaluate_rational(coefficient)?;
                let off = self.evaluate_rational(offset)?;
                Ok(ConstraintTerm::Linear {
                    coefficient: coef,
                    entity_id: id,
                    component: comp,
                    offset: off,
                })
            }
        }
    }

    /// Evaluate an expression string to get an EntityId.
    fn evaluate_entity_id(&self, expr_str: &str) -> CodlResult<EntityId> {
        let expr = parse_expr(expr_str)?;
        let value = self.evaluate_expr(&expr)?;
        value.as_entity_id().ok_or_else(|| CodlError::TypeError {
            expected: "EntityId".to_string(),
            actual: format!("{:?}", value),
        })
    }

    /// Evaluate an expression string to get a Rational.
    fn evaluate_rational(&self, expr_str: &str) -> CodlResult<Rational> {
        let expr = parse_expr(expr_str)?;
        let value = self.evaluate_expr(&expr)?;
        match value {
            CodlValue::Rational(r) => Ok(r),
            _ => Err(CodlError::TypeError {
                expected: "Rational".to_string(),
                actual: format!("{:?}", value),
            }),
        }
    }

    /// Evaluate a CODL expression to a value.
    fn evaluate_expr(&self, expr: &CodlExpr) -> CodlResult<CodlValue> {
        match expr {
            CodlExpr::Literal(s) => {
                // Try to parse as rational
                if s.contains('/') {
                    let parts: Vec<&str> = s.split('/').collect();
                    if parts.len() == 2 {
                        let numer: i64 = parts[0].trim().parse().map_err(|_| {
                            CodlError::ParseError(format!("Invalid numerator: {}", parts[0]))
                        })?;
                        let denom: i64 = parts[1].trim().parse().map_err(|_| {
                            CodlError::ParseError(format!("Invalid denominator: {}", parts[1]))
                        })?;
                        return Ok(CodlValue::Rational(Rational::new(numer, denom)));
                    }
                }
                // Try as integer
                if let Ok(n) = s.parse::<i64>() {
                    return Ok(CodlValue::Rational(Rational::from_int(n)));
                }
                // Return as string
                Ok(CodlValue::String(s.clone()))
            }
            CodlExpr::Variable(name) => self
                .scope
                .get(name)
                .cloned()
                .ok_or_else(|| CodlError::UnknownVariable(name.clone())),
            CodlExpr::ArrayIndex { array, index } => {
                let arr_val = self
                    .scope
                    .get(array)
                    .ok_or_else(|| CodlError::UnknownVariable(array.clone()))?;
                let idx_val = self.evaluate_expr(index)?;
                let idx = idx_val.as_i64().ok_or_else(|| CodlError::TypeError {
                    expected: "integer index".to_string(),
                    actual: format!("{:?}", idx_val),
                })?;

                match arr_val {
                    CodlValue::ArrayEntityId(arr) => {
                        if idx < 0 || idx as usize >= arr.len() {
                            return Err(CodlError::IndexOutOfBounds {
                                array: array.clone(),
                                index: idx,
                                length: arr.len(),
                            });
                        }
                        Ok(CodlValue::EntityId(arr[idx as usize]))
                    }
                    CodlValue::ArrayRational(arr) => {
                        if idx < 0 || idx as usize >= arr.len() {
                            return Err(CodlError::IndexOutOfBounds {
                                array: array.clone(),
                                index: idx,
                                length: arr.len(),
                            });
                        }
                        Ok(CodlValue::Rational(arr[idx as usize].clone()))
                    }
                    _ => Err(CodlError::TypeError {
                        expected: "Array".to_string(),
                        actual: format!("{:?}", arr_val),
                    }),
                }
            }
            CodlExpr::BinaryOp { left, op, right } => {
                let l = self.evaluate_expr(left)?;
                let r = self.evaluate_expr(right)?;

                let l_rat = l.as_rational().cloned().or_else(|| l.as_i64().map(Rational::from_int));
                let r_rat = r.as_rational().cloned().or_else(|| r.as_i64().map(Rational::from_int));

                match (l_rat, r_rat) {
                    (Some(lv), Some(rv)) => {
                        let result = match op {
                            BinaryOp::Add => lv + rv,
                            BinaryOp::Sub => lv - rv,
                            BinaryOp::Mul => lv * rv,
                            BinaryOp::Div => {
                                if rv == Rational::zero() {
                                    return Err(CodlError::DivisionByZero);
                                }
                                lv / rv
                            }
                        };
                        Ok(CodlValue::Rational(result))
                    }
                    _ => Err(CodlError::TypeError {
                        expected: "Rational operands".to_string(),
                        actual: format!("{:?} {:?} {:?}", l, op, r),
                    }),
                }
            }
            CodlExpr::Comparison { left, op, right } => {
                let l = self.evaluate_expr(left)?;
                let r = self.evaluate_expr(right)?;

                let l_rat = l.as_rational().cloned().or_else(|| l.as_i64().map(Rational::from_int));
                let r_rat = r.as_rational().cloned().or_else(|| r.as_i64().map(Rational::from_int));

                match (l_rat, r_rat) {
                    (Some(lv), Some(rv)) => {
                        let result = match op {
                            ComparisonOp::Eq => lv == rv,
                            ComparisonOp::Ne => lv != rv,
                            ComparisonOp::Lt => lv < rv,
                            ComparisonOp::Le => lv <= rv,
                            ComparisonOp::Gt => lv > rv,
                            ComparisonOp::Ge => lv >= rv,
                        };
                        Ok(CodlValue::Bool(result))
                    }
                    _ => Err(CodlError::TypeError {
                        expected: "Rational operands for comparison".to_string(),
                        actual: format!("{:?} {:?} {:?}", l, op, r),
                    }),
                }
            }
            CodlExpr::IsNull(name) => {
                let val = self.scope.get(name);
                Ok(CodlValue::Bool(val.map(|v| v.is_null()).unwrap_or(true)))
            }
            CodlExpr::IsNotNull(name) => {
                let val = self.scope.get(name);
                Ok(CodlValue::Bool(val.map(|v| !v.is_null()).unwrap_or(false)))
            }
        }
    }

    /// Evaluate a condition expression to boolean.
    fn evaluate_condition(&self, expr: &CodlExpr) -> CodlResult<bool> {
        let value = self.evaluate_expr(expr)?;
        match value {
            CodlValue::Bool(b) => Ok(b),
            _ => Err(CodlError::TypeError {
                expected: "Bool".to_string(),
                actual: format!("{:?}", value),
            }),
        }
    }

    /// Convert CODL component to vsc_core VectorComponent.
    fn codl_to_core_component(&self, comp: CodlComponent) -> VectorComponent {
        match comp {
            CodlComponent::X => VectorComponent::X,
            CodlComponent::Y => VectorComponent::Y,
            CodlComponent::Z => VectorComponent::Z,
            CodlComponent::T => VectorComponent::T,
        }
    }

    /// Convert CODL relation to vsc_core RelationType.
    fn codl_to_core_relation(&self, rel: CodlRelation) -> RelationType {
        match rel {
            CodlRelation::Eq => RelationType::Eq,
            CodlRelation::Lt => RelationType::Lt,
            CodlRelation::Le => RelationType::Le,
            CodlRelation::Gt => RelationType::Gt,
            CodlRelation::Ge => RelationType::Ge,
        }
    }

    /// Convert CODL priority to vsc_core ConstraintPriority.
    fn codl_to_core_priority(&self, priority: CodlPriority) -> ConstraintPriority {
        match priority {
            CodlPriority::Hard => ConstraintPriority::Hard,
            CodlPriority::Soft => ConstraintPriority::Soft,
        }
    }
}

impl Default for CodlInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

/// Execute a CODL command with JSON arguments.
///
/// Convenience function that creates an interpreter and runs the command.
pub fn execute_codl(
    cmd: &CodlCommand,
    args: &JsonValue,
) -> CodlResult<Vec<Constraint>> {
    let mut interpreter = CodlInterpreter::new();
    interpreter.execute(cmd, args)
}

/// Execute a CODL command with JSON arguments and custom start ID.
pub fn execute_codl_with_id(
    cmd: &CodlCommand,
    args: &JsonValue,
    start_id: u64,
) -> CodlResult<Vec<Constraint>> {
    let mut interpreter = CodlInterpreter::new().with_start_id(start_id);
    interpreter.execute(cmd, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_cmd(yaml: &str) -> CodlCommand {
        serde_yaml::from_str(yaml).expect("Failed to parse YAML")
    }

    #[test]
    fn test_simple_constraint() {
        let yaml = r#"
name: simple
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
        let cmd = parse_cmd(yaml);
        let args = json!({
            "target": 42,
            "value": "100/1"
        });

        let constraints = execute_codl(&cmd, &args).unwrap();
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0].target, EntityId(42));
        assert_eq!(constraints[0].component, VectorComponent::X);
        assert_eq!(constraints[0].relation, RelationType::Eq);
        match &constraints[0].term {
            ConstraintTerm::Const { value } => {
                assert_eq!(*value, Rational::from_int(100));
            }
            _ => panic!("Expected Const term"),
        }
    }

    #[test]
    fn test_foreach_with_where() {
        let yaml = r#"
name: stack_vertical
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
        let cmd = parse_cmd(yaml);
        let args = json!({
            "instances": [10, 20, 30],
            "gap": 16
        });

        let constraints = execute_codl(&cmd, &args).unwrap();

        // Should generate 2 constraints (i=1 and i=2, skipping i=0)
        assert_eq!(constraints.len(), 2);

        // First constraint: instances[1].y = instances[0].y + 16
        assert_eq!(constraints[0].target, EntityId(20));
        assert_eq!(constraints[0].component, VectorComponent::Y);
        match &constraints[0].term {
            ConstraintTerm::Linear {
                entity_id,
                component,
                offset,
                ..
            } => {
                assert_eq!(*entity_id, EntityId(10));
                assert_eq!(*component, VectorComponent::Y);
                assert_eq!(*offset, Rational::from_int(16));
            }
            _ => panic!("Expected Linear term"),
        }

        // Second constraint: instances[2].y = instances[1].y + 16
        assert_eq!(constraints[1].target, EntityId(30));
        match &constraints[1].term {
            ConstraintTerm::Linear { entity_id, .. } => {
                assert_eq!(*entity_id, EntityId(20));
            }
            _ => panic!("Expected Linear term"),
        }
    }

    #[test]
    fn test_default_parameter() {
        let yaml = r#"
name: with_default
parameters:
  - name: target
    type: EntityId
  - name: value
    type: Rational
    default: "42"
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${value}"
"#;
        let cmd = parse_cmd(yaml);
        // Don't provide "value", use default
        let args = json!({ "target": 1 });

        let constraints = execute_codl(&cmd, &args).unwrap();
        assert_eq!(constraints.len(), 1);
        match &constraints[0].term {
            ConstraintTerm::Const { value } => {
                assert_eq!(*value, Rational::from_int(42));
            }
            _ => panic!("Expected Const term"),
        }
    }

    #[test]
    fn test_origin_yield() {
        let yaml = r#"
name: set_origin
parameters:
  - name: target
    type: EntityId
  - name: x_val
    type: Rational
operations:
  - type: origin
    target: "${target}"
    component: x
    value: "${x_val}"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({
            "target": 99,
            "x_val": "50/1"
        });

        let constraints = execute_codl(&cmd, &args).unwrap();
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0].target, EntityId(99));
        assert_eq!(constraints[0].component, VectorComponent::X);
        assert_eq!(constraints[0].relation, RelationType::Eq);
    }

    #[test]
    fn test_conditional() {
        let yaml = r#"
name: conditional_origin
parameters:
  - name: target
    type: EntityId
  - name: origin_x
    type: Rational
operations:
  - if: "origin_x != null"
    then:
      - type: origin
        target: "${target}"
        component: x
        value: "${origin_x}"
"#;
        let cmd = parse_cmd(yaml);

        // With origin_x provided
        let args_with = json!({
            "target": 1,
            "origin_x": 100
        });
        let constraints = execute_codl(&cmd, &args_with).unwrap();
        assert_eq!(constraints.len(), 1);

        // Without origin_x (null)
        let args_without = json!({ "target": 1 });
        let constraints = execute_codl(&cmd, &args_without).unwrap();
        assert_eq!(constraints.len(), 0);
    }

    #[test]
    fn test_constraint_id_sequence() {
        let yaml = r#"
name: multi
parameters:
  - name: instances
    type: Array<EntityId>
operations:
  - foreach:
      item: e
      index: i
      in: instances
    yield:
      type: origin
      target: "${e}"
      component: x
      value: "0"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({ "instances": [1, 2, 3] });

        let constraints = execute_codl_with_id(&cmd, &args, 100).unwrap();
        assert_eq!(constraints.len(), 3);
        assert_eq!(constraints[0].id, 100);
        assert_eq!(constraints[1].id, 101);
        assert_eq!(constraints[2].id, 102);
    }

    #[test]
    fn test_priority_soft() {
        let yaml = r#"
name: soft_constraint
parameters:
  - name: target
    type: EntityId
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    priority: soft
    term:
      type: const
      value: "100"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({ "target": 1 });

        let constraints = execute_codl(&cmd, &args).unwrap();
        assert_eq!(constraints[0].priority, ConstraintPriority::Soft);
    }

    // =========================================================================
    // Phase 15.1: Mathematical Exactness Edge Case Tests
    // =========================================================================

    #[test]
    fn test_division_by_zero_returns_error_not_panic() {
        // Division by zero in runtime evaluation must return CodlError::DivisionByZero,
        // NOT panic or produce NaN/Infinity.
        let yaml = r#"
name: div_by_zero
parameters:
  - name: target
    type: EntityId
  - name: divisor
    type: Rational
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${100 / divisor}"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({
            "target": 1,
            "divisor": 0  // Division by zero
        });

        let result = execute_codl(&cmd, &args);
        assert!(result.is_err(), "Division by zero should return Err");

        match result.unwrap_err() {
            CodlError::DivisionByZero => {
                // Expected error type
            }
            other => panic!(
                "Expected CodlError::DivisionByZero, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_undefined_variable_returns_error_not_panic() {
        // Referencing undefined variable must return CodlError::UnknownVariable,
        // NOT panic or produce default value.
        let yaml = r#"
name: undefined_var
parameters:
  - name: target
    type: EntityId
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${nonexistent_variable}"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({ "target": 1 });

        let result = execute_codl(&cmd, &args);
        assert!(result.is_err(), "Undefined variable should return Err");

        match result.unwrap_err() {
            CodlError::UnknownVariable(var_name) => {
                assert_eq!(var_name, "nonexistent_variable");
            }
            other => panic!(
                "Expected CodlError::UnknownVariable, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_complex_rational_arithmetic_exact_no_f64_contamination() {
        // Verify that rational arithmetic produces EXACT results without
        // floating-point contamination.
        //
        // Pratt Parser correctly handles operator precedence:
        //   a * 2 + 16/3 is parsed as (a * 2) + (16/3)
        // This test verifies the arithmetic is exact with correct precedence.
        //
        // If a = 3, then: (3 * 2) + (16/3) = 6 + 16/3 = 18/3 + 16/3 = 34/3
        let yaml = r#"
name: complex_arithmetic
parameters:
  - name: target
    type: EntityId
  - name: a
    type: Rational
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${a * 2 + 16/3}"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({
            "target": 1,
            "a": 3
        });

        let constraints = execute_codl(&cmd, &args).unwrap();
        assert_eq!(constraints.len(), 1);

        match &constraints[0].term {
            ConstraintTerm::Const { value } => {
                // With correct precedence: (3 * 2) + (16/3) = 6 + 16/3 = 34/3
                let expected = Rational::new(34, 3);
                assert_eq!(
                    *value, expected,
                    "Complex arithmetic must produce exact Rational (34/3), not f64 approximation"
                );
            }
            _ => panic!("Expected Const term"),
        }
    }

    #[test]
    fn test_fraction_arithmetic_preserves_exactness() {
        // This test explicitly verifies that fractional results are exact.
        // 10 / 3 = 10/3 (not 3.333...)
        let yaml = r#"
name: fraction_result
parameters:
  - name: target
    type: EntityId
  - name: dividend
    type: Rational
  - name: divisor
    type: Rational
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "${dividend / divisor}"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({
            "target": 1,
            "dividend": 10,
            "divisor": 3
        });

        let constraints = execute_codl(&cmd, &args).unwrap();

        match &constraints[0].term {
            ConstraintTerm::Const { value } => {
                // 10/3 must be EXACT, not truncated to 3 or approximated
                let expected = Rational::new(10, 3);
                assert_eq!(
                    *value, expected,
                    "10/3 must be exact Rational(10,3), not truncated or approximated"
                );
            }
            _ => panic!("Expected Const term"),
        }
    }

    #[test]
    fn test_rational_literal_parsing_preserves_exactness() {
        // Literal "16/3" in YAML must parse to exact Rational(16, 3),
        // NOT a floating-point approximation.
        let yaml = r#"
name: literal_rational
parameters:
  - name: target
    type: EntityId
operations:
  - type: constraint
    target: "${target}"
    component: x
    relation: eq
    term:
      type: const
      value: "16/3"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({ "target": 1 });

        let constraints = execute_codl(&cmd, &args).unwrap();

        match &constraints[0].term {
            ConstraintTerm::Const { value } => {
                // Verify exact 16/3, not 5.333... approximation
                let expected = Rational::new(16, 3);
                assert_eq!(
                    *value, expected,
                    "Literal 16/3 must be exact Rational, not f64 approximation"
                );

                // Additional check: verify it's NOT equal to 5 (truncation)
                assert_ne!(*value, Rational::from_int(5));
            }
            _ => panic!("Expected Const term"),
        }
    }

    #[test]
    fn test_array_index_out_of_bounds_returns_error() {
        // Array access with invalid index must return IndexOutOfBounds error.
        let yaml = r#"
name: bad_index
parameters:
  - name: arr
    type: Array<EntityId>
operations:
  - type: constraint
    target: "${arr[10]}"
    component: x
    relation: eq
    term:
      type: const
      value: "0"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({ "arr": [1, 2, 3] });  // Only 3 elements, index 10 is OOB

        let result = execute_codl(&cmd, &args);
        assert!(result.is_err(), "Out-of-bounds index should return Err");

        match result.unwrap_err() {
            CodlError::IndexOutOfBounds { array, index, length } => {
                assert_eq!(array, "arr");
                assert_eq!(index, 10);
                assert_eq!(length, 3);
            }
            other => panic!(
                "Expected CodlError::IndexOutOfBounds, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn test_negative_array_index_returns_error() {
        // Negative index must return IndexOutOfBounds error.
        let yaml = r#"
name: negative_index
parameters:
  - name: arr
    type: Array<EntityId>
  - name: offset
    type: Rational
operations:
  - type: constraint
    target: "${arr[offset]}"
    component: x
    relation: eq
    term:
      type: const
      value: "0"
"#;
        let cmd = parse_cmd(yaml);
        let args = json!({
            "arr": [1, 2, 3],
            "offset": -1  // Negative index
        });

        let result = execute_codl(&cmd, &args);
        assert!(result.is_err(), "Negative index should return Err");

        match result.unwrap_err() {
            CodlError::IndexOutOfBounds { index, .. } => {
                assert_eq!(index, -1);
            }
            other => panic!(
                "Expected CodlError::IndexOutOfBounds, got {:?}",
                other
            ),
        }
    }
}
