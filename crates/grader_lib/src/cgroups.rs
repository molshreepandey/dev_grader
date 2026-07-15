use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::error::{GraderError, StageResult};
use task_types::producer::GradeStage;

static CGROUP_INIT: Once = Once::new();

/// Move the worker itself out of the root cgroup and delegate the controllers
/// we need to children. cgroup v2 refuses to enable a controller in
/// `subtree_control` while the cgroup still has processes in it, so the worker
/// has to step aside into `<root>/init` first. Runs once per process.
pub fn initialize_global_cgroups_once(cgroup_root: &Path) {
    CGROUP_INIT.call_once(|| {
        let init_cgroup = cgroup_root.join("init");
        if let Err(e) = std::fs::create_dir_all(&init_cgroup) {
            error!("[cgroup] failed to create {}: {}", init_cgroup.display(), e);
            return;
        }

        let my_pid = std::process::id();
        if let Err(e) = std::fs::write(init_cgroup.join("cgroup.procs"), my_pid.to_string()) {
            error!("[cgroup] failed to move worker into the init cgroup: {}", e);
        }

        match std::fs::write(
            cgroup_root.join("cgroup.subtree_control"),
            "+memory +cpu +pids",
        ) {
            Ok(_) => info!("[cgroup] subtree_control delegation ok"),
            Err(e) => error!(
                "[cgroup] subtree_control delegation failed: {} (limits will not apply)",
                e
            ),
        }
    });
}

/// Owns one cgroup directory for the lifetime of one sandboxed exec.
///
/// `Drop` does the teardown the kernel demands: kill everything still inside,
/// wait for the cgroup to actually drain, and only then `rmdir`. Removing a
/// populated cgroup fails with EBUSY, so skipping the wait leaks a directory
/// forever and eventually exhausts the cgroup tree.
pub struct CgroupGuard {
    pub path: PathBuf,
}

impl CgroupGuard {
    pub fn create(cgroup_root: &Path, name: &str, stage: GradeStage) -> StageResult<Self> {
        let path = cgroup_root.join(name);
        std::fs::create_dir_all(&path).map_err(|e| {
            GraderError::internal(
                stage,
                format!("failed to create cgroup {}: {}", path.display(), e),
            )
        })?;
        Ok(CgroupGuard { path })
    }

    pub fn apply_limits(
        &self,
        memory_limit_mb: u64,
        cpu_limit: f64,
        pids_limit: u64,
        stage: GradeStage,
    ) -> StageResult<()> {
        let write = |file: &str, value: String| -> StageResult<()> {
            std::fs::write(self.path.join(file), &value).map_err(|e| {
                GraderError::internal(
                    stage,
                    format!("failed to write cgroup {}={}: {}", file, value, e),
                )
            })
        };

        write("memory.max", (memory_limit_mb * 1024 * 1024).to_string())?;
        // Without this a memory-hungry run just swaps instead of being killed,
        // which turns a bounded failure into a machine-wide slowdown.
        let _ = std::fs::write(self.path.join("memory.swap.max"), "0");

        const PERIOD_US: f64 = 100_000.0;
        let quota = (cpu_limit * PERIOD_US).round() as u64;
        write("cpu.max", format!("{} {}", quota, PERIOD_US as u64))?;
        write("pids.max", pids_limit.to_string())?;
        Ok(())
    }

    pub fn procs_file(&self) -> PathBuf {
        self.path.join("cgroup.procs")
    }

    /// SIGKILL every process in the cgroup, atomically, including anything that
    /// forked away from the process we spawned.
    pub fn kill_all(&self) {
        if let Err(e) = std::fs::write(self.path.join("cgroup.kill"), "1") {
            warn!("[cgroup] kill {} failed: {}", self.path.display(), e);
        }
    }

    /// True if the kernel OOM-killed anything in this cgroup.
    pub fn was_oom_killed(&self) -> bool {
        let events = std::fs::read_to_string(self.path.join("memory.events")).unwrap_or_default();
        events.lines().any(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next();
            let count: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            matches!(key, Some("oom_kill") | Some("oom")) && count > 0
        })
    }
}

impl Drop for CgroupGuard {
    fn drop(&mut self) {
        self.kill_all();

        // cgroup.events flips `populated` to 0 once the last task is reaped.
        // Poll for it: rmdir before that point returns EBUSY.
        let events_file = self.path.join("cgroup.events");
        let mut retries = 250; // ~5s
        while retries > 0 {
            match std::fs::read_to_string(&events_file) {
                Ok(events) if events.contains("populated 0") => break,
                Err(_) => break, // already gone
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(20));
            retries -= 1;
        }
        if retries == 0 {
            warn!(
                "[cgroup] {} still populated after 5s; rmdir will likely fail",
                self.path.display()
            );
        }

        match std::fs::remove_dir(&self.path) {
            Ok(_) => debug!("[cgroup] removed {}", self.path.display()),
            Err(e) => error!("[cgroup] failed to remove {}: {}", self.path.display(), e),
        }
    }
}
