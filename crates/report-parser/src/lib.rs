//! Parse JUnit XML into a normalized [`TestReport`].
//!
//! One parser serves every stack because pytest (`--junitxml`), Bun (`--reporter=junit`),
//! and Maven surefire all emit JUnit XML. The dialects differ slightly:
//!
//! * the root may be `<testsuites>` (pytest, bun) or a bare `<testsuite>` (surefire);
//! * a passing case may be self-closing (`<testcase .../>`) or an open element with only
//!   a `<system-out>` child;
//! * failure detail lives in a `<failure>`, `<error>`, or `<skipped>` child.
//!
//! We stream events with `quick-xml` and derive counts from the cases we actually see,
//! rather than trusting the summary attributes.

use grader_types::{CaseStatus, TestCase, TestReport};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("malformed xml: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("malformed xml attribute: {0}")]
    Attr(#[from] quick_xml::events::attributes::AttrError),
    #[error("no <testcase> elements found in report")]
    Empty,
}

/// Parse a single JUnit XML document into a [`TestReport`].
pub fn parse_junit(xml: &str) -> Result<TestReport, ParseError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut cases: Vec<TestCase> = Vec::new();
    // The testcase currently open (between its Start and End events).
    let mut open: Option<TestCase> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => match e.local_name().as_ref() {
                b"testcase" => open = Some(testcase_from(&e)?),
                b"failure" => mark(&mut open, CaseStatus::Failed, &e)?,
                b"error" => mark(&mut open, CaseStatus::Errored, &e)?,
                b"skipped" => mark(&mut open, CaseStatus::Skipped, &e)?,
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                // Self-closing testcase = passed, no child outcome.
                b"testcase" => cases.push(testcase_from(&e)?),
                b"failure" => mark(&mut open, CaseStatus::Failed, &e)?,
                b"error" => mark(&mut open, CaseStatus::Errored, &e)?,
                b"skipped" => mark(&mut open, CaseStatus::Skipped, &e)?,
                _ => {}
            },
            Event::End(e) if e.local_name().as_ref() == b"testcase" => {
                if let Some(tc) = open.take() {
                    cases.push(tc);
                }
            }
            _ => {}
        }
        buf.clear();
    }

    if cases.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(TestReport::from_cases(cases))
}

/// Parse and merge several JUnit XML documents (surefire emits one file per class).
pub fn parse_junit_many<'a>(
    docs: impl IntoIterator<Item = &'a str>,
) -> Result<TestReport, ParseError> {
    let mut cases = Vec::new();
    for doc in docs {
        cases.extend(parse_junit(doc)?.cases);
    }
    if cases.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(TestReport::from_cases(cases))
}

fn testcase_from(e: &BytesStart) -> Result<TestCase, ParseError> {
    let mut name = String::new();
    let mut classname = None;
    let mut time_secs = 0.0;
    for attr in e.attributes() {
        let attr = attr?;
        let val = String::from_utf8_lossy(&attr.value).into_owned();
        match attr.key.local_name().as_ref() {
            b"name" => name = val,
            b"classname" => classname = Some(val).filter(|c| !c.is_empty()),
            b"time" => time_secs = val.parse().unwrap_or(0.0),
            _ => {}
        }
    }
    Ok(TestCase {
        name,
        classname,
        status: CaseStatus::Passed,
        message: None,
        time_secs,
    })
}

/// Apply a non-passing outcome (and its `message` attribute) to the open testcase.
fn mark(open: &mut Option<TestCase>, status: CaseStatus, e: &BytesStart) -> Result<(), ParseError> {
    let Some(tc) = open.as_mut() else {
        return Ok(());
    };
    tc.status = status;
    for attr in e.attributes() {
        let attr = attr?;
        if attr.key.local_name().as_ref() == b"message" {
            tc.message = Some(String::from_utf8_lossy(&attr.value).into_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // pytest --junitxml: <testsuites> root, classname carries the module.
    const PYTEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<testsuites>
  <testsuite name="pytest" errors="0" failures="1" skipped="1" tests="4" time="0.03">
    <testcase classname="tests.test_solution" name="test_add" time="0.001"/>
    <testcase classname="tests.test_solution" name="test_sub" time="0.001"/>
    <testcase classname="tests.test_solution" name="test_mul" time="0.002">
      <failure message="assert 6 == 5">Solution.mul returned 6</failure>
    </testcase>
    <testcase classname="tests.test_solution" name="test_div" time="0.000">
      <skipped type="pytest.skip" message="not implemented"/>
    </testcase>
  </testsuite>
</testsuites>"#;

    // bun test --reporter=junit: <testsuites> root, file as classname.
    const BUN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="bun test" tests="3" failures="1">
  <testsuite name="solution.test.js">
    <testcase name="adds two numbers" classname="solution.test.js" time="0.0004"/>
    <testcase name="subtracts" classname="solution.test.js" time="0.0002"/>
    <testcase name="multiplies" classname="solution.test.js" time="0.0009">
      <failure message="expect(received).toBe(expected)">at solution.test.js:12</failure>
    </testcase>
  </testsuite>
</testsuites>"#;

    // maven surefire: bare <testsuite> root (no <testsuites> wrapper), one per class.
    const SUREFIRE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="SolutionTest" tests="2" failures="0" errors="1" skipped="0" time="0.12">
  <testcase name="testAdd" classname="SolutionTest" time="0.01"/>
  <testcase name="testThrows" classname="SolutionTest" time="0.02">
    <error message="java.lang.NullPointerException" type="java.lang.NullPointerException">stacktrace</error>
  </testcase>
</testsuite>"#;

    #[test]
    fn pytest_counts_and_statuses() {
        let r = parse_junit(PYTEST).unwrap();
        assert_eq!(r.total, 4);
        assert_eq!(r.passed, 2);
        assert_eq!(r.failed, 1);
        assert_eq!(r.skipped, 1);
    }

    #[test]
    fn pytest_captures_failure_name_and_message() {
        let r = parse_junit(PYTEST).unwrap();
        let failing: Vec<_> = r.failures().collect();
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].name, "test_mul");
        assert_eq!(failing[0].qualified_name(), "tests.test_solution::test_mul");
        assert_eq!(failing[0].message.as_deref(), Some("assert 6 == 5"));
    }

    #[test]
    fn bun_junit_parses() {
        let r = parse_junit(BUN).unwrap();
        assert_eq!(r.total, 3);
        assert_eq!(r.passed, 2);
        assert_eq!(r.failed, 1);
        assert!(!r.all_passed());
    }

    #[test]
    fn surefire_bare_testsuite_root_and_error_counts_as_failure() {
        let r = parse_junit(SUREFIRE).unwrap();
        assert_eq!(r.total, 2);
        assert_eq!(r.passed, 1);
        assert_eq!(r.failed, 1); // <error> counts against the student
        let f = r.failures().next().unwrap();
        assert_eq!(f.status, CaseStatus::Errored);
        assert_eq!(f.name, "testThrows");
    }

    #[test]
    fn merges_multiple_surefire_files() {
        let r = parse_junit_many([SUREFIRE, PYTEST]).unwrap();
        assert_eq!(r.total, 6);
        assert_eq!(r.passed, 3);
        assert_eq!(r.failed, 2);
        assert_eq!(r.skipped, 1);
    }

    #[test]
    fn all_passed_report() {
        let xml = r#"<testsuite name="x" tests="1"><testcase name="ok" time="0"/></testsuite>"#;
        let r = parse_junit(xml).unwrap();
        assert!(r.all_passed());
        assert_eq!(r.passed, 1);
    }

    #[test]
    fn empty_report_is_an_error() {
        let xml = r#"<testsuites></testsuites>"#;
        assert!(matches!(parse_junit(xml), Err(ParseError::Empty)));
    }
}
