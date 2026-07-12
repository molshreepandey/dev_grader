#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to prepare sandbox: {0}")]
    Prepare(String),

    #[error("failed to configure cgroup: {0}")]
    Cgroup(#[from] std::io::Error),

    #[error("failed to spawn sandboxed process: {0}")]
    Spawn(String),

    #[error("error waiting on sandboxed process: {0}")]
    Wait(String),
}
