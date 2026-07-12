//! Production implementations of the pipeline ports.
//!
//! These are the real, side-effecting wirings used by the worker. The pipeline logic itself is
//! tested against fakes in [`crate::pipeline`]; the sandbox-backed runner additionally needs
//! root-equivalent namespace privileges and a baked rootfs at run time.

use std::path::{Path, PathBuf};

use grader_types::StackConfig;

use crate::ports::{Assignment, AssignmentStore, EngineError, ProjectRunner, RepoFetcher, RunOutcome};

/// Assignments laid out on disk as `<root>/<assignment_id>/{grader.json, template/}`.
pub struct FsAssignmentStore {
    root: PathBuf,
}

impl FsAssignmentStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsAssignmentStore { root: root.into() }
    }
}

impl AssignmentStore for FsAssignmentStore {
    fn resolve(&self, assignment_id: &str) -> Result<Assignment, EngineError> {
        // Guard against path traversal in the (trusted-but-be-safe) id.
        if assignment_id.is_empty() || assignment_id.contains('/') || assignment_id.contains("..") {
            return Err(EngineError::new(format!("invalid assignment id `{assignment_id}`")));
        }
        let dir = self.root.join(assignment_id);
        let config_path = dir.join("grader.json");
        let json = std::fs::read_to_string(&config_path)
            .map_err(|e| EngineError::new(format!("read {}: {e}", config_path.display())))?;
        let config: StackConfig =
            serde_json::from_str(&json).map_err(|e| EngineError::new(format!("parse grader.json: {e}")))?;
        let template_dir = dir.join("template");
        if !template_dir.is_dir() {
            return Err(EngineError::new(format!(
                "template dir missing: {}",
                template_dir.display()
            )));
        }
        Ok(Assignment { template_dir, config })
    }
}

/// Downloads student repos from GitHub via the [`fetcher`] crate.
pub struct HttpRepoFetcher {
    caps: fetcher::Caps,
}

impl HttpRepoFetcher {
    pub fn new() -> Self {
        HttpRepoFetcher {
            caps: fetcher::Caps::default(),
        }
    }
}

impl Default for HttpRepoFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoFetcher for HttpRepoFetcher {
    fn fetch(&self, repo_url: &str, git_ref: Option<&str>, dest: &Path) -> Result<(), EngineError> {
        fetcher::fetch_repo(repo_url, git_ref, dest, &self.caps)
            .map(|_| ())
            .map_err(|e| EngineError::new(e.to_string()))
    }
}

/// Runs install+test inside the Linux [`sandbox`]. Selects the per-stack rootfs as
/// `<rootfs_base>/<stack>` (e.g. `/opt/sandbox_rootfs/python`).
pub struct SandboxProjectRunner {
    rootfs_base: PathBuf,
    limits: sandbox::Limits,
}

impl SandboxProjectRunner {
    pub fn new(rootfs_base: impl Into<PathBuf>) -> Self {
        SandboxProjectRunner {
            rootfs_base: rootfs_base.into(),
            limits: sandbox::Limits::default(),
        }
    }

    pub fn with_limits(mut self, limits: sandbox::Limits) -> Self {
        self.limits = limits;
        self
    }
}

impl ProjectRunner for SandboxProjectRunner {
    fn run(
        &self,
        submission_id: &str,
        work_dir: &Path,
        config: &StackConfig,
    ) -> Result<RunOutcome, EngineError> {
        let argv = sandbox::build_shell_command(&config.install, &config.test);

        // The mount-namespace root is assembled in a sibling temp dir.
        let root_dir = tempfile::Builder::new()
            .prefix("sbroot")
            .tempdir_in(work_dir.parent().unwrap_or(work_dir))
            .map_err(|e| EngineError::new(e.to_string()))?;

        let stdout_path = root_dir.path().join("stdout.log");
        let stderr_path = root_dir.path().join("stderr.log");
        let stdout_file =
            std::fs::File::create(&stdout_path).map_err(|e| EngineError::new(e.to_string()))?;
        let stderr_file =
            std::fs::File::create(&stderr_path).map_err(|e| EngineError::new(e.to_string()))?;

        let cfg = sandbox::ProjectSandboxConfig {
            id: submission_id.to_string(),
            root_dir: root_dir.path().to_path_buf(),
            rootfs: self.rootfs_base.join(config.stack.as_str()),
            work_dir: work_dir.to_path_buf(),
            argv,
            limits: self.limits,
            stdout_file,
            stderr_file,
        };

        let outcome =
            sandbox::run_project_sandbox(cfg).map_err(|e| EngineError::new(e.to_string()))?;

        Ok(RunOutcome {
            exit_code: outcome.exit_code,
            timed_out: outcome.timed_out,
            is_oom: outcome.is_oom,
            stderr_tail: read_tail(&stderr_path, 4096),
        })
    }
}

/// Read up to the last `max` bytes of a (possibly large) log file, lossily as UTF-8.
fn read_tail(path: &Path, max: usize) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}
