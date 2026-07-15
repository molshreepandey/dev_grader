use std::env;
use std::path::PathBuf;
use std::time::Duration;

/// Everything the grading pipeline needs to bound a single task.
///
/// Defaults are tuned for the container image in `app/grader/Dockerfile`:
/// the work root and the package caches both live on a disk-backed volume,
/// never on tmpfs — a `mvn`/`bun install` tree is far too big to spend RAM on.
#[derive(Debug, Clone)]
pub struct GraderConfig {
    /// Disk-backed parent of every per-task workspace.
    pub work_root: PathBuf,
    /// Shared package cache (m2 / bun / pip), bind-mounted into the sandbox.
    /// `None` gives every task a cold cache — correct but slow, especially Java.
    pub cache_root: Option<PathBuf>,
    pub cgroup_root: PathBuf,

    pub memory_limit_mb: u64,
    /// CPU cores; 2.0 means "two full cores worth of runtime".
    pub cpu_limit: f64,
    pub pids_limit: u64,

    pub clone_timeout: Duration,
    pub install_timeout: Duration,
    pub test_timeout: Duration,

    /// A clone bigger than this is rejected before anything is executed.
    pub max_repo_bytes: u64,
    /// How much of a runner's output travels back on the result topic.
    pub log_tail_bytes: usize,

    /// Namespaces + cgroups. Turn off only for local dev on a machine where
    /// unprivileged user namespaces or cgroup v2 delegation are unavailable —
    /// with this off, student code runs with the worker's own privileges.
    pub isolation: bool,
}

impl GraderConfig {
    pub fn from_env() -> Self {
        let get = |key: &str, default: &str| env::var(key).unwrap_or_else(|_| default.to_string());
        let num = |key: &str, default: u64| -> u64 {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };

        let cache_root = get("GRADER_CACHE_DIR", "/var/lib/grader/cache");
        let cache_root = if cache_root.is_empty() || cache_root == "off" {
            None
        } else {
            Some(PathBuf::from(cache_root))
        };

        GraderConfig {
            work_root: PathBuf::from(get("GRADER_WORK_DIR", "/var/lib/grader/work")),
            cache_root,
            cgroup_root: PathBuf::from(get("GRADER_CGROUP_ROOT", "/sys/fs/cgroup")),
            memory_limit_mb: num("GRADER_MEMORY_MB", 2048),
            cpu_limit: env::var("GRADER_CPU_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2.0),
            pids_limit: num("GRADER_PIDS_MAX", 512),
            clone_timeout: Duration::from_secs(num("GRADER_CLONE_TIMEOUT_SECS", 120)),
            install_timeout: Duration::from_secs(num("GRADER_INSTALL_TIMEOUT_SECS", 600)),
            test_timeout: Duration::from_secs(num("GRADER_TEST_TIMEOUT_SECS", 600)),
            max_repo_bytes: num("GRADER_MAX_REPO_MB", 512) * 1024 * 1024,
            log_tail_bytes: num("GRADER_LOG_TAIL_BYTES", 8192) as usize,
            isolation: get("GRADER_ISOLATION", "true") != "false",
        }
    }
}
