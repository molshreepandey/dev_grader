use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use task_types::producer::{TestCaseResult, TestOutcome};

use crate::error::{GraderError, StageResult};
use task_types::producer::{GradeStage, GradeStatus};

/// Longest assertion message we carry back to Kafka, per test case.
const MAX_MESSAGE_CHARS: usize = 1000;

/// Parse a JUnit XML report. Bun, pytest and Maven Surefire all emit this same
/// schema, which is the whole reason the three languages need one parser and
/// not three.
///
/// Counts come from the `<testcase>` elements themselves rather than the
/// `tests="…"`/`failures="…"` attributes on `<testsuite>`, because the runners
/// disagree about whether errors are also counted as failures.
pub fn parse_junit_xml(xml: &str) -> Result<Vec<TestCaseResult>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut cases: Vec<TestCaseResult> = Vec::new();
    let mut suite_stack: Vec<String> = Vec::new();
    let mut current: Option<TestCaseResult> = None;
    // Set while inside <failure>/<error>, so the element's text body can be
    // used when it carries no message="…" attribute (Surefire often does this).
    let mut capturing_message = false;

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("malformed JUnit XML: {}", e)),
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                open_element(&e, &mut suite_stack, &mut current, &mut capturing_message);
            }

            // A self-closing element never produces an `End`, so it is opened
            // and closed here. `<testcase …/>` with no children is a pass.
            Ok(Event::Empty(e)) => {
                let tag = e.local_name().as_ref().to_vec();
                open_element(&e, &mut suite_stack, &mut current, &mut capturing_message);
                capturing_message = false;
                match tag.as_slice() {
                    b"testcase" => {
                        if let Some(case) = current.take() {
                            cases.push(case);
                        }
                    }
                    b"testsuite" => {
                        suite_stack.pop();
                    }
                    _ => {}
                }
            }

            Ok(Event::Text(t)) if capturing_message => {
                let body = t.xml_content().unwrap_or_default();
                set_message(&mut current, &body, &mut capturing_message);
            }

            // Surefire puts the stack trace in a CDATA section.
            Ok(Event::CData(t)) if capturing_message => {
                let body = t.decode().unwrap_or_default();
                set_message(&mut current, &body, &mut capturing_message);
            }

            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"testcase" => {
                    capturing_message = false;
                    if let Some(case) = current.take() {
                        cases.push(case);
                    }
                }
                b"testsuite" => {
                    suite_stack.pop();
                }
                b"failure" | b"error" | b"skipped" => capturing_message = false,
                _ => {}
            },

            _ => {}
        }
    }

    Ok(cases)
}

fn set_message(current: &mut Option<TestCaseResult>, body: &str, capturing_message: &mut bool) {
    if let Some(case) = current.as_mut()
        && !body.trim().is_empty()
    {
        case.message = Some(truncate(body.trim()));
    }
    *capturing_message = false;
}

fn open_element(
    e: &quick_xml::events::BytesStart<'_>,
    suite_stack: &mut Vec<String>,
    current: &mut Option<TestCaseResult>,
    capturing_message: &mut bool,
) {
    let tag = e.local_name().as_ref().to_vec();
    match tag.as_slice() {
        b"testsuite" => suite_stack.push(attr(e, b"name").unwrap_or_default()),
        b"testcase" => {
            let name = attr(e, b"name").unwrap_or_else(|| "<unnamed>".to_string());
            let suite = attr(e, b"classname")
                .filter(|s| !s.is_empty())
                .or_else(|| suite_stack.last().cloned().filter(|s| !s.is_empty()));
            let duration_ms = attr(e, b"time")
                .and_then(|t| t.parse::<f64>().ok())
                .map(|secs| (secs * 1000.0).round() as u64)
                .unwrap_or(0);
            *current = Some(TestCaseResult {
                name,
                suite,
                outcome: TestOutcome::Passed,
                duration_ms,
                message: None,
            });
        }
        b"failure" | b"error" | b"skipped" => {
            if let Some(case) = current.as_mut() {
                case.outcome = match tag.as_slice() {
                    b"failure" => TestOutcome::Failed,
                    b"error" => TestOutcome::Errored,
                    _ => TestOutcome::Skipped,
                };
                case.message = attr(e, b"message")
                    .or_else(|| attr(e, b"type"))
                    .map(|m| truncate(&m));
                *capturing_message = case.message.is_none();
            }
        }
        _ => {}
    }
}

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == key)
        .and_then(|a| a.unescape_value().ok())
        .map(|v| v.into_owned())
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_MESSAGE_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_MESSAGE_CHARS).collect();
    format!("{head}…")
}

/// Read every JUnit report the runner produced and flatten them into one list.
/// Surefire writes one file per test class, bun and pytest write a single file.
pub async fn collect_reports(paths: &[std::path::PathBuf]) -> StageResult<Vec<TestCaseResult>> {
    let mut all = Vec::new();
    for path in paths {
        let xml = tokio::fs::read_to_string(path).await.map_err(|e| {
            GraderError::new(
                GradeStage::Report,
                GradeStatus::RunFailed,
                format!("failed to read JUnit report {}: {}", path.display(), e),
            )
        })?;
        let cases = parse_junit_xml(&xml).map_err(|e| {
            GraderError::new(
                GradeStage::Report,
                GradeStatus::RunFailed,
                format!("{} ({})", e, path.display()),
            )
        })?;
        all.extend(cases);
    }
    Ok(all)
}

/// Every `*.xml` directly inside `dir`, sorted for a stable result order.
pub async fn xml_files_in(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return files;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("xml") {
            files.push(path);
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_mixed_suite() {
        let xml = r#"
        <testsuites>
          <testsuite name="grade_utils">
            <testcase name="passes" classname="A" time="0.01"/>
            <testcase name="fails" classname="A" time="0.02">
              <failure message="expected 90 got 89">at line 4</failure>
            </testcase>
            <testcase name="errors" classname="A">
              <error type="TypeError"/>
            </testcase>
            <testcase name="skipped" classname="A">
              <skipped/>
            </testcase>
          </testsuite>
        </testsuites>"#;

        let cases = parse_junit_xml(xml).unwrap();
        assert_eq!(cases.len(), 4);
        assert_eq!(cases[0].outcome, TestOutcome::Passed);
        assert_eq!(cases[0].duration_ms, 10);
        assert_eq!(cases[1].outcome, TestOutcome::Failed);
        assert_eq!(cases[1].message.as_deref(), Some("expected 90 got 89"));
        assert_eq!(cases[2].outcome, TestOutcome::Errored);
        assert_eq!(cases[3].outcome, TestOutcome::Skipped);
    }

    #[test]
    fn falls_back_to_the_element_body_for_a_message() {
        let xml = r#"<testsuite name="s">
            <testcase name="t"><failure>AssertionError: nope</failure></testcase>
        </testsuite>"#;
        let cases = parse_junit_xml(xml).unwrap();
        assert_eq!(cases[0].message.as_deref(), Some("AssertionError: nope"));
        assert_eq!(cases[0].suite.as_deref(), Some("s"));
    }

    #[test]
    fn an_empty_report_yields_no_cases() {
        let cases = parse_junit_xml(r#"<testsuites/>"#).unwrap();
        assert!(cases.is_empty());
    }
}
