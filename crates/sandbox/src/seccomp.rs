//! Seccomp profile for the **project** sandbox.
//!
//! Same shape as IronJudge — default-deny (`ENOSYS`) with an explicit allowlist — but the list
//! is extended, because a whole build toolchain (bun/pip/maven, plus a shell and the language
//! runtime) touches far more of the kernel than a single competitive-programming binary:
//! it creates and renames files, spawns many processes, opens sockets (localhost/IPC, and DNS
//! attempts even when offline), and watches directories.
//!
//! Deliberately still **denied** (never added): `ptrace`, `mount`/`umount2`, `pivot_root`,
//! `chroot`, `setns`, `unshare`, `bpf`, `keyctl`/`add_key`/`request_key`, `kexec_load`,
//! `init_module`/`finit_module`, `reboot`, `swapon`/`swapoff`, `settimeofday`/`clock_settime`/
//! `adjtimex`, `perf_event_open`, `setuid`/`setgid`/`setgroups`. Network isolation is enforced
//! by the network namespace, not by blocking `socket`.

use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};

/// Build the extended allowlist BPF program for the project sandbox.
pub fn build_project_seccomp_profile() -> Vec<libc::sock_filter> {
    let mut rules = std::collections::BTreeMap::new();
    for syscall in allowed_syscalls() {
        rules.insert(syscall, vec![]);
    }

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Errno(libc::ENOSYS as u32),
        SeccompAction::Allow,
        std::env::consts::ARCH.try_into().expect("unsupported arch"),
    )
    .expect("failed to build seccomp filter");

    let program: BpfProgram = filter.try_into().expect("failed to compile bpf");
    program
        .into_iter()
        .map(|i| libc::sock_filter {
            code: i.code,
            jt: i.jt,
            jf: i.jf,
            k: i.k,
        })
        .collect()
}

fn allowed_syscalls() -> Vec<libc::c_long> {
    let mut v = vec![
        // ---- memory ----
        libc::SYS_mmap, libc::SYS_mprotect, libc::SYS_munmap, libc::SYS_brk, libc::SYS_mremap,
        libc::SYS_madvise, libc::SYS_mincore, libc::SYS_membarrier, libc::SYS_mlock,
        libc::SYS_munlock, libc::SYS_memfd_create,
        // ---- basic io ----
        libc::SYS_read, libc::SYS_write, libc::SYS_readv, libc::SYS_writev, libc::SYS_pread64,
        libc::SYS_pwrite64, libc::SYS_lseek, libc::SYS_close, libc::SYS_ioctl, libc::SYS_fcntl,
        libc::SYS_dup, libc::SYS_dup2, libc::SYS_dup3, libc::SYS_pipe, libc::SYS_pipe2,
        libc::SYS_sendfile, libc::SYS_splice, libc::SYS_tee, libc::SYS_copy_file_range,
        // ---- file open / metadata ----
        libc::SYS_open, libc::SYS_openat, libc::SYS_stat, libc::SYS_fstat, libc::SYS_lstat,
        libc::SYS_newfstatat, libc::SYS_statx, libc::SYS_statfs, libc::SYS_fstatfs,
        libc::SYS_access, libc::SYS_faccessat, libc::SYS_faccessat2, libc::SYS_readlink,
        libc::SYS_readlinkat, libc::SYS_getcwd, libc::SYS_getdents, libc::SYS_getdents64,
        libc::SYS_getxattr, libc::SYS_lgetxattr, libc::SYS_fgetxattr, libc::SYS_listxattr,
        // ---- file mutation (build tools write, rename, chmod, symlink) ----
        libc::SYS_mkdir, libc::SYS_mkdirat, libc::SYS_rmdir, libc::SYS_unlink,
        libc::SYS_unlinkat, libc::SYS_rename, libc::SYS_renameat, libc::SYS_renameat2,
        libc::SYS_symlink, libc::SYS_symlinkat, libc::SYS_link, libc::SYS_linkat,
        libc::SYS_chmod, libc::SYS_fchmod, libc::SYS_fchmodat, libc::SYS_chown,
        libc::SYS_fchown, libc::SYS_lchown, libc::SYS_fchownat, libc::SYS_truncate,
        libc::SYS_ftruncate, libc::SYS_fallocate, libc::SYS_utimensat, libc::SYS_futimesat,
        libc::SYS_umask, libc::SYS_chdir, libc::SYS_fchdir, libc::SYS_fsync,
        libc::SYS_fdatasync, libc::SYS_flock, libc::SYS_fadvise64, libc::SYS_sync,
        // ---- process / thread lifecycle ----
        libc::SYS_clone, libc::SYS_clone3, libc::SYS_execve, libc::SYS_execveat,
        libc::SYS_wait4, libc::SYS_waitid, libc::SYS_exit, libc::SYS_exit_group,
        libc::SYS_kill, libc::SYS_tgkill, libc::SYS_tkill, libc::SYS_setpgid,
        libc::SYS_getpgid, libc::SYS_getpgrp, libc::SYS_setsid, libc::SYS_getsid,
        libc::SYS_set_tid_address, libc::SYS_set_robust_list, libc::SYS_get_robust_list,
        libc::SYS_prctl, libc::SYS_arch_prctl, libc::SYS_rseq, libc::SYS_futex,
        // ---- ids / limits / info ----
        libc::SYS_getpid, libc::SYS_getppid, libc::SYS_gettid, libc::SYS_getuid,
        libc::SYS_geteuid, libc::SYS_getgid, libc::SYS_getegid, libc::SYS_getgroups,
        libc::SYS_getresuid, libc::SYS_getresgid, libc::SYS_prlimit64, libc::SYS_getrlimit,
        libc::SYS_getrusage, libc::SYS_times, libc::SYS_uname, libc::SYS_sysinfo,
        libc::SYS_getpriority, libc::SYS_setpriority, libc::SYS_capget,
        // ---- scheduling ----
        libc::SYS_sched_yield, libc::SYS_sched_getaffinity, libc::SYS_sched_setaffinity,
        libc::SYS_sched_getparam, libc::SYS_sched_getscheduler,
        libc::SYS_sched_get_priority_max, libc::SYS_sched_get_priority_min,
        // ---- signals ----
        libc::SYS_rt_sigaction, libc::SYS_rt_sigprocmask, libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigpending, libc::SYS_rt_sigtimedwait, libc::SYS_rt_sigqueueinfo,
        libc::SYS_rt_sigsuspend, libc::SYS_sigaltstack, libc::SYS_signalfd4,
        // ---- time ----
        libc::SYS_gettimeofday, libc::SYS_clock_gettime, libc::SYS_clock_getres,
        libc::SYS_nanosleep, libc::SYS_clock_nanosleep, libc::SYS_getrandom,
        // ---- polling / events / inotify ----
        libc::SYS_epoll_create, libc::SYS_epoll_create1, libc::SYS_epoll_ctl,
        libc::SYS_epoll_wait, libc::SYS_epoll_pwait, libc::SYS_eventfd, libc::SYS_eventfd2,
        libc::SYS_poll, libc::SYS_ppoll, libc::SYS_select, libc::SYS_pselect6,
        libc::SYS_inotify_init, libc::SYS_inotify_init1, libc::SYS_inotify_add_watch,
        libc::SYS_inotify_rm_watch, libc::SYS_timerfd_create, libc::SYS_timerfd_settime,
        libc::SYS_timerfd_gettime,
        // ---- sockets (localhost / IPC; contained by the network namespace) ----
        libc::SYS_socket, libc::SYS_socketpair, libc::SYS_connect, libc::SYS_bind,
        libc::SYS_listen, libc::SYS_accept, libc::SYS_accept4, libc::SYS_getsockname,
        libc::SYS_getpeername, libc::SYS_setsockopt, libc::SYS_getsockopt, libc::SYS_sendto,
        libc::SYS_recvfrom, libc::SYS_sendmsg, libc::SYS_recvmsg, libc::SYS_sendmmsg,
        libc::SYS_recvmmsg, libc::SYS_shutdown,
    ];
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_compiles_to_nonempty_bpf() {
        let prog = build_project_seccomp_profile();
        assert!(!prog.is_empty());
    }

    #[test]
    fn allowlist_extends_beyond_a_minimal_runtime() {
        // The project profile must be materially larger than a tiny single-binary allowlist,
        // otherwise build tools would trip on ENOSYS.
        let list = allowed_syscalls();
        assert!(list.len() > 100, "expected a broad allowlist, got {}", list.len());
        assert!(list.contains(&libc::SYS_mkdirat));
        assert!(list.contains(&libc::SYS_connect));
        assert!(list.contains(&libc::SYS_execveat));
        // dangerous syscalls must NOT be present
        assert!(!list.contains(&libc::SYS_ptrace));
        assert!(!list.contains(&libc::SYS_mount));
        assert!(!list.contains(&libc::SYS_setns));
    }
}
