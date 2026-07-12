//! The namespaced runner. Ported from IronJudge's `sandbox.rs`, adapted for whole-project
//! grading: a single read-only per-stack rootfs is bind-mounted as the base, the merged project
//! is bind-mounted **read-write** at `/work` (the cwd), the tmpfs/limits are sized up, and the
//! extended seccomp profile is applied.
//!
//! Isolation, unchanged from IronJudge: a rootless user namespace plus PID/NET/IPC/UTS/mount
//! namespaces (so the run has **no network**), a pivot into the assembled rootfs, cgroup v2
//! limits, `RLIMIT_CPU`/`RLIMIT_FSIZE`, `NO_NEW_PRIVS`, and seccomp.
//!
//! # Runtime requirements (cannot be exercised without them)
//! Requires the ability to create user namespaces and a baked rootfs at `config.rootfs`
//! containing the toolchain and warmed dependency caches. The pure helpers are unit-tested in
//! [`crate::config`] / [`crate::seccomp`]; this function is integration-tested only on a host
//! with the rootfs present.

use std::ffi::CString;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nix::sched::{CloneFlags, unshare};

use crate::cgroup::Cgroup;
use crate::config::{ProjectSandboxConfig, SandboxOutcome};
use crate::error::SandboxError;
use crate::seccomp::build_project_seccomp_profile;

/// Read-only system directories bind-mounted from the rootfs (the toolchain + baked caches).
const RO_DIRS: &[&str] = &["/bin", "/sbin", "/lib", "/lib64", "/usr", "/etc", "/opt", "/home"];
/// Device files exposed inside the sandbox.
const DEV_FILES: &[&str] = &["/dev/null", "/dev/urandom", "/dev/zero"];
const PATH_ENV: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// HOME inside the sandbox; baked dependency caches (`~/.m2`, `~/.bun`) live under here.
const SANDBOX_HOME: &str = "/home/grader";

/// Run one install+test command inside the sandbox and return its outcome.
pub fn run_project_sandbox(config: ProjectSandboxConfig) -> Result<SandboxOutcome, SandboxError> {
    let start = Instant::now();
    let cgroup = Cgroup::create(&config.id, &config.limits)?;

    let root = &config.root_dir;
    // Assemble the mount-namespace root: writable scaffolding dirs + ro/rw bind targets.
    let scaffold = ["oldroot", "proc", "tmp", "dev/shm", "work"];
    for dir in scaffold {
        std::fs::create_dir_all(root.join(dir)).map_err(prep)?;
    }
    for dir in RO_DIRS {
        std::fs::create_dir_all(root.join(dir.trim_start_matches('/'))).map_err(prep)?;
    }
    // Bind targets for device files.
    for dev in DEV_FILES {
        let target = root.join(dev.trim_start_matches('/'));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(prep)?;
        }
        let _ = std::fs::File::create(&target);
    }

    // Precompute the C strings the pre_exec closure needs (no allocation in the child).
    let mut ro_mounts: Vec<(CString, CString)> = Vec::new();
    for dir in RO_DIRS {
        let src = config.rootfs.join(dir.trim_start_matches('/'));
        if src.exists() {
            ro_mounts.push((cstr(src.to_str().unwrap()), cstr(root.join(dir.trim_start_matches('/')).to_str().unwrap())));
        }
    }
    let mut dev_mounts: Vec<(CString, CString)> = Vec::new();
    for dev in DEV_FILES {
        if std::path::Path::new(dev).exists() {
            dev_mounts.push((cstr(dev), cstr(root.join(dev.trim_start_matches('/')).to_str().unwrap())));
        }
    }

    let root_c = cstr(root.to_str().unwrap());
    let work_src_c = cstr(config.work_dir.to_str().unwrap());
    let work_tgt_c = cstr(root.join("work").to_str().unwrap());
    let proc_tgt_c = cstr(root.join("proc").to_str().unwrap());
    let tmp_tgt_c = cstr(root.join("tmp").to_str().unwrap());
    let shm_tgt_c = cstr(root.join("dev/shm").to_str().unwrap());
    let procs_file_c = cstr(&cgroup.procs_file());

    let host_uid = unsafe { libc::geteuid() };
    let host_gid = unsafe { libc::getegid() };
    let uid_map = format!("0 {host_uid} 1\n").into_bytes();
    let gid_map = format!("0 {host_gid} 1\n").into_bytes();
    let uid_map_c = cstr("/proc/self/uid_map");
    let gid_map_c = cstr("/proc/self/gid_map");
    let setgroups_c = cstr("/proc/self/setgroups");

    let tmp_opts = cstr(&format!("size={}m,mode=1777", config.limits.tmp_size_mib));
    let shm_opts = cstr("size=64m,mode=1777");
    let cpu_time_s = config.limits.cpu_time_s;
    let fsize = config.limits.fsize_bytes;
    let bpf = build_project_seccomp_profile();

    let (exe, args) = config.argv.split_first().ok_or_else(|| prep_msg("empty argv"))?;
    let mut cmd = Command::new(exe);
    cmd.args(args);
    cmd.env_clear();
    cmd.env("PATH", PATH_ENV);
    cmd.env("HOME", SANDBOX_HOME);
    cmd.env("TMPDIR", "/tmp");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(config.stdout_file));
    cmd.stderr(Stdio::from(config.stderr_file));

    unsafe {
        cmd.pre_exec(move || {
            // Join the restricted cgroup, then create the namespaces.
            if !write_all(&procs_file_c, format!("{}\n", libc::getpid()).as_bytes()) {
                libc::_exit(101);
            }
            let flags = CloneFlags::CLONE_NEWPID
                | CloneFlags::CLONE_NEWIPC
                | CloneFlags::CLONE_NEWNET
                | CloneFlags::CLONE_NEWUTS
                | CloneFlags::CLONE_NEWNS
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

            // Fork so the exec'd process is PID 1's child inside the new PID namespace.
            let child = libc::fork();
            if child < 0 {
                libc::_exit(103);
            }
            if child > 0 {
                let mut status = 0;
                if libc::waitpid(child, &mut status, 0) < 0 {
                    libc::_exit(104);
                }
                if libc::WIFEXITED(status) {
                    libc::_exit(libc::WEXITSTATUS(status));
                } else if libc::WIFSIGNALED(status) {
                    libc::_exit(128 + libc::WTERMSIG(status));
                }
                libc::_exit(1);
            }
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                libc::_exit(130);
            }

            // Make all mounts private, then bind the assembled root onto itself.
            libc::mount(c"/".as_ptr(), c"/".as_ptr(), c"bind".as_ptr(), libc::MS_BIND | libc::MS_REC, std::ptr::null());
            if libc::mount(c"none".as_ptr(), c"/".as_ptr(), std::ptr::null(), libc::MS_REC | libc::MS_PRIVATE, std::ptr::null()) != 0 {
                libc::_exit(105);
            }
            if libc::mount(root_c.as_ptr(), root_c.as_ptr(), c"bind".as_ptr(), libc::MS_BIND | libc::MS_REC, std::ptr::null()) != 0 {
                libc::_exit(106);
            }

            // Read-only toolchain/cache dirs from the rootfs.
            for (src, tgt) in &ro_mounts {
                if libc::mount(src.as_ptr(), tgt.as_ptr(), c"bind".as_ptr(), libc::MS_BIND | libc::MS_REC, std::ptr::null()) != 0 {
                    libc::_exit(107);
                }
                let ro = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV;
                if libc::mount(std::ptr::null(), tgt.as_ptr(), std::ptr::null(), ro, std::ptr::null()) != 0 {
                    libc::_exit(108);
                }
            }

            // Writable project at /work (nosuid,nodev but read-write).
            if libc::mount(work_src_c.as_ptr(), work_tgt_c.as_ptr(), c"bind".as_ptr(), libc::MS_BIND | libc::MS_REC, std::ptr::null()) != 0 {
                libc::_exit(109);
            }
            let rw = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_NOSUID | libc::MS_NODEV;
            if libc::mount(std::ptr::null(), work_tgt_c.as_ptr(), std::ptr::null(), rw, std::ptr::null()) != 0 {
                libc::_exit(110);
            }

            for (src, tgt) in &dev_mounts {
                if libc::mount(src.as_ptr(), tgt.as_ptr(), c"bind".as_ptr(), libc::MS_BIND, std::ptr::null()) != 0 {
                    libc::_exit(111);
                }
            }

            let secure = libc::MS_NOEXEC | libc::MS_NOSUID | libc::MS_NODEV;
            if libc::mount(c"proc".as_ptr(), proc_tgt_c.as_ptr(), c"proc".as_ptr(), secure, std::ptr::null()) != 0 {
                libc::_exit(112);
            }
            if libc::mount(c"tmpfs".as_ptr(), tmp_tgt_c.as_ptr(), c"tmpfs".as_ptr(), libc::MS_NOSUID | libc::MS_NODEV, tmp_opts.as_ptr() as *const libc::c_void) != 0 {
                libc::_exit(113);
            }
            if libc::mount(c"tmpfs".as_ptr(), shm_tgt_c.as_ptr(), c"tmpfs".as_ptr(), secure, shm_opts.as_ptr() as *const libc::c_void) != 0 {
                libc::_exit(114);
            }

            // pivot into the assembled root and drop the old one.
            if libc::chdir(root_c.as_ptr()) != 0 {
                libc::_exit(115);
            }
            if libc::syscall(libc::SYS_pivot_root, c".".as_ptr(), c"oldroot".as_ptr()) != 0 {
                libc::_exit(116);
            }
            if libc::chdir(c"/".as_ptr()) != 0 {
                libc::_exit(117);
            }
            if libc::umount2(c"/oldroot".as_ptr(), libc::MNT_DETACH) != 0 {
                libc::_exit(118);
            }
            libc::rmdir(c"/oldroot".as_ptr());

            // cwd = the writable project.
            if libc::chdir(c"/work".as_ptr()) != 0 {
                libc::_exit(119);
            }

            // Resource backstops.
            set_rlimit(libc::RLIMIT_CPU, cpu_time_s, cpu_time_s + 1);
            set_rlimit(libc::RLIMIT_CORE, 0, 0);
            set_rlimit(libc::RLIMIT_FSIZE, fsize, fsize);

            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                libc::_exit(120);
            }
            let prog = libc::sock_fprog {
                len: bpf.len() as u16,
                filter: bpf.as_ptr() as *mut libc::sock_filter,
            };
            if libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &prog) != 0 {
                libc::_exit(121);
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|e| SandboxError::Spawn(e.to_string()))?;

    // Wall-clock timeout: poll, then kill the whole cgroup if we blow the deadline.
    let deadline = start + Duration::from_millis(config.limits.wall_time_ms);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().map_err(|e| SandboxError::Wait(e.to_string()))? {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    cgroup.kill();
                    let _ = child.kill();
                    break child.wait().map_err(|e| SandboxError::Wait(e.to_string()))?;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };

    Ok(SandboxOutcome {
        exit_code: status.code().unwrap_or(-1),
        signal: status.signal(),
        wall_time_ms: start.elapsed().as_millis(),
        is_oom: cgroup.was_oom_killed(),
        timed_out,
    })
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("path contained an interior NUL")
}

fn prep(e: std::io::Error) -> SandboxError {
    SandboxError::Prepare(e.to_string())
}

fn prep_msg(s: &str) -> SandboxError {
    SandboxError::Prepare(s.to_string())
}

/// `open(O_WRONLY)` + single `write`, for use in the post-fork child (no allocation).
fn write_all(path: &CString, data: &[u8]) -> bool {
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_WRONLY);
        if fd < 0 {
            return false;
        }
        let n = libc::write(fd, data.as_ptr() as *const libc::c_void, data.len());
        libc::close(fd);
        n == data.len() as isize
    }
}

fn set_rlimit(resource: libc::__rlimit_resource_t, soft: u64, hard: u64) {
    let rlim = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    unsafe {
        libc::setrlimit(resource, &rlim);
    }
}
