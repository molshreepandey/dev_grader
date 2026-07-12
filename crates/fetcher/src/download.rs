//! Thin IO layer: download a student's GitHub repo tarball and extract it.
//!
//! This is the only part of the crate that touches the network. Fetching is done by the
//! trusted worker (outside the sandbox), so ordinary HTTP is fine; the untrusted bytes are
//! contained by [`extract`](crate::extract)'s guards.

use std::path::Path;
use std::time::Duration;

use crate::error::FetchError;
use crate::extract::{Caps, ExtractStats, extract_tar_gz};
use crate::url::{RepoRef, parse_github_url, tarball_url};

#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub repo: RepoRef,
    pub stats: ExtractStats,
}

/// Parse `url`, download the tarball (optionally at `git_ref`), and extract it into `dest`.
pub fn fetch_repo(
    url: &str,
    git_ref: Option<&str>,
    dest: &Path,
    caps: &Caps,
) -> Result<FetchOutcome, FetchError> {
    let repo = parse_github_url(url)?;
    let tar_url = tarball_url(&repo, git_ref);

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build();
    let response = agent
        .get(&tar_url)
        .set("User-Agent", "dev_engine-grader")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| FetchError::Http(e.to_string()))?;

    let stats = extract_tar_gz(response.into_reader(), dest, caps)?;
    Ok(FetchOutcome { repo, stats })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    #[ignore = "hits the network; run with `cargo test -- --ignored`"]
    fn fetches_a_real_public_repo() {
        let dir = TempDir::new().unwrap();
        let outcome = fetch_repo(
            "https://github.com/octocat/Hello-World",
            None,
            dir.path(),
            &Caps::default(),
        )
        .unwrap();
        assert_eq!(outcome.repo.repo, "Hello-World");
        assert!(outcome.stats.files > 0);
        assert!(dir.path().join("README").exists());
    }
}
