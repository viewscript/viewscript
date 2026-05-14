### 1. Test code
    fn test_override_soft_constraint_with_hard() {
        // Scenario: Add a text entity with Soft position constraints,
        // then add a Hard constraint that overrides the position.
        // The solver should accept this (shadowing).

        let project = TestProject::new();
        project.init().assert_success();

        // Add text entity at position (100, 50) - these are Soft constraints
        let add_result = project.run_vsc(&[
            "add-entity",
            "-t",
            "text",
            "-c",
            "Hello",
            "-x",
            "100",
            "-y",
            "50",
        ]);
        add_result.assert_success();
        let output = add_result.stdout_json();
        let corner_tl = output.get("corner_tl").and_then(|v| v.as_u64()).unwrap();

        // Now add a Hard constraint that overrides the position
        // TL.x = 200 (should shadow the Soft constraint TL.x = 100)
        let override_result = project.run_vsc(&[
            "add-constraint",
            &corner_tl.to_string(),
            "x",
            "eq",

### 2. find_conflicting_eq implementation
    fn find_conflicting_eq(&self, target_var: VarId) -> Option<(u64, Rational)> {
        for constraint in &self.active_queue {
            if constraint.relation == LinearRelation::Eq && constraint.terms.len() == 1 {
                let (var_id, coeff) = &constraint.terms[0];
                if *var_id == target_var && !coeff.is_zero() {
                    // var = -constant / coeff
                    let value = -constraint.constant.clone() / coeff.clone();
                    return Some((constraint.id, value));
                }
            }
        }
        None
    }

    /// Add a bilinear constraint to the suspended queue.
    pub fn add_bilinear(&mut self, constraint: BilinearConstraint) {
