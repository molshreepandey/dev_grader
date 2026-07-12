//! cgroup v2 setup for the project sandbox. Same delegation dance as IronJudge (move the
//! executor into an `init` cgroup, enable `+memory +cpu +pids` on the root subtree), but with
//! headroom-sized limits for real build toolchains. A [`Cgroup`] guard kills and removes its
//! cgroup on drop.

use std::sync::Once;

use tracing::{error, info};

use crate::config::{Limits, cpu_max_line};

static CGROUP_INIT: Once = Once::new();
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Mount cgroup2 (if needed), park the executor in an `init` cgroup, and delegate controllers
/// to child cgroups. Idempotent.
pub fn initialize_global_cgroups_once() {
    CGROUP_INIT.call_once(|| {
        // Ensure cgroup2 is mounted (normally already is; ignore failure in containers).
        unsafe {
            let fs = std::ffi::CString::new("cgroup2").unwrap();
            let target = std::ffi::CString::new(CGROUP_ROOT).unwrap();
            if libc::mount(fs.as_ptr(), target.as_ptr(), fs.as_ptr(), 0, std::ptr::null()) != 0 {
                info!("[cgroup] cgroup2 already mounted or mount not permitted (normal)");
            }
        }

        // cgroup2 forbids processes in a cgroup that delegates controllers to children, so move
        // ourselves into an `init` leaf first.
        let init = format!("{CGROUP_ROOT}/init");
        let _ = std::fs::create_dir_all(&init);
        let pid = std::process::id().to_string();
        if let Err(e) = std::fs::write(format!("{init}/cgroup.procs"), &pid) {
            error!("[cgroup] failed to move executor into init cgroup: {e}");
        }
        match std::fs::write(format!("{CGROUP_ROOT}/cgroup.subtree_control"), "+memory +cpu +pids") {
            Ok(_) => info!("[cgroup] subtree_control delegated"),
            Err(e) => error!("[cgroup] subtree_control delegation failed: {e}"),
        }
    });
}

/// A per-run cgroup that applies [`Limits`] and cleans itself up on drop.
pub struct Cgroup {
    path: String,
}

impl Cgroup {
    /// Create `/sys/fs/cgroup/proj_<id>` and write the limits.
    pub fn create(id: &str, limits: &Limits) -> std::io::Result<Self> {
        initialize_global_cgroups_once();
        let path = format!("{CGROUP_ROOT}/proj_{id}");
        std::fs::create_dir_all(&path)?;

        let mem_bytes = limits.memory_mib * 1024 * 1024;
        std::fs::write(format!("{path}/memory.max"), mem_bytes.to_string())?;
        let _ = std::fs::write(format!("{path}/memory.swap.max"), "0");
        std::fs::write(format!("{path}/cpu.max"), cpu_max_line(limits.cpu_cores))?;
        std::fs::write(format!("{path}/pids.max"), limits.pids_max.to_string())?;

        Ok(Cgroup { path })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Absolute path of this cgroup's `cgroup.procs`, for the child to join itself.
    pub fn procs_file(&self) -> String {
        format!("{}/cgroup.procs", self.path)
    }

    /// Kill every process in the cgroup (used on timeout).
    pub fn kill(&self) {
        if let Err(e) = std::fs::write(format!("{}/cgroup.kill", self.path), "1") {
            error!("[cgroup] failed to write cgroup.kill: {e}");
        }
    }

    /// Whether the kernel OOM-killed anything in this cgroup.
    pub fn was_oom_killed(&self) -> bool {
        let events = std::fs::read_to_string(format!("{}/memory.events", self.path)).unwrap_or_default();
        events.lines().any(|line| {
            let mut it = line.split_whitespace();
            matches!((it.next(), it.next()), (Some("oom_kill"), Some(n)) if n.parse::<u32>().unwrap_or(0) > 0)
        })
    }
}

impl Drop for Cgroup {
    fn drop(&mut self) {
        self.kill();
        // Wait for the cgroup to drain before removing it.
        for _ in 0..200 {
            match std::fs::read_to_string(format!("{}/cgroup.events", self.path)) {
                Ok(ev) if ev.contains("populated 0") => break,
                _ => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        if let Err(e) = std::fs::remove_dir(&self.path) {
            error!("[cgroup] failed to remove {}: {e}", self.path);
        }
    }
}
