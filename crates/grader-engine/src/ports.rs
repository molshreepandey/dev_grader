//! The side-effecting stages of the pipeline, expressed as traits so the orchestration in
//! [`crate::pipeline`] can be unit-tested against fakes (no network, no root, no rootfs).

use std::path::{Path, PathBuf};

use grader_types::StackConfig;

/// A resolved assignment: where its template lives and how to grade it.
#[derive(Debug, Clone)]
pub struct Assignment {
    /// Directory holding the trusted template (hidden tests, locked config, stubs).
    pub template_dir: PathBuf,
    pub config: StackConfig,
}

/// Resolve an `assignment_id` to its template + grader config.
pub trait AssignmentStore {
    fn resolve(&self, assignment_id: &str) -> Result<Assignment, EngineError>;
}

/// Fetch a student's repository into `dest`.
pub trait RepoFetcher {
    fn fetch(&self, repo_url: &str, git_ref: Option<&str>, dest: &Path) -> Result<(), EngineError>;
}

/// Outcome of running install+test over a merged workspace.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub exit_code: i32,
    pub timed_out: bool,
    pub is_oom: bool,
    /// Tail of stderr, surfaced when no report was produced (build/install failure).
    pub stderr_tail: String,
}

/// Run the stack's install+test command over `work_dir`, leaving the JUnit report in `work_dir`.
pub trait ProjectRunner {
    fn run(
        &self,
        submission_id: &str,
        work_dir: &Path,
        config: &StackConfig,
    ) -> Result<RunOutcome, EngineError>;
}

/// A pipeline-stage failure. Carries a message; the pipeline maps it to a `GradeStatus`.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EngineError(pub String);

impl EngineError {
    pub fn new(msg: impl Into<String>) -> Self {
        EngineError(msg.into())
    }
}
