use serde::{Deserialize, Serialize};

/// Outcome of a single test case, normalized across pytest / bun / surefire.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaseStatus {
    Passed,
    /// An assertion failed.
    Failed,
    /// The test threw / errored before an assertion (JUnit `<error>`).
    Errored,
    Skipped,
}

impl CaseStatus {
    /// Both failures and errors count against the student.
    pub fn is_failure(self) -> bool {
        matches!(self, CaseStatus::Failed | CaseStatus::Errored)
    }
}

/// One normalized test case.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TestCase {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classname: Option<String>,
    pub status: CaseStatus,
    /// Failure/error message when the case did not pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub time_secs: f64,
}

impl TestCase {
    /// Fully-qualified display name (`ClassName::test_name` when a classname is present).
    pub fn qualified_name(&self) -> String {
        match &self.classname {
            Some(c) if !c.is_empty() => format!("{c}::{}", self.name),
            _ => self.name.clone(),
        }
    }
}

/// A normalized test report. Counts are derived from `cases`, never trusted from the XML
/// summary attributes (which merged/tampered reports can get wrong).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TestReport {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub cases: Vec<TestCase>,
}

impl TestReport {
    /// Build a report from parsed cases, computing counts from the cases themselves.
    pub fn from_cases(cases: Vec<TestCase>) -> Self {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for c in &cases {
            match c.status {
                CaseStatus::Passed => passed += 1,
                CaseStatus::Failed | CaseStatus::Errored => failed += 1,
                CaseStatus::Skipped => skipped += 1,
            }
        }
        Self {
            total: cases.len() as u32,
            passed,
            failed,
            skipped,
            cases,
        }
    }

    /// Merge multiple reports (e.g. surefire writes one XML file per test class).
    pub fn merge(reports: impl IntoIterator<Item = TestReport>) -> Self {
        let mut cases = Vec::new();
        for r in reports {
            cases.extend(r.cases);
        }
        Self::from_cases(cases)
    }

    /// Iterator over failing/errored cases only.
    pub fn failures(&self) -> impl Iterator<Item = &TestCase> {
        self.cases.iter().filter(|c| c.status.is_failure())
    }

    /// True when every non-skipped case passed.
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }
}
