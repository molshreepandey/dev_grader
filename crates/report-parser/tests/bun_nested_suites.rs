//! Bun nests a `<testsuite>` per `describe` block inside the file's suite, and writes a
//! self-closing `<failure type="AssertionError" />` with no message. Verbatim excerpt of what
//! `bun test --reporter=junit` (1.3) produced for the `mern-todo-api` assignment.

use grader_types::CaseStatus;
use report_parser::parse_junit;

const BUN_REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="bun test" tests="3" assertions="5" failures="1" skipped="0" time="0.032508622">
  <testsuite name="tests/todo.test.js" file="tests/todo.test.js" tests="3" assertions="5" failures="1" skipped="0" time="0" hostname="host">
    <testsuite name="GET /todos" file="tests/todo.test.js" line="26" tests="1" assertions="2" failures="0" skipped="0" time="0" hostname="host">
      <testcase name="starts empty" classname="GET /todos" time="0.002181" file="tests/todo.test.js" line="27" assertions="2" />
    </testsuite>
    <testsuite name="POST /todos" file="tests/todo.test.js" line="46" tests="2" assertions="3" failures="1" skipped="0" time="0" hostname="host">
      <testcase name="rejects a missing title with 400" classname="POST /todos" time="0.000517" file="tests/todo.test.js" line="52" assertions="1">
        <failure type="AssertionError" />
      </testcase>
      <testcase name="gives each todo a distinct id" classname="POST /todos" time="0.000248" file="tests/todo.test.js" line="62" assertions="1" />
    </testsuite>
  </testsuite>
</testsuites>"#;

#[test]
fn cases_nested_under_describe_suites_are_all_collected() {
    let report = parse_junit(BUN_REPORT).unwrap();

    assert_eq!(report.total, 3);
    assert_eq!(report.passed, 2);
    assert_eq!(report.failed, 1);

    let failing: Vec<String> = report.failures().map(|c| c.qualified_name()).collect();
    assert_eq!(
        failing,
        vec!["POST /todos::rejects a missing title with 400"]
    );
    assert!(report.cases.iter().all(|c| c.status != CaseStatus::Skipped));
}
