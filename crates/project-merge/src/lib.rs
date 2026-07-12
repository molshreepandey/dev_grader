//! Merge a student's submission with the assignment template into a graded workspace.
//!
//! The pipeline never runs the student's repository unmodified. Two shapes are supported via
//! [`MergeMode`]:
//!
//! * [`MergeMode::SolutionFiles`] — base is a fresh copy of the trusted **template**; only the
//!   declared student files are overlaid in. Strongest anti-cheat (function-level exercises).
//! * [`MergeMode::WholeProject`] — the student submits a whole project; base is **their repo**
//!   and the template's `protected_paths` (hidden tests + locked build/test config) are stamped
//!   on top, always winning. The student's version of each protected path is removed first.
//!
//! Either way, the graded tree carries the template's hidden tests and the pipeline runs its
//! own test command — never the student's scripts. Untrusted paths are validated in [`path`]:
//! symlinks are never traversed or recreated, and `.git` is dropped.

mod error;
mod path;

use std::path::Path;

use grader_types::MergeMode;

pub use error::MergeError;
pub use path::{is_safe_relative, resolve_student_file};

/// Top-level directories in a student repo that are never copied into the graded workspace.
const STUDENT_IGNORE: &[&str] = &[".git"];

/// What the merge produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Solution files copied from the student (`SolutionFiles` mode).
    pub files_copied: Vec<String>,
    /// Protected paths stamped from the template over the student's repo (`WholeProject` mode).
    pub protected_applied: Vec<String>,
}

/// Build a graded workspace at `dest` from the `template_dir` and `student_dir` per `mode`.
/// `dest` is created if needed and is expected to be empty.
pub fn merge(
    mode: &MergeMode,
    template_dir: &Path,
    student_dir: &Path,
    dest: &Path,
) -> Result<MergeReport, MergeError> {
    match mode {
        MergeMode::SolutionFiles { files } => {
            merge_solution_files(template_dir, student_dir, files, dest)
        }
        MergeMode::WholeProject { protected_paths } => {
            merge_whole_project(template_dir, student_dir, protected_paths, dest)
        }
    }
}

/// Template is the base; copy in only the declared student solution files.
fn merge_solution_files(
    template_dir: &Path,
    student_dir: &Path,
    solution_files: &[String],
    dest: &Path,
) -> Result<MergeReport, MergeError> {
    copy_tree(template_dir, dest)?;

    let mut files_copied = Vec::with_capacity(solution_files.len());
    for rel in solution_files {
        let src = resolve_student_file(student_dir, rel)?;
        copy_file_creating_parents(&src, &dest.join(rel))?;
        files_copied.push(rel.clone());
    }

    Ok(MergeReport {
        files_copied,
        protected_applied: Vec::new(),
    })
}

/// Student's whole repo is the base; stamp the template's protected paths on top.
fn merge_whole_project(
    template_dir: &Path,
    student_dir: &Path,
    protected_paths: &[String],
    dest: &Path,
) -> Result<MergeReport, MergeError> {
    // 1. Copy the student's entire project (skipping .git and symlinks).
    copy_tree(student_dir, dest)?;

    // 2. Force each protected path to come from the template, wiping the student's version.
    let mut protected_applied = Vec::with_capacity(protected_paths.len());
    for rel in protected_paths {
        if !is_safe_relative(rel) {
            return Err(MergeError::UnsafePath(rel.clone(), "not a clean relative path"));
        }
        let src = template_dir.join(rel);
        let meta = std::fs::symlink_metadata(&src)
            .map_err(|_| MergeError::ProtectedPathMissing(rel.clone()))?;

        let target = dest.join(rel);
        remove_path(&target)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MergeError::io(parent, e))?;
        }
        if meta.file_type().is_dir() {
            copy_tree(&src, &target)?;
        } else {
            copy_file_creating_parents(&src, &target)?;
        }
        protected_applied.push(rel.clone());
    }

    Ok(MergeReport {
        files_copied: Vec::new(),
        protected_applied,
    })
}

fn copy_file_creating_parents(src: &Path, target: &Path) -> Result<(), MergeError> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| MergeError::io(parent, e))?;
    }
    std::fs::copy(src, target).map_err(|e| MergeError::io(target, e))?;
    Ok(())
}

/// Remove a file or directory if it exists; a missing path is not an error.
fn remove_path(p: &Path) -> Result<(), MergeError> {
    match std::fs::symlink_metadata(p) {
        Ok(meta) if meta.file_type().is_dir() => {
            std::fs::remove_dir_all(p).map_err(|e| MergeError::io(p, e))
        }
        Ok(_) => std::fs::remove_file(p).map_err(|e| MergeError::io(p, e)),
        Err(_) => Ok(()),
    }
}

/// Recursively copy `src` into `dst`, copying regular files and recreating directories.
/// Symlinks/special files are skipped (never recreated from an untrusted tree), and any
/// top-level entry in [`STUDENT_IGNORE`] (`.git`) is dropped.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), MergeError> {
    std::fs::create_dir_all(dst).map_err(|e| MergeError::io(dst, e))?;
    let entries = std::fs::read_dir(src).map_err(|e| MergeError::io(src, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| MergeError::io(src, e))?;
        let name = entry.file_name();
        if STUDENT_IGNORE.iter().any(|ig| name == *ig) {
            continue;
        }
        let file_type = entry.file_type().map_err(|e| MergeError::io(entry.path(), e))?;
        let from = entry.path();
        let to = dst.join(&name);
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

        fn dest_exists(&self, rel: &str) -> bool {
            self.dest.path().join(rel).exists()
        }

        fn run(&self, mode: &MergeMode) -> Result<MergeReport, MergeError> {
            merge(mode, self.template.path(), self.student.path(), self.dest.path())
        }

        fn solution(files: &[&str]) -> MergeMode {
            MergeMode::SolutionFiles {
                files: files.iter().map(|s| s.to_string()).collect(),
            }
        }

        fn whole(protected: &[&str]) -> MergeMode {
            MergeMode::WholeProject {
                protected_paths: protected.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    // ---- SolutionFiles mode ----

    #[test]
    fn solution_files_overlays_student_onto_template() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "src/solution.py", "stub");
        Fixture::write(&fx.template, "tests/test_solution.py", "HIDDEN TEST");
        Fixture::write(&fx.student, "src/solution.py", "real");

        let report = fx.run(&Fixture::solution(&["src/solution.py"])).unwrap();
        assert_eq!(report.files_copied, vec!["src/solution.py"]);
        assert_eq!(fx.dest_read("src/solution.py").unwrap(), "real");
        assert_eq!(fx.dest_read("tests/test_solution.py").unwrap(), "HIDDEN TEST");
    }

    #[test]
    fn solution_files_ignores_smuggled_files_and_missing_errors() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "src/solution.py", "stub");
        Fixture::write(&fx.student, "hack.py", "cheat");
        // Missing src/solution.py from student.
        let err = fx.run(&Fixture::solution(&["src/solution.py"])).unwrap_err();
        assert!(matches!(err, MergeError::MissingSolution(_)));
        assert!(!fx.dest_exists("hack.py"));
    }

    #[test]
    fn solution_files_rejects_symlinked_solution() {
        let fx = Fixture::new();
        Fixture::write(&fx.template, "src/solution.py", "stub");
        let secret = fx.template.path().join("tests/test_solution.py");
        fs::create_dir_all(fx.student.path().join("src")).unwrap();
        std::os::unix::fs::symlink(&secret, fx.student.path().join("src/solution.py")).unwrap();

        let err = fx.run(&Fixture::solution(&["src/solution.py"])).unwrap_err();
        assert!(matches!(err, MergeError::UnsafePath(..)));
    }

    // ---- WholeProject mode ----

    #[test]
    fn whole_project_keeps_student_tree_but_stamps_hidden_tests() {
        let fx = Fixture::new();
        // Student's whole MERN-ish project.
        Fixture::write(&fx.student, "src/app.js", "student app code");
        Fixture::write(&fx.student, "package.json", "{ \"name\": \"student\" }");
        // Student tries to ship their own passing test.
        Fixture::write(&fx.student, "tests/fake.test.js", "test('x',()=>{})");
        // Template's hidden test.
        Fixture::write(&fx.template, "tests/hidden.test.js", "HIDDEN");

        let report = fx.run(&Fixture::whole(&["tests"])).unwrap();
        assert_eq!(report.protected_applied, vec!["tests"]);

        // Student's own source and package.json survive...
        assert_eq!(fx.dest_read("src/app.js").unwrap(), "student app code");
        assert_eq!(fx.dest_read("package.json").unwrap(), "{ \"name\": \"student\" }");
        // ...the hidden test is present...
        assert_eq!(fx.dest_read("tests/hidden.test.js").unwrap(), "HIDDEN");
        // ...and the student's fake test inside the protected dir is gone.
        assert!(!fx.dest_exists("tests/fake.test.js"));
    }

    #[test]
    fn whole_project_protected_file_overwrites_student_version() {
        let fx = Fixture::new();
        Fixture::write(&fx.student, "src/main/java/Solution.java", "class Solution {}");
        Fixture::write(&fx.student, "pom.xml", "STUDENT POM (tampered)");
        Fixture::write(&fx.template, "pom.xml", "LOCKED POM");
        Fixture::write(&fx.template, "src/test/SolutionTest.java", "HIDDEN");

        fx.run(&Fixture::whole(&["src/test", "pom.xml"])).unwrap();

        assert_eq!(fx.dest_read("src/main/java/Solution.java").unwrap(), "class Solution {}");
        assert_eq!(fx.dest_read("pom.xml").unwrap(), "LOCKED POM");
        assert_eq!(fx.dest_read("src/test/SolutionTest.java").unwrap(), "HIDDEN");
    }

    #[test]
    fn whole_project_drops_git_and_skips_symlinks() {
        let fx = Fixture::new();
        Fixture::write(&fx.student, "src/app.js", "code");
        Fixture::write(&fx.student, ".git/config", "[core]");
        std::os::unix::fs::symlink("/etc/passwd", fx.student.path().join("link")).unwrap();
        Fixture::write(&fx.template, "tests/hidden.test.js", "HIDDEN");

        fx.run(&Fixture::whole(&["tests"])).unwrap();

        assert_eq!(fx.dest_read("src/app.js").unwrap(), "code");
        assert!(!fx.dest_exists(".git")); // .git never copied
        assert!(!fx.dest_exists("link")); // symlink never recreated
    }

    #[test]
    fn whole_project_missing_template_protected_path_errors() {
        let fx = Fixture::new();
        Fixture::write(&fx.student, "src/app.js", "code");
        // template has no `tests` dir
        let err = fx.run(&Fixture::whole(&["tests"])).unwrap_err();
        assert!(matches!(err, MergeError::ProtectedPathMissing(p) if p == "tests"));
    }
}
