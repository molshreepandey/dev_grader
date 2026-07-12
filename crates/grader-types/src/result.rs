use serde::{Deserialize, Serialize};

use crate::report::TestReport;

/// Terminal state of a grading run — the payload the worker produces back.
///
/// The variants track the pipeline's stages, so a student-facing message can say *which* step
/// let them down: fetch → merge → install → test.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GradeStatus {
    /// Tests ran to completion (some may have failed — that's still a successful grade).
    Graded,
    /// Submission could not be fetched (bad URL, private repo, network).
    FetchError,
    /// The submission does not have the shape the assignment requires, so no gradable workspace
    /// could be built (a required solution file is missing, a path is a symlink, …).
    MergeError,
    /// Dependency installation failed — the manifest is broken, a package does not exist, or the
    /// registry was unreachable. This is the one stage with network access.
    InstallError,
    /// Dependencies installed, but the tests never produced a report: the code did not compile,
    /// the suite could not be collected, or the run was OOM-killed.
    BuildError,
    /// The run exceeded its wall-clock/CPU budget (in either phase).
    Timeout,
    /// Sandbox/infrastructure failure — not the student's fault; safe to retry.
    InternalError,
}

/// The graded outcome returned for a submission.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GradeResult {
    pub submission_id: String,
    pub status: GradeStatus,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub total: u32,
    /// Qualified names of failing/errored tests, for student-facing feedback.
    pub failing_tests: Vec<String>,
    /// Populated for non-`Graded` statuses (build log, fetch error, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GradeResult {
    /// Build a successful grade from a normalized report.
    pub fn graded(submission_id: impl Into<String>, report: &TestReport) -> Self {
        Self {
            submission_id: submission_id.into(),
            status: GradeStatus::Graded,
            passed: report.passed,
            failed: report.failed,
            skipped: report.skipped,
            total: report.total,
            failing_tests: report.failures().map(|c| c.qualified_name()).collect(),
            error: None,
        }
    }

    /// Build a failed-before-testing result (fetch/build/timeout/internal).
    pub fn failed(
        submission_id: impl Into<String>,
        status: GradeStatus,
        error: impl Into<String>,
    ) -> Self {
        Self {
            submission_id: submission_id.into(),
            status,
            passed: 0,
            failed: 0,
            skipped: 0,
            total: 0,
            failing_tests: Vec::new(),
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{CaseStatus, TestCase, TestReport};

    fn case(name: &str, status: CaseStatus) -> TestCase {
        TestCase {
            name: name.into(),
            classname: Some("m".into()),
            status,
            message: None,
            time_secs: 0.0,
        }
    }

    #[test]
    fn graded_maps_counts_and_lists_only_failures() {
        let report = TestReport::from_cases(vec![
            case("a", CaseStatus::Passed),
            case("b", CaseStatus::Failed),
            case("c", CaseStatus::Errored),
            case("d", CaseStatus::Skipped),
        ]);
        let result = GradeResult::graded("sub-1", &report);

        assert_eq!(result.status, GradeStatus::Graded);
        assert_eq!(result.total, 4);
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 2);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.failing_tests, vec!["m::b", "m::c"]);
        assert!(result.error.is_none());
    }

    #[test]
    fn failed_result_carries_error_and_zero_counts() {
        let r = GradeResult::failed("sub-2", GradeStatus::FetchError, "404 not found");
        assert_eq!(r.status, GradeStatus::FetchError);
        assert_eq!(r.total, 0);
        assert_eq!(r.error.as_deref(), Some("404 not found"));
        assert!(r.failing_tests.is_empty());
    }
}
