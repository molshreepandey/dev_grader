//! `grade-local` — grade a student project sitting on disk, without Kafka or GitHub.
//!
//! Same engine, same sandbox, same assignment layout as the worker; only the fetch stage is
//! swapped for a directory copy. This is how you smoke-test an assignment (template + hidden
//! tests + `grader.json`) before shipping it to students.
//!
//! ```text
//! grade-local <assignment_id> <student_dir>
//! ```
//!
//! Prints the `GradeResult` as JSON. Exits non-zero when the submission did not earn a clean
//! pass, so it can be used directly as a CI assertion.

use std::path::{Path, PathBuf};

use grader_engine::{Engine, EngineError, FsAssignmentStore, RepoFetcher, SandboxProjectRunner};
use grader_types::{GradeStatus, StackConfig, Submission};

/// Stands in for the GitHub fetcher: "downloading" the repo is copying a local directory.
struct LocalDirFetcher;

impl RepoFetcher for LocalDirFetcher {
    fn fetch(
        &self,
        repo_url: &str,
        _git_ref: Option<&str>,
        dest: &Path,
    ) -> Result<(), EngineError> {
        let src = PathBuf::from(repo_url);
        if !src.is_dir() {
            return Err(EngineError::new(format!(
                "not a directory: {}",
                src.display()
            )));
        }
        copy_dir(&src, dest).map_err(|e| EngineError::new(e.to_string()))
    }
}

/// Recursive copy, skipping `.git` and never following symlinks — the same things the real
/// fetcher's extractor refuses to unpack.
fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let target = dest.join(&name);
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let (Some(assignment_id), Some(student_dir)) = (args.next(), args.next()) else {
        eprintln!("usage: grade-local <assignment_id> <student_dir>");
        std::process::exit(64);
    };

    let env = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.to_string());
    let assignments_root = PathBuf::from(env("ASSIGNMENTS_ROOT", "/opt/assignments"));
    let rootfs_base = env("ROOTFS_BASE", "/opt/sandbox_rootfs");
    let work_root = env("WORK_ROOT", "/tmp/grader");

    // The submission's `stack` is informational (the assignment's grader.json is authoritative),
    // so read it from there and keep the two consistent.
    let grader_json = assignments_root.join(&assignment_id).join("grader.json");
    let config: StackConfig = serde_json::from_str(&std::fs::read_to_string(&grader_json)?)?;

    let engine = Engine::new(
        FsAssignmentStore::new(&assignments_root),
        LocalDirFetcher,
        SandboxProjectRunner::new(&rootfs_base),
        work_root,
    );

    let result = engine.grade(&Submission {
        submission_id: format!("local-{assignment_id}"),
        assignment_id,
        stack: config.stack,
        repo_url: student_dir,
        git_ref: None,
    });

    println!("{}", serde_json::to_string_pretty(&result)?);

    let clean_pass = result.status == GradeStatus::Graded && result.failed == 0;
    std::process::exit(if clean_pass { 0 } else { 1 });
}
