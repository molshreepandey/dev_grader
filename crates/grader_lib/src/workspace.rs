use std::path::{Path, PathBuf};
use std::process::Stdio;

use task_types::producer::{GradeStage, GradeStatus};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::config::GraderConfig;
use crate::error::{GraderError, StageResult};
use crate::sandbox::prepare_rootfs;

/// A per-task directory tree on the work volume. Dropping it removes the whole
/// thing — including on a panic or an early `?` return, so a failed task never
/// leaves a half-built `node_modules` behind on the disk.
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    pub async fn create(config: &GraderConfig, run_id: &str) -> StageResult<Self> {
        let root = config.work_root.join(run_id);
        tokio::fs::create_dir_all(&root).await.map_err(|e| {
            GraderError::internal(
                GradeStage::Clone,
                format!("failed to create workspace {}: {}", root.display(), e),
            )
        })?;
        prepare_rootfs(&root).await.map_err(|e| {
            GraderError::internal(GradeStage::Clone, format!("failed to lay out rootfs: {}", e))
        })?;
        Ok(Workspace { root })
    }

    /// Host path of the merged project — this is what the sandbox sees as `/project`.
    pub fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    /// Host path of `/out`, where the runners write their JUnit XML.
    pub fn out(&self) -> PathBuf {
        self.root.join("out")
    }

    /// Scratch checkout of the hidden test repo. Deleted before anything runs,
    /// so student code can never read the tests from here.
    fn tests_checkout(&self) -> PathBuf {
        self.root.join("hidden-tests")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // The sandbox's bind mounts lived in its own mount namespace, which is
        // gone by now, so this only ever walks plain files on the work volume.
        match std::fs::remove_dir_all(&self.root) {
            Ok(_) => debug!("removed workspace {}", self.root.display()),
            Err(e) => warn!("failed to remove workspace {}: {}", self.root.display(), e),
        }
    }
}

fn validate_url(url: &str) -> StageResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(GraderError::new(
            GradeStage::Clone,
            GradeStatus::CloneFailed,
            format!("only http(s) git URLs are accepted, got: {url}"),
        ));
    }
    Ok(())
}

async fn git_clone(url: &str, dest: &Path, config: &GraderConfig) -> StageResult<()> {
    validate_url(url)?;

    let mut cmd = Command::new("git");
    cmd.args([
        "clone",
        "--depth",
        "1",
        "--single-branch",
        "--no-tags",
        url,
        &dest.to_string_lossy(),
    ]);
    // Without this git blocks forever on the credential prompt for a private repo.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_ASKPASS", "");
    cmd.env("GCM_INTERACTIVE", "never");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let child = cmd.spawn().map_err(|e| {
        GraderError::internal(GradeStage::Clone, format!("failed to run git: {}", e))
    })?;

    let output = match timeout(config.clone_timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(GraderError::internal(
                GradeStage::Clone,
                format!("git failed: {}", e),
            ));
        }
        Err(_) => {
            return Err(GraderError::new(
                GradeStage::Clone,
                GradeStatus::Timeout,
                format!(
                    "cloning {} took longer than {}s",
                    url,
                    config.clone_timeout.as_secs()
                ),
            ));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(GraderError::new(
            GradeStage::Clone,
            GradeStatus::CloneFailed,
            format!("git clone of {url} failed"),
        )
        .with_logs(Some(tail)));
    }
    Ok(())
}

/// Sum of every regular file under `dir`, following no symlinks.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// Delete the student's copy of the test tree, then lay the hidden test repo
/// down on top of the checkout.
///
/// The purge is what makes the hidden suite authoritative — a student who edits
/// or deletes `tests/` in their own repo gets our copy regardless. The overlay
/// is a plain file copy, not a git merge: the two repos share no history, so
/// there is nothing for git to conflict over.
fn merge_trees(project: &Path, tests: &Path, purge_paths: &[&str]) -> StageResult<()> {
    for relative in purge_paths {
        let target = project.join(relative);
        // symlink_metadata, not metadata: a student can plant `tests -> /etc`,
        // and following it would delete the host's files.
        let Ok(meta) = std::fs::symlink_metadata(&target) else {
            continue;
        };
        let removed = if meta.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        removed.map_err(|e| {
            GraderError::new(
                GradeStage::Merge,
                GradeStatus::MergeFailed,
                format!("failed to purge {}: {}", relative, e),
            )
        })?;
        debug!("purged student path {}", relative);
    }

    overlay_copy(tests, project)
}

/// Recursive copy of `src` over `dst`, creating what is missing and replacing
/// what collides. Symlinks in either tree are never followed or recreated:
/// following one in `dst` would let a student's repo redirect our writes
/// anywhere on the host filesystem.
fn overlay_copy(src: &Path, dst: &Path) -> StageResult<()> {
    let fail = |msg: String| GraderError::new(GradeStage::Merge, GradeStatus::MergeFailed, msg);

    let entries = std::fs::read_dir(src)
        .map_err(|e| fail(format!("cannot read {}: {}", src.display(), e)))?;

    for entry in entries {
        let entry = entry.map_err(|e| fail(format!("cannot read entry: {}", e)))?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);

        let meta = std::fs::symlink_metadata(&from)
            .map_err(|e| fail(format!("cannot stat {}: {}", from.display(), e)))?;
        if meta.is_symlink() {
            warn!("skipping symlink in test repo: {}", from.display());
            continue;
        }

        // Whatever is at `to` loses: a plain file where we need a directory, a
        // symlink where we need anything at all.
        if let Ok(existing) = std::fs::symlink_metadata(&to) {
            let clash = existing.is_symlink() || (existing.is_dir() != meta.is_dir());
            if clash {
                let removed = if existing.is_dir() && !existing.is_symlink() {
                    std::fs::remove_dir_all(&to)
                } else {
                    std::fs::remove_file(&to)
                };
                removed.map_err(|e| fail(format!("cannot replace {}: {}", to.display(), e)))?;
            }
        }

        if meta.is_dir() {
            std::fs::create_dir_all(&to)
                .map_err(|e| fail(format!("cannot create {}: {}", to.display(), e)))?;
            overlay_copy(&from, &to)?;
        } else {
            let _ = std::fs::remove_file(&to);
            std::fs::copy(&from, &to)
                .map_err(|e| fail(format!("cannot copy {}: {}", to.display(), e)))?;
        }
    }
    Ok(())
}

/// Clone both repos, strip their git metadata, and produce the single tree the
/// sandbox will build: student code + hidden tests.
pub async fn prepare_project(
    workspace: &Workspace,
    student_link: &str,
    test_link: &str,
    purge_paths: &'static [&'static str],
    config: &GraderConfig,
) -> StageResult<()> {
    let project = workspace.project();
    let tests = workspace.tests_checkout();

    // git refuses to clone into an existing non-empty directory, and
    // prepare_rootfs already made `project/`.
    let _ = tokio::fs::remove_dir(&project).await;

    git_clone(student_link, &project, config).await?;
    git_clone(test_link, &tests, config).await?;

    let max_bytes = config.max_repo_bytes;
    let (project_c, tests_c) = (project.clone(), tests.clone());
    let size = tokio::task::spawn_blocking(move || dir_size(&project_c) + dir_size(&tests_c))
        .await
        .map_err(|e| GraderError::internal(GradeStage::Clone, format!("size check: {}", e)))?;
    if size > max_bytes {
        return Err(GraderError::new(
            GradeStage::Clone,
            GradeStatus::CloneFailed,
            format!(
                "checkouts total {} MB, over the {} MB limit",
                size / 1024 / 1024,
                max_bytes / 1024 / 1024
            ),
        ));
    }

    // The build must not see a git repo: a `.git` dir invites hooks and lets a
    // test runner pick up history it has no business reading.
    let _ = tokio::fs::remove_dir_all(project.join(".git")).await;

    let (project_c, tests_c) = (project.clone(), tests.clone());
    tokio::task::spawn_blocking(move || merge_trees(&project_c, &tests_c, purge_paths))
        .await
        .map_err(|e| GraderError::internal(GradeStage::Merge, format!("merge panicked: {}", e)))??;

    // The hidden tests are now inside the project; the scratch checkout would
    // otherwise sit inside the sandbox root where the student's code could read it.
    tokio::fs::remove_dir_all(&tests).await.map_err(|e| {
        GraderError::internal(
            GradeStage::Merge,
            format!("failed to remove the hidden-test checkout: {}", e),
        )
    })?;

    Ok(())
}
