//! Merge a student's solution into a fresh copy of the assignment template.
//!
//! The pipeline never runs the student's repository as-is. Instead it takes a **fresh copy of
//! the trusted template** (which carries the hidden tests, lockfiles, and build config) and
//! overlays **only** the declared `solution_files` pulled out of the student's repo. Anything
//! else the student pushed — extra config, a `conftest.py`, a rewritten test — is ignored.
//!
//! This closes the common autograder cheats:
//! * rewriting the test file or the test script → tests come from the template, always;
//! * smuggling a file that monkeypatches the tests → only solution files are copied in;
//! * redirecting a solution path via a symlink → rejected in [`path`].

mod error;
mod path;

use std::path::Path;

pub use error::MergeError;
pub use path::{is_safe_relative, resolve_student_file};

/// What the merge produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeReport {
    /// Solution files copied from the student into the template (in declaration order).
    pub files_copied: Vec<String>,
}

/// Build a graded workspace at `dest` by copying `template_dir` there, then overlaying each
/// path in `solution_files` from `student_dir`.
///
/// `dest` is created if needed. `solution_files` are repo-relative paths (from the stack /
/// assignment config). Every solution file must exist in the student's repo as a regular file
/// reachable without traversing a symlink, otherwise the merge fails and nothing is graded.
pub fn merge_solution(
    template_dir: &Path,
    student_dir: &Path,
    solution_files: &[String],
    dest: &Path,
) -> Result<MergeReport, MergeError> {
    copy_tree(template_dir, dest)?;

    let mut files_copied = Vec::with_capacity(solution_files.len());
    for rel in solution_files {
        let src = resolve_student_file(student_dir, rel)?;
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MergeError::io(parent, e))?;
        }
        std::fs::copy(&src, &target).map_err(|e| MergeError::io(&target, e))?;
        files_copied.push(rel.clone());
    }

    Ok(MergeReport { files_copied })
}

/// Recursively copy `src` into `dst`, copying regular files and recreating directories.
/// Symlinks and special files in the template are skipped (the template is trusted, but we
/// keep the graded tree plain and deterministic).
fn copy_tree(src: &Path, dst: &Path) -> Result<(), MergeError> {
    std::fs::create_dir_all(dst).map_err(|e| MergeError::io(dst, e))?;
    let entries = std::fs::read_dir(src).map_err(|e| MergeError::io(src, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| MergeError::io(src, e))?;
        let file_type = entry.file_type().map_err(|e| MergeError::io(entry.path(), e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&from, &to)?;
        } else if file_type.is_file() {
            std::fs::copy(&from, &to).map_err(|e| MergeError::io(&to, e))?;
        }
        // symlinks / fifos / etc. are intentionally skipped
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Build a (template, student, dest) trio of temp dirs.
    struct Fixture {
        template: TempDir,
        student: TempDir,
        dest: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                template: TempDir::new().unwrap(),
                student: TempDir::new().unwrap(),
                dest: TempDir::new().unwrap(),
            }
        }

        fn write(dir: &TempDir, rel: &str, contents: &str) {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, contents).unwrap();
        }

        fn dest_read(&self, rel: &str) -> Option<String> {
            fs::read_to_string(self.dest.path().join(rel)).ok()
        }

        fn merge(&self, solution_files: &[&str]) -> Result<MergeReport, MergeError> {
            let owned: Vec<String> = solution_files.iter().map(|s| s.to_string()).collect();
            merge_solution(
                self.template.path(),
                self.student.path(),
                &owned,
                self.dest.path(),
            )
        }
    }

    #[test]
    fn overlays_student_solution_onto_template() {
        let fx = Fixture::new();
        // Template: stub solution + the hidden test + a lockfile.
        Fixture::write(&fx.template, "src/solution.py", "def add(a,b):\n    raise NotImplementedError\n");
        Fixture::write(&fx.template, "tests/test_solution.py", "HIDDEN TEST");
        Fixture::write(&fx.template, "requirements.txt", "pytest\n");
        // Student: real solution.
        Fixture::write(&fx.student, "src/solution.py", "def add(a,b):\n    return a+b\n");

        let report = fx.merge(&["src/solution.py"]).unwrap();
        assert_eq!(report.files_copied, vec!["src/solution.py"]);

        // Student's solution replaced the stub...
        assert_eq!(fx.dest_read("src/solution.py").unwrap(), "def add(a,b):\n    return a+b\n");
        // ...but the hidden test and lockfile came from the template.
        assert_eq!(fx.dest_read("tests/test_solution.py").unwrap(), "HIDDEN TEST");
        assert_eq!(fx.dest_read("requirements.txt").unwrap(), "pytest\n");
    }

    #[test]
    fn ignores_everything_the_student_pushed_except_solution_files() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "tests/test_solution.py", "HIDDEN TEST");
        Fixture::write(&fx.template, "src/solution.py", "stub");
        Fixture::write(&fx.student, "src/solution.py", "real");
        // Student tries to override the hidden test and smuggle a helper + a conftest.
        Fixture::write(&fx.student, "tests/test_solution.py", "def test_pass(): assert True");
        Fixture::write(&fx.student, "conftest.py", "cheat");
        Fixture::write(&fx.student, "hack.py", "cheat");

        fx.merge(&["src/solution.py"]).unwrap();

        // Hidden test is the template's, not the student's override.
        assert_eq!(fx.dest_read("tests/test_solution.py").unwrap(), "HIDDEN TEST");
        // Smuggled files did not make it into the graded workspace.
        assert!(fx.dest_read("conftest.py").is_none());
        assert!(fx.dest_read("hack.py").is_none());
    }

    #[test]
    fn missing_solution_file_is_an_error() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "src/solution.py", "stub");
        // student provides nothing

        let err = fx.merge(&["src/solution.py"]).unwrap_err();
        assert!(matches!(err, MergeError::MissingSolution(p) if p == "src/solution.py"));
    }

    #[test]
    fn creates_nested_parent_dirs_for_deep_solution_paths() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "readme.md", "x");
        Fixture::write(&fx.student, "src/main/java/Solution.java", "class Solution {}");

        fx.merge(&["src/main/java/Solution.java"]).unwrap();
        assert_eq!(
            fx.dest_read("src/main/java/Solution.java").unwrap(),
            "class Solution {}"
        );
    }

    #[test]
    fn rejects_symlinked_solution_file() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "src/solution.py", "stub");
        // Student makes the solution path a symlink pointing at a secret outside the repo.
        let secret = fx.template.path().join("tests/test_solution.py");
        fs::create_dir_all(fx.student.path().join("src")).unwrap();
        std::os::unix::fs::symlink(&secret, fx.student.path().join("src/solution.py")).unwrap();

        let err = fx.merge(&["src/solution.py"]).unwrap_err();
        assert!(matches!(err, MergeError::UnsafePath(..)));
    }

    #[test]
    fn rejects_directory_in_place_of_solution_file() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "src/solution.py", "stub");
        fs::create_dir_all(fx.student.path().join("src/solution.py")).unwrap();

        let err = fx.merge(&["src/solution.py"]).unwrap_err();
        assert!(matches!(err, MergeError::NotAFile(_)));
    }
}
