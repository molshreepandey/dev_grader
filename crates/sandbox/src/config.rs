use std::fs::File;
use std::path::PathBuf;

/// Resource limits for a sandboxed grading run. Defaults are sized for real build toolchains
/// (maven/bun/pip), which fork many processes and use far more memory than a single snippet.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub memory_mib: u64,
    /// CPU quota in cores (e.g. `2.0` = two cores). Written to `cpu.max`.
    pub cpu_cores: f64,
    pub pids_max: u64,
    /// Wall-clock budget; the run is killed via `cgroup.kill` past this.
    pub wall_time_ms: u64,
    /// `RLIMIT_CPU` seconds (SIGXCPU backstop against busy loops).
    pub cpu_time_s: u64,
    /// `RLIMIT_FSIZE` — largest file the run may create.
    pub fsize_bytes: u64,
    /// Size of the writable `/tmp` tmpfs.
    pub tmp_size_mib: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_mib: 2048,
            cpu_cores: 2.0,
            pids_max: 512,
            wall_time_ms: 120_000,
            cpu_time_s: 120,
            fsize_bytes: 256 * 1024 * 1024,
            tmp_size_mib: 512,
        }
    }
}

/// A single sandboxed run: install + test a merged project inside a per-stack rootfs.
#[derive(Debug)]
pub struct ProjectSandboxConfig {
    /// Stable id (submission id) used to name the cgroup.
    pub id: String,
    /// Empty, writable directory used to assemble the mount-namespace root.
    pub root_dir: PathBuf,
    /// Baked, read-only per-stack rootfs (toolchain + warmed dependency caches).
    pub rootfs: PathBuf,
    /// The merged project (student code + hidden tests); bind-mounted read-write at `/work`.
    pub work_dir: PathBuf,
    /// Command to run, already wrapped as a shell invocation (see [`build_shell_command`]).
    pub argv: Vec<String>,
    pub limits: Limits,
    /// Captured stdout of the run.
    pub stdout_file: File,
    /// Captured stderr of the run.
    pub stderr_file: File,
}

/// Outcome of a sandboxed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxOutcome {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub wall_time_ms: u128,
    pub is_oom: bool,
    pub timed_out: bool,
}

/// The working directory, inside the sandbox, where the merged project is mounted.
pub const WORK_MOUNT: &str = "/work";

/// Build the shell command that installs (if any) then runs the tests, from `/work`.
/// Steps are `&&`-chained so a failed install short-circuits before the test step.
pub fn build_shell_command(install: &[String], test: &[String]) -> Vec<String> {
    let mut steps = vec![format!("cd {}", shell_quote(WORK_MOUNT))];
    if !install.is_empty() {
        steps.push(join_argv(install));
    }
    steps.push(join_argv(test));
    vec!["/bin/sh".into(), "-c".into(), steps.join(" && ")]
}

fn join_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// POSIX single-quote a shell argument (wrap in `'...'`, escaping embedded single quotes).
pub fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b'=' | b':' | b','))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Render `cpu.max` ("quota period") for a cores value, using a 100 ms period.
pub fn cpu_max_line(cores: f64) -> String {
    const PERIOD_US: u64 = 100_000;
    let quota = (cores * PERIOD_US as f64).round() as u64;
    format!("{quota} {PERIOD_US}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_command_chains_install_then_test_from_work() {
        let cmd = build_shell_command(
            &["pip".into(), "install".into(), "-r".into(), "requirements.txt".into()],
            &["pytest".into(), "--junitxml=report.xml".into()],
        );
        assert_eq!(cmd[0], "/bin/sh");
        assert_eq!(cmd[1], "-c");
        assert_eq!(
            cmd[2],
            "cd /work && pip install -r requirements.txt && pytest --junitxml=report.xml"
        );
    }

    #[test]
    fn shell_command_without_install_is_just_the_test() {
        let cmd = build_shell_command(&[], &["mvn".into(), "-o".into(), "test".into()]);
        assert_eq!(cmd[2], "cd /work && mvn -o test");
    }

    #[test]
    fn shell_quote_wraps_only_when_needed() {
        assert_eq!(shell_quote("pytest"), "pytest");
        assert_eq!(shell_quote("--junitxml=report.xml"), "--junitxml=report.xml");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("rm -rf /; echo"), "'rm -rf /; echo'");
        // embedded single quote is escaped, preventing shell breakout
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn cpu_max_line_formats_cores() {
        assert_eq!(cpu_max_line(1.0), "100000 100000");
        assert_eq!(cpu_max_line(2.0), "200000 100000");
        assert_eq!(cpu_max_line(0.5), "50000 100000");
    }
}
