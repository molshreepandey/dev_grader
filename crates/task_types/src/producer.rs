use serde::{Deserialize, Serialize};

/// Where the pipeline stopped. Every task reports the stage it reached, so a
/// consumer can tell "the student's code is broken" apart from "our grader is
/// broken" without parsing the error string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeStage {
    Clone,
    Merge,
    Install,
    Test,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeStatus {
    /// Every discovered test passed.
    Passed,
    /// At least one passed and at least one failed.
    PartiallyPassed,
    /// The suite ran to completion and every test failed.
    Failed,
    /// A repository could not be cloned (bad link, private repo, network).
    CloneFailed,
    /// The two checkouts could not be combined (missing/odd project layout).
    MergeFailed,
    /// Dependency install or compilation failed — the suite never ran.
    InstallFailed,
    /// The runner itself blew up (crash, no JUnit report, no tests discovered).
    RunFailed,
    /// Wall-clock limit hit on install or test.
    Timeout,
    /// The cgroup OOM-killed the run.
    MemoryLimitExceeded,
    /// Something on our side broke (cgroup/namespace/disk).
    InternalError,
}

impl GradeStatus {
    /// True when the grade reflects the student's code rather than an
    /// infrastructure problem. Useful for retry decisions upstream.
    pub fn is_graded(&self) -> bool {
        matches!(
            self,
            GradeStatus::Passed | GradeStatus::PartiallyPassed | GradeStatus::Failed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Passed,
    Failed,
    Errored,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCaseResult {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    pub outcome: TestOutcome,
    pub duration_ms: u64,
    /// Assertion message / stack tail for a failed or errored test.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsePayload {
    pub task_id: String,
    pub student_id: String,
    pub status: GradeStatus,
    pub stage: GradeStage,
    pub total_testcases: usize,
    pub passed_testcases: usize,
    pub failed_testcases: usize,
    pub skipped_testcases: usize,
    /// passed / total * 100, rounded to two decimals. 0.0 when nothing ran.
    pub score: f64,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub testcases: Vec<TestCaseResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Tail of the runner's stdout/stderr — enough for a student to see why an
    /// install or compile step died, capped so a noisy build can't flood Kafka.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<String>,
}

impl ResponsePayload {
    fn base(task_id: String, student_id: String, status: GradeStatus, stage: GradeStage) -> Self {
        Self {
            task_id,
            student_id,
            status,
            stage,
            total_testcases: 0,
            passed_testcases: 0,
            failed_testcases: 0,
            skipped_testcases: 0,
            score: 0.0,
            duration_ms: 0,
            testcases: Vec::new(),
            error: None,
            logs: None,
        }
    }

    /// A completed suite. `status` is derived from the counts, so an all-pass,
    /// partial-pass and all-fail run all land here.
    pub fn graded(
        task_id: String,
        student_id: String,
        testcases: Vec<TestCaseResult>,
        duration_ms: u64,
    ) -> Self {
        let total = testcases.len();
        let passed = testcases
            .iter()
            .filter(|t| t.outcome == TestOutcome::Passed)
            .count();
        let skipped = testcases
            .iter()
            .filter(|t| t.outcome == TestOutcome::Skipped)
            .count();
        let failed = total - passed - skipped;

        // Skipped tests are excluded from the denominator: a student is not
        // penalised for a test the runner chose not to execute.
        let gradable = total - skipped;
        let score = if gradable == 0 {
            0.0
        } else {
            ((passed as f64 / gradable as f64) * 10000.0).round() / 100.0
        };

        let status = if gradable == 0 {
            GradeStatus::Failed
        } else if failed == 0 {
            GradeStatus::Passed
        } else if passed == 0 {
            GradeStatus::Failed
        } else {
            GradeStatus::PartiallyPassed
        };

        Self {
            total_testcases: total,
            passed_testcases: passed,
            failed_testcases: failed,
            skipped_testcases: skipped,
            score,
            duration_ms,
            testcases,
            ..Self::base(task_id, student_id, status, GradeStage::Report)
        }
    }

    pub fn failure(
        task_id: String,
        student_id: String,
        status: GradeStatus,
        stage: GradeStage,
        error: String,
        logs: Option<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            error: Some(error),
            logs,
            duration_ms,
            ..Self::base(task_id, student_id, status, stage)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(outcome: TestOutcome) -> TestCaseResult {
        TestCaseResult {
            name: "t".to_string(),
            suite: None,
            outcome,
            duration_ms: 1,
            message: None,
        }
    }

    fn grade(outcomes: &[TestOutcome]) -> ResponsePayload {
        ResponsePayload::graded(
            "task".to_string(),
            "student".to_string(),
            outcomes.iter().copied().map(case).collect(),
            0,
        )
    }

    #[test]
    fn all_passing_scores_one_hundred() {
        let r = grade(&[TestOutcome::Passed, TestOutcome::Passed]);
        assert_eq!(r.status, GradeStatus::Passed);
        assert_eq!(r.score, 100.0);
        assert_eq!(r.failed_testcases, 0);
    }

    #[test]
    fn a_mix_is_partially_passed() {
        let r = grade(&[
            TestOutcome::Passed,
            TestOutcome::Failed,
            TestOutcome::Errored,
        ]);
        assert_eq!(r.status, GradeStatus::PartiallyPassed);
        assert_eq!(r.passed_testcases, 1);
        assert_eq!(r.failed_testcases, 2);
        assert_eq!(r.score, 33.33);
    }

    #[test]
    fn all_failing_scores_zero() {
        let r = grade(&[TestOutcome::Failed, TestOutcome::Errored]);
        assert_eq!(r.status, GradeStatus::Failed);
        assert_eq!(r.score, 0.0);
    }

    #[test]
    fn skipped_tests_leave_the_denominator() {
        let r = grade(&[
            TestOutcome::Passed,
            TestOutcome::Failed,
            TestOutcome::Skipped,
        ]);
        assert_eq!(r.total_testcases, 3);
        assert_eq!(r.skipped_testcases, 1);
        assert_eq!(r.score, 50.0);
        assert_eq!(r.status, GradeStatus::PartiallyPassed);
    }

    #[test]
    fn a_suite_that_only_skipped_is_not_a_pass() {
        let r = grade(&[TestOutcome::Skipped]);
        assert_eq!(r.status, GradeStatus::Failed);
        assert_eq!(r.score, 0.0);
    }
}
