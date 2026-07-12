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

/// Which half of the run this is. The two differ in exactly two ways — network access and time
/// budget — and everything else about the sandbox is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Download dependencies. **Online**, generous budget. Runs our command over the project's
    /// manifest; a non-zero exit is the student's broken manifest, not their broken code.
    Install,
    /// Run the hidden tests. **Offline**, tight budget. This is the phase that executes untrusted
    /// student code.
    Test,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Install => "install",
            Phase::Test => "test",
        }
    }

    /// Only the install phase may reach the network.
    pub fn network(self) -> bool {
        self == Phase::Install
    }
}

/// Outcome of one sandboxed phase.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub exit_code: i32,
    pub timed_out: bool,
    pub is_oom: bool,
    /// Tail of stderr — the student-facing diagnostic when a phase fails.
    pub stderr_tail: String,
}

impl RunOutcome {
    pub fn failed(&self) -> bool {
        self.exit_code != 0
    }
}

/// Run one phase of the stack's recipe over `work_dir`, with `home_dir` as a writable `$HOME`
/// shared across phases (dependency caches live there).
pub trait ProjectRunner {
    fn run(
        &self,
        submission_id: &str,
        phase: Phase,
        work_dir: &Path,
        home_dir: &Path,
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
