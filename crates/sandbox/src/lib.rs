//! Linux sandbox for whole-project grading — namespaces + cgroups + seccomp, no Docker.
//!
//! Forked from IronJudge (`lamicons-code-engine`) and adapted for build-tool workloads:
//! a per-stack read-only rootfs, a read-write `/work` project, raised cgroup limits, and an
//! extended (still default-deny) seccomp allowlist. See [`runner::run_project_sandbox`].
//!
//! The pure pieces — shell-command assembly, `cpu.max` formatting, and the seccomp program —
//! are unit-tested. The `unshare`/`pivot_root`/`exec` core requires root-equivalent namespace
//! privileges and a baked rootfs, so it is integration-tested on a provisioned host only.

mod cgroup;
mod config;
mod error;
mod runner;
mod seccomp;

pub use cgroup::{Cgroup, initialize_global_cgroups_once};
pub use config::{
    HOME_MOUNT, Limits, ProjectSandboxConfig, SandboxOutcome, WORK_MOUNT, build_shell_command,
    cpu_max_line, shell_quote,
};
pub use error::SandboxError;
pub use runner::run_project_sandbox;
pub use seccomp::build_project_seccomp_profile;
