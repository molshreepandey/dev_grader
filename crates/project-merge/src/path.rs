//! Path-safety helpers. The student's repository is untrusted input, so every path we read
//! out of it is validated: it must be a clean relative path, and no component along the way
//! may be a symlink (which could redirect a solution file to `/etc/passwd`, the hidden test
//! file, or outside the sandbox entirely).

use std::path::{Component, Path, PathBuf};

use crate::error::MergeError;

/// A clean relative path: not absolute, not empty, and made only of normal components
/// (no `..`, no `.`, no root/prefix). Solution paths come from our config, but we validate
/// them anyway so a malformed assignment config can't punch through the template.
pub fn is_safe_relative(rel: &str) -> bool {
    if rel.is_empty() {
        return false;
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return false;
    }
    let mut any = false;
    for comp in p.components() {
        match comp {
            Component::Normal(_) => any = true,
            _ => return false,
        }
    }
    any
}

/// Resolve `rel` under `base`, refusing to traverse a symlink at any component and requiring
/// the final component to be a regular file. Uses `symlink_metadata` (lstat) so the final
/// symlink is detected rather than followed.
pub fn resolve_student_file(base: &Path, rel: &str) -> Result<PathBuf, MergeError> {
    if !is_safe_relative(rel) {
        return Err(MergeError::UnsafePath(rel.to_string(), "not a clean relative path"));
    }

    let mut cur = base.to_path_buf();
    for comp in Path::new(rel).components() {
        let Component::Normal(name) = comp else {
            // Already excluded by is_safe_relative, but keep the guard exhaustive.
            return Err(MergeError::UnsafePath(rel.to_string(), "non-normal component"));
        };
        cur.push(name);
        let meta = std::fs::symlink_metadata(&cur)
            .map_err(|_| MergeError::MissingSolution(rel.to_string()))?;
        if meta.file_type().is_symlink() {
            return Err(MergeError::UnsafePath(rel.to_string(), "path traverses a symlink"));
        }
    }

    // Final component was lstat'd in the loop; confirm it is a regular file.
    let meta = std::fs::symlink_metadata(&cur).map_err(|e| MergeError::io(&cur, e))?;
    if !meta.file_type().is_file() {
        return Err(MergeError::NotAFile(rel.to_string()));
    }
    Ok(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_clean_relative_paths() {
        assert!(is_safe_relative("src/solution.py"));
        assert!(is_safe_relative("Solution.java"));
        assert!(is_safe_relative("a/b/c/d.js"));
    }

    #[test]
    fn rejects_traversal_absolute_and_empty() {
        assert!(!is_safe_relative(""));
        assert!(!is_safe_relative("/etc/passwd"));
        assert!(!is_safe_relative("../secret"));
        assert!(!is_safe_relative("src/../../etc/passwd"));
        assert!(!is_safe_relative("./src/x")); // leading `.` is a CurDir component
    }
}
