use std::ffi::CString;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use nix::sched::{CloneFlags, unshare};
use task_types::producer::GradeStage;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{info, warn};

use crate::cgroups::{CgroupGuard, initialize_global_cgroups_once};
use crate::config::GraderConfig;
use crate::error::{GraderError, StageResult};

/// Host directories exposed read-only inside the sandbox. This is the whole
/// toolchain — bun, python, the JDK, maven, git — which is why the grader can
/// afford to be far less restrictive than the judge sandbox: it has to *build*
/// a real project, not just run a single translation unit.
const READONLY_DIRS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt"];

/// Files that Docker bind-mounts individually into /etc. A recursive bind of
/// /etc carries them along, but they arrive writable, so they get remounted
/// read-only one by one — otherwise student code could break DNS for every
/// task that follows it in this container.
const ETC_FILES: &[&str] = &["/etc/resolv.conf", "/etc/hosts", "/etc/hostname"];

const DEVICE_FILES: &[&str] = &["/dev/null", "/dev/zero", "/dev/urandom", "/dev/random"];

/// Largest single file the sandboxed build may create (RLIMIT_FSIZE).
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// Sandbox-visible paths. The per-task workspace on disk becomes `/`.
pub const PROJECT_DIR: &str = "/project";
pub const OUT_DIR: &str = "/out";
pub const CACHE_DIR: &str = "/cache";
pub const HOME_DIR: &str = "/home";
pub const TMP_DIR: &str = "/tmp";

#[derive(Debug)]
pub struct SandboxSpec {
    /// Unique per exec — names the cgroup.
    pub run_id: String,
    pub stage: GradeStage,
    /// Host path of the per-task workspace; becomes `/` inside the sandbox.
    pub root_dir: PathBuf,
    /// Host path of the shared package cache, mounted at `/cache`.
    pub cache_dir: Option<PathBuf>,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct SandboxOutcome {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub oom_killed: bool,
    pub wall_ms: u64,
    /// Tail of the merged stdout+stderr, capped at `log_tail_bytes`.
    pub logs: String,
}

impl SandboxOutcome {
    pub fn succeeded(&self) -> bool {
        !self.timed_out && !self.oom_killed && self.exit_code == 0 && self.signal.is_none()
    }
}

/// Build the directory skeleton that becomes the sandbox root. Everything here
/// is a real directory on the disk-backed work volume — no tmpfs — because an
/// `npm`/`m2`/`target` tree is far too large to hold in RAM.
pub async fn prepare_rootfs(root_dir: &Path) -> std::io::Result<()> {
    for dir in [
        "project", "out", "home", "tmp", "cache", "proc", "dev", "oldroot",
    ] {
        tokio::fs::create_dir_all(root_dir.join(dir)).await?;
    }
    // Mount points for the read-only toolchain binds.
    for dir in READONLY_DIRS {
        tokio::fs::create_dir_all(root_dir.join(dir.trim_start_matches('/'))).await?;
    }
    tokio::fs::create_dir_all(root_dir.join("dev/shm")).await?;
    let _ = std::os::unix::fs::symlink("/proc/self/fd", root_dir.join("dev/fd"));
    Ok(())
}

fn base_env(cache_mounted: bool) -> Vec<(String, String)> {
    let mut env = vec![
        (
            "PATH".to_string(),
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        ),
        ("HOME".to_string(), HOME_DIR.to_string()),
        ("TMPDIR".to_string(), TMP_DIR.to_string()),
        ("LANG".to_string(), "C.UTF-8".to_string()),
        // Package managers hang forever on a TTY prompt otherwise.
        ("CI".to_string(), "true".to_string()),
        ("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string()),
    ];
    if cache_mounted {
        env.push(("XDG_CACHE_HOME".to_string(), CACHE_DIR.to_string()));
    }
    env
}

/// Rewrite the sandbox-absolute paths in a command back to host paths. Only
/// used when `GRADER_ISOLATION=false`, so the same language specs work on a
/// dev box that cannot create user namespaces or cgroups.
fn to_host_path(value: &str, root_dir: &Path, cache_dir: Option<&Path>) -> String {
    let root = root_dir.to_string_lossy();
    let mut out = value.to_string();
    if let Some(cache) = cache_dir {
        out = out.replace(CACHE_DIR, &cache.to_string_lossy());
    }
    for dir in [PROJECT_DIR, OUT_DIR, HOME_DIR] {
        out = out.replace(dir, &format!("{}{}", root, dir));
    }
    out
}

pub async fn run_sandboxed(
    spec: SandboxSpec,
    config: &GraderConfig,
) -> StageResult<SandboxOutcome> {
    if config.isolation {
        run_isolated(spec, config).await
    } else {
        run_unisolated(spec, config).await
    }
}

async fn run_isolated(spec: SandboxSpec, config: &GraderConfig) -> StageResult<SandboxOutcome> {
    let started = Instant::now();
    initialize_global_cgroups_once(&config.cgroup_root);

    let cgroup = CgroupGuard::create(
        &config.cgroup_root,
        &format!("grader_{}", spec.run_id),
        spec.stage,
    )?;
    cgroup.apply_limits(
        config.memory_limit_mb,
        config.cpu_limit,
        config.pids_limit,
        spec.stage,
    )?;

    let log_path = spec.root_dir.join("out").join(format!(
        "{}.log",
        format!("{:?}", spec.stage).to_lowercase()
    ));
    let log_file = std::fs::File::create(&log_path).map_err(|e| {
        GraderError::internal(spec.stage, format!("failed to create stage log: {}", e))
    })?;
    let log_file_err = log_file.try_clone().map_err(|e| {
        GraderError::internal(spec.stage, format!("failed to dup stage log: {}", e))
    })?;

    // Everything the pre_exec closure touches has to be allocated up front:
    // after fork() only async-signal-safe calls are legal, so no String, no
    // PathBuf, no allocation of any kind past this point.
    let cstr = |s: &str| -> StageResult<CString> {
        CString::new(s).map_err(|e| GraderError::internal(spec.stage, format!("bad path: {}", e)))
    };

    let mut ro_mounts: Vec<(CString, CString)> = Vec::new();
    for dir in READONLY_DIRS {
        if Path::new(dir).exists() {
            let target = spec.root_dir.join(dir.trim_start_matches('/'));
            ro_mounts.push((cstr(dir)?, cstr(&target.to_string_lossy())?));
        }
    }
    let mut etc_files: Vec<CString> = Vec::new();
    for file in ETC_FILES {
        if Path::new(file).exists() {
            let target = spec.root_dir.join(file.trim_start_matches('/'));
            etc_files.push(cstr(&target.to_string_lossy())?);
        }
    }

    let mut dev_mounts: Vec<(CString, CString)> = Vec::new();
    for dev in DEVICE_FILES {
        if Path::new(dev).exists() {
            let target = spec.root_dir.join(dev.trim_start_matches('/'));
            let _ = std::fs::File::create(&target);
            dev_mounts.push((cstr(dev)?, cstr(&target.to_string_lossy())?));
        }
    }

    let cache_mount: Option<(CString, CString)> = match &spec.cache_dir {
        Some(cache) => {
            tokio::fs::create_dir_all(cache).await.map_err(|e| {
                GraderError::internal(spec.stage, format!("failed to create cache dir: {}", e))
            })?;
            Some((
                cstr(&cache.to_string_lossy())?,
                cstr(&spec.root_dir.join("cache").to_string_lossy())?,
            ))
        }
        None => None,
    };

    // Each stage pivots into the same workspace, and the previous stage's
    // pivot_root left `oldroot` detached-but-present (or absent, if it managed
    // to remove it). Recreate it: pivot_root fails with ENOENT otherwise.
    tokio::fs::create_dir_all(spec.root_dir.join("oldroot"))
        .await
        .map_err(|e| {
            GraderError::internal(spec.stage, format!("failed to create oldroot: {}", e))
        })?;

    let root_c = cstr(&spec.root_dir.to_string_lossy())?;
    let proc_c = cstr(&spec.root_dir.join("proc").to_string_lossy())?;
    let shm_c = cstr(&spec.root_dir.join("dev/shm").to_string_lossy())?;
    let cwd_c = cstr(PROJECT_DIR)?;
    let procs_c = cstr(&cgroup.procs_file().to_string_lossy())?;

    let uid_map = format!("0 {} 1\n", unsafe { libc::geteuid() }).into_bytes();
    let gid_map = format!("0 {} 1\n", unsafe { libc::getegid() }).into_bytes();
    let uid_map_c = cstr("/proc/self/uid_map")?;
    let gid_map_c = cstr("/proc/self/gid_map")?;
    let setgroups_c = cstr("/proc/self/setgroups")?;

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    cmd.env_clear();
    for (k, v) in base_env(spec.cache_dir.is_some())
        .into_iter()
        .chain(spec.env.iter().cloned())
    {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(log_file));
    cmd.stderr(Stdio::from(log_file_err));
    cmd.kill_on_drop(true);

    unsafe {
        cmd.pre_exec(move || {
            // Join the cgroup before unsharing: the limits must already be in
            // force when the kernel starts allocating for the new namespaces.
            let (pid_buf, pid_len) = itoa_pid();
            if !write_all(&procs_c, &pid_buf[..pid_len]) {
                libc::_exit(101);
            }

            // No CLONE_NEWNET on purpose. bun/pip/maven have to reach the
            // registry; the user namespace still denies CAP_NET_ADMIN over the
            // container's network, so the run can use the net but not reconfigure it.
            let flags = CloneFlags::CLONE_NEWNS
                | CloneFlags::CLONE_NEWPID
                | CloneFlags::CLONE_NEWIPC
                | CloneFlags::CLONE_NEWUTS
                | CloneFlags::CLONE_NEWUSER;
            if unshare(flags).is_err() {
                libc::_exit(102);
            }

            if !write_all(&uid_map_c, &uid_map) {
                libc::_exit(150);
            }
            if !write_all(&setgroups_c, b"deny\n") {
                libc::_exit(151);
            }
            if !write_all(&gid_map_c, &gid_map) {
                libc::_exit(152);
            }

            // CLONE_NEWPID only takes effect for children, so fork: the child is
            // PID 1 of the new namespace (and can therefore mount /proc), and
            // this process becomes its reaper.
            let child = libc::fork();
            if child < 0 {
                libc::_exit(103);
            }
            if child > 0 {
                for fd in 3..1024 {
                    libc::close(fd);
                }
                let mut status = 0;
                if libc::waitpid(child, &mut status, 0) < 0 {
                    libc::_exit(104);
                }
                if libc::WIFEXITED(status) {
                    libc::_exit(libc::WEXITSTATUS(status));
                }
                if libc::WIFSIGNALED(status) {
                    libc::_exit(128 + libc::WTERMSIG(status));
                }
                libc::_exit(1);
            }

            // If the worker dies, everything in here dies with it.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                libc::_exit(130);
            }

            // Detach the whole tree from the host's propagation, or our binds
            // would leak back out into the container's mount table.
            if libc::mount(
                c"none".as_ptr(),
                c"/".as_ptr(),
                std::ptr::null(),
                libc::MS_REC | libc::MS_PRIVATE,
                std::ptr::null(),
            ) != 0
            {
                libc::_exit(105);
            }
            // pivot_root needs its new root to be a mount point of its own.
            if libc::mount(
                root_c.as_ptr(),
                root_c.as_ptr(),
                c"bind".as_ptr(),
                libc::MS_BIND | libc::MS_REC,
                std::ptr::null(),
            ) != 0
            {
                libc::_exit(106);
            }

            for (src, target) in &ro_mounts {
                // Recursive: /etc in Docker has nested per-file binds hanging off it.
                if libc::mount(
                    src.as_ptr(),
                    target.as_ptr(),
                    c"bind".as_ptr(),
                    libc::MS_BIND | libc::MS_REC,
                    std::ptr::null(),
                ) != 0
                {
                    libc::_exit(107);
                }
                if libc::mount(
                    std::ptr::null(),
                    target.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND
                        | libc::MS_REMOUNT
                        | libc::MS_RDONLY
                        | libc::MS_NOSUID
                        | libc::MS_NODEV,
                    std::ptr::null(),
                ) != 0
                {
                    libc::_exit(108);
                }
            }

            // MS_REMOUNT|MS_RDONLY above only seals the top mount; the nested
            // /etc file binds need sealing individually.
            for file in &etc_files {
                libc::mount(
                    std::ptr::null(),
                    file.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
                    std::ptr::null(),
                );
            }

            for (src, target) in &dev_mounts {
                if libc::mount(
                    src.as_ptr(),
                    target.as_ptr(),
                    c"bind".as_ptr(),
                    libc::MS_BIND,
                    std::ptr::null(),
                ) != 0
                {
                    libc::_exit(109);
                }
            }

            if let Some((src, target)) = &cache_mount {
                // Read-write on purpose: a warm m2/bun cache is the difference
                // between a 30s and a 5min Java grade.
                if libc::mount(
                    src.as_ptr(),
                    target.as_ptr(),
                    c"bind".as_ptr(),
                    libc::MS_BIND | libc::MS_REC,
                    std::ptr::null(),
                ) != 0
                {
                    libc::_exit(110);
                }
            }

            if libc::mount(
                c"proc".as_ptr(),
                proc_c.as_ptr(),
                c"proc".as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                std::ptr::null(),
            ) != 0
            {
                libc::_exit(111);
            }
            // /dev/shm is the one tmpfs we keep — the JVM wants it, and it is
            // capped so it cannot eat the host's RAM.
            libc::mount(
                c"tmpfs".as_ptr(),
                shm_c.as_ptr(),
                c"tmpfs".as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV,
                c"size=64m,mode=1777".as_ptr() as *const libc::c_void,
            );

            if libc::chdir(root_c.as_ptr()) != 0 {
                libc::_exit(112);
            }
            if libc::syscall(libc::SYS_pivot_root, c".".as_ptr(), c"oldroot".as_ptr()) != 0 {
                libc::_exit(113);
            }
            if libc::chdir(c"/".as_ptr()) != 0 {
                libc::_exit(114);
            }
            // Detach the old root, or the sandbox keeps a live handle on the host fs.
            if libc::umount2(c"/oldroot".as_ptr(), libc::MNT_DETACH) != 0 {
                libc::_exit(115);
            }
            // Deliberately not rmdir'd: /oldroot is a real directory on the
            // workspace volume, shared by every stage of this task. Removing it
            // here would break the next stage's pivot_root.

            if libc::chdir(cwd_c.as_ptr()) != 0 {
                libc::_exit(116);
            }

            let no_core = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &no_core);
            let fsize = libc::rlimit {
                rlim_cur: MAX_FILE_BYTES,
                rlim_max: MAX_FILE_BYTES,
            };
            libc::setrlimit(libc::RLIMIT_FSIZE, &fsize);

            // No seccomp filter here, unlike the judge: a build legitimately
            // needs sockets, threads, subprocesses and mmap. NO_NEW_PRIVS still
            // blocks any setuid escalation from inside.
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                libc::_exit(117);
            }

            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|e| {
        GraderError::internal(spec.stage, format!("failed to spawn sandbox: {}", e))
    })?;

    let mut timed_out = false;
    let status = match timeout(spec.timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            return Err(GraderError::internal(
                spec.stage,
                format!("failed to wait on sandbox: {}", e),
            ));
        }
        Err(_) => {
            timed_out = true;
            // cgroup.kill takes out the whole process tree atomically; killing
            // just the child we spawned would leave orphaned build daemons.
            cgroup.kill_all();
            let _ = child.wait().await;
            std::process::ExitStatus::from_raw(9)
        }
    };

    let oom_killed = cgroup.was_oom_killed();
    let logs = read_log_tail(&log_path, config.log_tail_bytes).await;

    let outcome = SandboxOutcome {
        exit_code: status.code().unwrap_or(-1),
        signal: status.signal(),
        timed_out,
        oom_killed,
        wall_ms: started.elapsed().as_millis() as u64,
        logs,
    };
    info!(
        stage = ?spec.stage,
        exit_code = outcome.exit_code,
        timed_out = outcome.timed_out,
        oom = outcome.oom_killed,
        wall_ms = outcome.wall_ms,
        "sandbox stage finished"
    );
    // CgroupGuard drops here: kills stragglers, waits for the cgroup to drain, rmdirs.
    Ok(outcome)
}

/// Dev-only path: same commands, no namespaces and no cgroups. Sandbox-absolute
/// paths are rewritten to host paths so the language specs stay identical.
async fn run_unisolated(spec: SandboxSpec, config: &GraderConfig) -> StageResult<SandboxOutcome> {
    warn!("GRADER_ISOLATION=false — running student code unsandboxed (dev only)");
    let started = Instant::now();
    let cache = spec.cache_dir.as_deref();

    let log_path = spec.root_dir.join("out").join(format!(
        "{}.log",
        format!("{:?}", spec.stage).to_lowercase()
    ));
    let log_file = std::fs::File::create(&log_path).map_err(|e| {
        GraderError::internal(spec.stage, format!("failed to create stage log: {}", e))
    })?;
    let log_file_err = log_file.try_clone().map_err(|e| {
        GraderError::internal(spec.stage, format!("failed to dup stage log: {}", e))
    })?;

    let mut cmd = Command::new(to_host_path(&spec.program, &spec.root_dir, cache));
    for arg in &spec.args {
        cmd.arg(to_host_path(arg, &spec.root_dir, cache));
    }
    cmd.current_dir(spec.root_dir.join("project"));
    for (k, v) in base_env(spec.cache_dir.is_some())
        .into_iter()
        .chain(spec.env.iter().cloned())
    {
        cmd.env(k, to_host_path(&v, &spec.root_dir, cache));
    }
    // The container's fixed PATH is meaningless on a dev box — bun, mvn and
    // python live wherever the developer installed them.
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(log_file));
    cmd.stderr(Stdio::from(log_file_err));
    cmd.kill_on_drop(true);
    // Own process group, so a timeout can take out the build's children too.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| GraderError::internal(spec.stage, format!("failed to spawn: {}", e)))?;
    let pid = child.id().unwrap_or(0) as i32;

    let mut timed_out = false;
    let status = match timeout(spec.timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            return Err(GraderError::internal(
                spec.stage,
                format!("failed to wait: {}", e),
            ));
        }
        Err(_) => {
            timed_out = true;
            if pid > 0 {
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
            let _ = child.kill().await;
            std::process::ExitStatus::from_raw(9)
        }
    };

    Ok(SandboxOutcome {
        exit_code: status.code().unwrap_or(-1),
        signal: status.signal(),
        timed_out,
        oom_killed: false,
        wall_ms: started.elapsed().as_millis() as u64,
        logs: read_log_tail(&log_path, config.log_tail_bytes).await,
    })
}

async fn read_log_tail(path: &Path, tail_bytes: usize) -> String {
    let content = tokio::fs::read(path).await.unwrap_or_default();
    if content.len() <= tail_bytes {
        return String::from_utf8_lossy(&content).into_owned();
    }
    let start = content.len() - tail_bytes;
    format!(
        "...[{} bytes truncated]...\n{}",
        start,
        String::from_utf8_lossy(&content[start..])
    )
}

/// Async-signal-safe write: no allocation, no buffering, retries short writes.
fn write_all(path: &CString, data: &[u8]) -> bool {
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_WRONLY);
        if fd < 0 {
            return false;
        }
        let mut written = 0usize;
        while written < data.len() {
            let n = libc::write(
                fd,
                data[written..].as_ptr() as *const libc::c_void,
                data.len() - written,
            );
            if n <= 0 {
                if n < 0 && *libc::__errno_location() == libc::EINTR {
                    continue;
                }
                libc::close(fd);
                return false;
            }
            written += n as usize;
        }
        libc::close(fd);
        true
    }
}

/// Our own pid rendered into a stack buffer, returned as `(buf, len)`.
/// `format!` would allocate, and malloc between fork and exec can deadlock
/// against a lock another thread of the runtime was holding at fork time.
fn itoa_pid() -> ([u8; 24], usize) {
    let mut pid = unsafe { libc::getpid() };
    let mut digits = [0u8; 24];
    let mut len = 0;
    if pid <= 0 {
        digits[0] = b'0';
        len = 1;
    }
    let mut rev = [0u8; 24];
    let mut rev_len = 0;
    while pid > 0 {
        rev[rev_len] = b'0' + (pid % 10) as u8;
        pid /= 10;
        rev_len += 1;
    }
    while rev_len > 0 {
        rev_len -= 1;
        digits[len] = rev[rev_len];
        len += 1;
    }
    digits[len] = b'\n';
    (digits, len + 1)
}
