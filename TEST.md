      "affected_constraints": [
        {
          "constraint_id": 0,
          "current_value": "100/1",
          "suggested_value": "200/1",
          "delta": "100/1"
        }
      ]
    }
  ],
  "analysis": {
    "constraints_analyzed": 1,
    "analysis_time_us": 10,
    "hideable_in_viewport": false
  }
}

  left: 1
 right: 0
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test tests::test_override_soft_constraint_with_hard ... FAILED

failures:

failures:
    tests::test_override_soft_constraint_with_hard

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 27 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p vsc-cli --test integration_harness`
