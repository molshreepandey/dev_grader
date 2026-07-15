//! The three runners emit JUnit XML in three different dialects: bun nests
//! `<testsuite>` per `describe` block, Surefire puts the stack trace in CDATA
//! and counts errors separately from failures. These fixtures are real output —
//! the bun one was produced by grading a deliberately broken student repo.

use grader_lib::junit::parse_junit_xml;
use task_types::producer::{GradeStatus, ResponsePayload, TestOutcome};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture is readable")
}

fn grade(name: &str) -> ResponsePayload {
    let cases = parse_junit_xml(&fixture(name)).expect("fixture parses");
    ResponsePayload::graded("task".to_string(), "student".to_string(), cases, 0)
}

#[test]
fn bun_partial_run_is_scored_as_partially_passed() {
    let result = grade("bun-partial.xml");

    assert_eq!(result.total_testcases, 9);
    assert_eq!(result.passed_testcases, 8);
    assert_eq!(result.failed_testcases, 1);
    assert_eq!(result.status, GradeStatus::PartiallyPassed);
    assert_eq!(result.score, 88.89);

    let failed = result
        .testcases
        .iter()
        .find(|c| c.outcome == TestOutcome::Failed)
        .expect("one failing case");
    assert_eq!(failed.name, "handles negative and fractional values");
    // bun's self-closing <failure type="AssertionError"/> carries no message.
    assert_eq!(failed.message.as_deref(), Some("AssertionError"));
    // The nested <testsuite> for the describe block, not the file-level one.
    assert_eq!(failed.suite.as_deref(), Some("average — scale and edge cases"));
}

#[test]
fn surefire_failures_errors_and_skips_all_land() {
    let result = grade("surefire-TEST-GradeUtilsHiddenTest.xml");

    assert_eq!(result.total_testcases, 4);
    assert_eq!(result.passed_testcases, 1);
    // Surefire's <error> is a failed test as far as a grade is concerned.
    assert_eq!(result.failed_testcases, 2);
    assert_eq!(result.skipped_testcases, 1);
    // Skipped leaves the denominator: 1 of 3 executed tests passed.
    assert_eq!(result.score, 33.33);
    assert_eq!(result.status, GradeStatus::PartiallyPassed);

    let failure = &result.testcases[1];
    assert_eq!(failure.outcome, TestOutcome::Failed);
    assert!(
        failure
            .message
            .as_deref()
            .unwrap()
            .contains("expected: <A> but was: <B>"),
        "the message attribute should be unescaped: {:?}",
        failure.message
    );

    let error = &result.testcases[2];
    assert_eq!(error.outcome, TestOutcome::Errored);
    // No message attribute, so the CDATA body is used instead.
    assert!(
        error
            .message
            .as_deref()
            .unwrap()
            .contains("NullPointerException"),
        "the CDATA body should be the message: {:?}",
        error.message
    );
}
