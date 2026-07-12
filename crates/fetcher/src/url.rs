//! Parse a GitHub repository URL into `owner`/`repo` and build the tarball download URL.
//!
//! Only `github.com` is accepted. Supported input forms:
//! `https://github.com/owner/repo`, `.../repo.git`, trailing slash, extra path segments
//! (`/tree/main`), and the SSH form `git@github.com:owner/repo.git`.

use crate::error::FetchError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub repo: String,
}

/// Parse any accepted GitHub URL form into an owner/repo pair.
pub fn parse_github_url(url: &str) -> Result<RepoRef, FetchError> {
    let raw = url.trim();
    let invalid = || FetchError::InvalidUrl(url.to_string());

    // Normalize `git@github.com:owner/repo` → `github.com/owner/repo`.
    let host_and_path = if let Some(rest) = raw.strip_prefix("git@") {
        rest.replacen(':', "/", 1)
    } else {
        raw.strip_prefix("https://")
            .or_else(|| raw.strip_prefix("http://"))
            .unwrap_or(raw)
            .to_string()
    };

    let path = host_and_path
        .strip_prefix("github.com/")
        .or_else(|| host_and_path.strip_prefix("www.github.com/"))
        .ok_or_else(|| FetchError::UnsupportedHost(url.to_string()))?;

    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let owner = segments.next().ok_or_else(invalid)?;
    let repo = segments.next().ok_or_else(invalid)?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);

    if !is_valid_segment(owner) || !is_valid_segment(repo) {
        return Err(invalid());
    }

    Ok(RepoRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

/// The GitHub API tarball endpoint. Without a ref it resolves to the default branch and
/// redirects to codeload (ureq follows the redirect).
pub fn tarball_url(repo: &RepoRef, git_ref: Option<&str>) -> String {
    match git_ref {
        Some(r) => format!(
            "https://api.github.com/repos/{}/{}/tarball/{}",
            repo.owner, repo.repo, r
        ),
        None => format!(
            "https://api.github.com/repos/{}/{}/tarball",
            repo.owner, repo.repo
        ),
    }
}

/// GitHub owner/repo segment: non-empty, no path separators or traversal, no whitespace.
fn is_valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_url_forms() {
        let expect = RepoRef {
            owner: "octocat".into(),
            repo: "Hello-World".into(),
        };
        for url in [
            "https://github.com/octocat/Hello-World",
            "https://github.com/octocat/Hello-World.git",
            "https://github.com/octocat/Hello-World/",
            "http://github.com/octocat/Hello-World",
            "github.com/octocat/Hello-World",
            "https://github.com/octocat/Hello-World/tree/main",
            "git@github.com:octocat/Hello-World.git",
        ] {
            assert_eq!(parse_github_url(url).unwrap(), expect, "parsing {url}");
        }
    }

    #[test]
    fn rejects_non_github_hosts() {
        let err = parse_github_url("https://gitlab.com/a/b").unwrap_err();
        assert!(matches!(err, FetchError::UnsupportedHost(_)));
    }

    #[test]
    fn rejects_incomplete_or_dangerous_paths() {
        assert!(matches!(
            parse_github_url("https://github.com/onlyowner").unwrap_err(),
            FetchError::InvalidUrl(_)
        ));
        assert!(matches!(
            parse_github_url("https://github.com/../etc/passwd").unwrap_err(),
            FetchError::InvalidUrl(_)
        ));
    }

    #[test]
    fn builds_tarball_urls() {
        let r = RepoRef {
            owner: "octocat".into(),
            repo: "Hello-World".into(),
        };
        assert_eq!(
            tarball_url(&r, None),
            "https://api.github.com/repos/octocat/Hello-World/tarball"
        );
        assert_eq!(
            tarball_url(&r, Some("abc123")),
            "https://api.github.com/repos/octocat/Hello-World/tarball/abc123"
        );
    }
}
