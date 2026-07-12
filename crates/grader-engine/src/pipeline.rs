//! The grading pipeline: resolve → fetch → merge → run → parse → grade.
//!
//! Every side effect is injected via a [`port`](crate::ports), so this logic is exercised end
//! to end in tests with fakes. Stage failures never panic or bubble out — they are mapped to a
//! [`GradeStatus`] so the worker always has a result to return.

use std::path::Path;

use grader_types::{GradeResult, GradeStatus, ReportLocation, Submission, TestReport};
use report_parser::{parse_junit, parse_junit_many};
use tracing::warn;

use crate::ports::{AssignmentStore, EngineError, ProjectRunner, RepoFetcher};

/// Wires the three ports together and grades submissions.
pub struct Engine<S, F, R> {
    store: S,
    fetcher: F,
    runner: R,
    /// Parent directory under which per-submission student/work dirs are created.
    work_root: std::path::PathBuf,
}

impl<S: AssignmentStore, F: RepoFetcher, R: ProjectRunner> Engine<S, F, R> {
    pub fn new(store: S, fetcher: F, runner: R, work_root: impl Into<std::path::PathBuf>) -> Self {
        Engine {
            store,
            fetcher,
            runner,
            work_root: work_root.into(),
        }
    }

    /// Grade one submission, always returning a [`GradeResult`].
    pub fn grade(&self, sub: &Submission) -> GradeResult {
        let id = &sub.submission_id;

        let assignment = match self.store.resolve(&sub.assignment_id) {
            Ok(a) => a,
            Err(e) => return fail(id, GradeStatus::InternalError, e),
        };

        // Isolated dirs for this run (dropped at the end).
        let student_dir = match tempdir(&self.work_root, "student") {
            Ok(d) => d,
            Err(e) => return fail(id, GradeStatus::InternalError, e),
        };
        let work_dir = match tempdir(&self.work_root, "work") {
            Ok(d) => d,
            Err(e) => return fail(id, GradeStatus::InternalError, e),
        };

        if let Err(e) = self
            .fetcher
            .fetch(&sub.repo_url, sub.git_ref.as_deref(), student_dir.path())
        {
            return fail(id, GradeStatus::FetchError, e);
        }

        // A merge failure (missing solution file, symlink, tampered protected path) means we
        // can't build a gradable workspace — treat as a build error.
        if let Err(e) = project_merge::merge(
            &assignment.config.merge,
            &assignment.template_dir,
            student_dir.path(),
            work_dir.path(),
        ) {
            return fail(id, GradeStatus::BuildError, EngineError::new(e.to_string()));
        }

        let outcome = match self.runner.run(id, work_dir.path(), &assignment.config) {
            Ok(o) => o,
            Err(e) => return fail(id, GradeStatus::InternalError, e),
        };

        if outcome.timed_out {
            return fail(id, GradeStatus::Timeout, EngineError::new("wall-clock limit exceeded"));
        }

        // Tests ran iff a report was produced. No report → install/compile failed (or OOM).
        match read_report(work_dir.path(), &assignment.config.report) {
            Ok(report) => GradeResult::graded(id, &report),
            Err(e) => {
                warn!("no report for {id}: {e}");
                let cause = if outcome.is_oom {
                    "memory limit exceeded"
                } else {
                    "install/build likely failed"
                };
                let detail = if outcome.stderr_tail.is_empty() {
                    format!("no test report produced ({cause})")
                } else {
                    format!("no test report produced ({cause}); output:\n{}", outcome.stderr_tail)
                };
                fail(id, GradeStatus::BuildError, EngineError::new(detail))
            }
        }
    }
}

fn fail(id: &str, status: GradeStatus, err: EngineError) -> GradeResult {
    GradeResult::failed(id, status, err.0)
}

fn tempdir(root: &Path, prefix: &str) -> Result<tempfile::TempDir, EngineError> {
    std::fs::create_dir_all(root).map_err(|e| EngineError::new(e.to_string()))?;
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .map_err(|e| EngineError::new(e.to_string()))
}

/// Read and parse the JUnit report(s) from the graded workspace.
fn read_report(work_dir: &Path, location: &ReportLocation) -> Result<TestReport, EngineError> {
    match location {
        ReportLocation::File(rel) => {
            let path = work_dir.join(rel);
            let xml = std::fs::read_to_string(&path)
                .map_err(|e| EngineError::new(format!("read {}: {e}", path.display())))?;
            parse_junit(&xml).map_err(|e| EngineError::new(e.to_string()))
        }
        ReportLocation::Glob(pattern) => {
            let full = work_dir.join(pattern);
            let full = full.to_str().ok_or_else(|| EngineError::new("non-utf8 glob"))?;
            let mut docs = Vec::new();
            for entry in glob::glob(full).map_err(|e| EngineError::new(e.to_string()))? {
                let path = entry.map_err(|e| EngineError::new(e.to_string()))?;
                docs.push(
                    std::fs::read_to_string(&path)
                        .map_err(|e| EngineError::new(format!("read {}: {e}", path.display())))?,
                );
            }
            if docs.is_empty() {
                return Err(EngineError::new("no report files matched glob"));
            }
            parse_junit_many(docs.iter().map(String::as_str))
                .map_err(|e| EngineError::new(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{Assignment, RunOutcome};
    use grader_types::{MergeMode, ReportLocation, Stack, StackConfig};
    use std::path::PathBuf;
    use tempfile::TempDir;

    const REPORT_2_PASS_1_FAIL: &str = r#"<testsuite name="s" tests="3">
      <testcase name="a" time="0"/>
      <testcase name="b" time="0"/>
      <testcase name="c" time="0"><failure message="boom">x</failure></testcase>
    </testsuite>"#;

    /// Assignment store backed by a template dir + fixed config.
    struct FakeStore {
        template_dir: PathBuf,
        config: StackConfig,
    }
    impl AssignmentStore for FakeStore {
        fn resolve(&self, _id: &str) -> Result<Assignment, EngineError> {
            Ok(Assignment {
                template_dir: self.template_dir.clone(),
                config: self.config.clone(),
            })
        }
    }
    struct MissingStore;
    impl AssignmentStore for MissingStore {
        fn resolve(&self, id: &str) -> Result<Assignment, EngineError> {
            Err(EngineError::new(format!("unknown assignment {id}")))
        }
    }

    /// Fetcher that drops a solution file into the student dir.
    struct SolutionFetcher {
        rel: String,
        contents: String,
    }
    impl RepoFetcher for SolutionFetcher {
        fn fetch(&self, _url: &str, _r: Option<&str>, dest: &Path) -> Result<(), EngineError> {
            let p = dest.join(&self.rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, &self.contents).unwrap();
            Ok(())
        }
    }
    struct FailingFetcher;
    impl RepoFetcher for FailingFetcher {
        fn fetch(&self, _url: &str, _r: Option<&str>, _d: &Path) -> Result<(), EngineError> {
            Err(EngineError::new("404 not found"))
        }
    }

    /// Runner that writes a canned report and returns a fixed outcome.
    struct ScriptedRunner {
        report: Option<(String, String)>, // (rel path, xml)
        outcome: RunOutcome,
    }
    impl ProjectRunner for ScriptedRunner {
        fn run(&self, _id: &str, work_dir: &Path, _c: &StackConfig) -> Result<RunOutcome, EngineError> {
            if let Some((rel, xml)) = &self.report {
                let p = work_dir.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, xml).unwrap();
            }
            Ok(self.outcome.clone())
        }
    }

    fn python_config() -> StackConfig {
        StackConfig {
            stack: Stack::Python,
            merge: MergeMode::SolutionFiles {
                files: vec!["src/solution.py".into()],
            },
            install: vec![],
            test: vec!["pytest".into()],
            report: ReportLocation::File("report.xml".into()),
        }
    }

    /// Template with the stub + hidden test present.
    fn template() -> TempDir {
        let t = TempDir::new().unwrap();
        std::fs::create_dir_all(t.path().join("src")).unwrap();
        std::fs::write(t.path().join("src/solution.py"), "stub").unwrap();
        t
    }

    fn engine<F: RepoFetcher, R: ProjectRunner>(
        template: &TempDir,
        config: StackConfig,
        fetcher: F,
        runner: R,
        work_root: &TempDir,
    ) -> Engine<FakeStore, F, R> {
        Engine::new(
            FakeStore {
                template_dir: template.path().to_path_buf(),
                config,
            },
            fetcher,
            runner,
            work_root.path().to_path_buf(),
        )
    }

    fn submission() -> Submission {
        Submission {
            submission_id: "sub-1".into(),
            assignment_id: "hw1".into(),
            stack: Stack::Python,
            repo_url: "https://github.com/stu/hw1".into(),
            git_ref: None,
        }
    }

    #[test]
    fn happy_path_grades_from_report() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            SolutionFetcher {
                rel: "src/solution.py".into(),
                contents: "real".into(),
            },
            ScriptedRunner {
                report: Some(("report.xml".into(), REPORT_2_PASS_1_FAIL.into())),
                outcome: RunOutcome::default(),
            },
            &root,
        );

        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Graded);
        assert_eq!(result.total, 3);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.failing_tests, vec!["c"]);
    }

    #[test]
    fn unknown_assignment_is_internal_error() {
        let root = TempDir::new().unwrap();
        let eng = Engine::new(
            MissingStore,
            SolutionFetcher { rel: "x".into(), contents: "y".into() },
            ScriptedRunner { report: None, outcome: RunOutcome::default() },
            root.path().to_path_buf(),
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::InternalError);
    }

    #[test]
    fn fetch_failure_maps_to_fetch_error() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            FailingFetcher,
            ScriptedRunner { report: None, outcome: RunOutcome::default() },
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::FetchError);
        assert_eq!(result.error.as_deref(), Some("404 not found"));
    }

    #[test]
    fn missing_solution_file_maps_to_build_error() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        // Fetcher writes an unrelated file, not the required src/solution.py.
        let eng = engine(
            &tpl,
            python_config(),
            SolutionFetcher { rel: "notes.txt".into(), contents: "hi".into() },
            ScriptedRunner { report: None, outcome: RunOutcome::default() },
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::BuildError);
    }

    #[test]
    fn timeout_maps_to_timeout_status() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            SolutionFetcher { rel: "src/solution.py".into(), contents: "real".into() },
            ScriptedRunner {
                report: None,
                outcome: RunOutcome { timed_out: true, ..Default::default() },
            },
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Timeout);
    }

    #[test]
    fn no_report_after_run_is_build_error_with_stderr() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            SolutionFetcher { rel: "src/solution.py".into(), contents: "real".into() },
            ScriptedRunner {
                report: None,
                outcome: RunOutcome {
                    exit_code: 1,
                    stderr_tail: "ModuleNotFoundError: numpy".into(),
                    ..Default::default()
                },
            },
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::BuildError);
        assert!(result.error.unwrap().contains("numpy"));
    }

    #[test]
    fn glob_report_location_merges_surefire_files() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let mut config = python_config();
        config.report = ReportLocation::Glob("target/surefire-reports/*.xml".into());
        let eng = engine(
            &tpl,
            config,
            SolutionFetcher { rel: "src/solution.py".into(), contents: "real".into() },
            ScriptedRunner {
                report: Some((
                    "target/surefire-reports/TEST-a.xml".into(),
                    REPORT_2_PASS_1_FAIL.into(),
                )),
                outcome: RunOutcome::default(),
            },
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Graded);
        assert_eq!(result.total, 3);
    }
}
