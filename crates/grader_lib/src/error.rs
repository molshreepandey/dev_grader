use task_types::producer::{GradeStage, GradeStatus};

/// A failure that ends the task but still produces a well-formed result on the
/// output topic — the caller always learns *which* stage broke and whether it
/// was the student's fault (`CloneFailed`, `InstallFailed`, …) or ours
/// (`InternalError`).
#[derive(Debug, Clone)]
pub struct GraderError {
    pub stage: GradeStage,
    pub status: GradeStatus,
    pub message: String,
    /// Tail of the runner output, when the stage produced any.
    pub logs: Option<String>,
}

pub type StageResult<T> = Result<T, GraderError>;

impl GraderError {
    pub fn new(stage: GradeStage, status: GradeStatus, message: impl Into<String>) -> Self {
        GraderError {
            stage,
            status,
            message: message.into(),
            logs: None,
        }
    }

    pub fn internal(stage: GradeStage, message: impl Into<String>) -> Self {
        Self::new(stage, GradeStatus::InternalError, message)
    }

    pub fn with_logs(mut self, logs: Option<String>) -> Self {
        self.logs = logs;
        self
    }
}

impl std::fmt::Display for GraderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}/{:?}] {}", self.stage, self.status, self.message)
    }
}
