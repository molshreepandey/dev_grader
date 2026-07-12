//! The grading pipeline: resolve → fetch → merge → install → test → parse → grade.
//!
//! Every side effect is injected via a [`port`](crate::ports), so this logic is exercised end
//! to end in tests with fakes. Stage failures never panic or bubble out — they are mapped to a
//! [`GradeStatus`] so the worker always has a result to return, and each stage maps to its *own*
//! status, so the student is told which step let them down.
//!
//! Install and test are two separate sandboxed runs over the same workspace: the first is online
//! (that is how dependencies arrive), the second is not.

use std::path::Path;

use grader_types::{GradeResult, GradeStatus, ReportLocation, Submission, TestReport};
use report_parser::{parse_junit, parse_junit_many};
use tracing::{info, warn};

use crate::ports::{AssignmentStore, EngineError, Phase, ProjectRunner, RepoFetcher};

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
        let config = &assignment.config;

        // Isolated dirs for this run (dropped at the end). `home` is the writable $HOME both
        // phases share, so what the install downloads is cached where the tests can see it.
        let (student_dir, work_dir, home_dir) = match (
            tempdir(&self.work_root, "student"),
            tempdir(&self.work_root, "work"),
            tempdir(&self.work_root, "home"),
        ) {
            (Ok(s), Ok(w), Ok(h)) => (s, w, h),
            (Err(e), ..) | (_, Err(e), _) | (_, _, Err(e)) => {
                return fail(id, GradeStatus::InternalError, e);
            }
        };

        if let Err(e) = self
            .fetcher
            .fetch(&sub.repo_url, sub.git_ref.as_deref(), student_dir.path())
        {
            return fail(id, GradeStatus::FetchError, e);
        }

        // The submission does not have the shape the assignment requires (a missing solution file,
        // a symlink, a template missing a protected path): no gradable workspace exists.
        if let Err(e) = project_merge::merge(
            &config.merge,
            &assignment.template_dir,
            student_dir.path(),
            work_dir.path(),
        ) {
            return fail(id, GradeStatus::MergeError, EngineError::new(e.to_string()));
        }

        // ---- phase 1: install dependencies, with the network up ----
        if !config.install.is_empty() {
            let outcome = match self.runner.run(
                id,
                Phase::Install,
                work_dir.path(),
                home_dir.path(),
                config,
            ) {
                Ok(o) => o,
                Err(e) => return fail(id, GradeStatus::InternalError, e),
            };

            if outcome.timed_out {
                return fail(
                    id,
                    GradeStatus::Timeout,
                    EngineError::new("dependency install exceeded its time limit"),
                );
            }
            // A broken manifest, a package that does not exist, an unreachable registry. The
            // student's code has not run at all yet, so this is never a code failure.
            if outcome.failed() {
                return fail(
                    id,
                    GradeStatus::InstallError,
                    EngineError::new(with_output(
                        &format!(
                            "dependency install failed (exit {})",
                            outcome.exit_code
                        ),
                        &outcome.stderr_tail,
                    )),
                );
            }
            info!("{id}: dependencies installed");
        }

        // Anything the install wrote where the report belongs is not evidence of a passing test —
        // a package's postinstall script could have planted it. Start the test phase clean.
        if let Err(e) = clear_report(work_dir.path(), &config.report) {
            return fail(id, GradeStatus::InternalError, e);
        }

        // ---- phase 2: run the hidden tests, offline ----
        let outcome = match self
            .runner
            .run(id, Phase::Test, work_dir.path(), home_dir.path(), config)
        {
            Ok(o) => o,
            Err(e) => return fail(id, GradeStatus::InternalError, e),
        };

        if outcome.timed_out {
            return fail(
                id,
                GradeStatus::Timeout,
                EngineError::new("tests exceeded the wall-clock limit"),
            );
        }

        // The tests ran iff a report was produced: a runner that starts always writes one, even
        // when every test fails. No report ⇒ the code did not compile or could not be collected.
        match read_report(work_dir.path(), &config.report) {
            Ok(report) => GradeResult::graded(id, &report),
            Err(e) => {
                warn!("no report for {id}: {e}");
                let cause = if outcome.is_oom {
                    "memory limit exceeded"
                } else {
                    "the code did not compile, or the tests could not be collected"
                };
                fail(
                    id,
                    GradeStatus::BuildError,
                    EngineError::new(with_output(
                        &format!("no test report produced ({cause})"),
                        &outcome.stderr_tail,
                    )),
                )
            }
        }
    }
}

/// Append the captured output of a failed phase to its summary, when there is any.
fn with_output(summary: &str, output: &str) -> String {
    let output = output.trim();
    if output.is_empty() {
        summary.to_string()
    } else {
        format!("{summary}; output:\n{output}")
    }
}

/// Delete whatever currently sits at the report location, so a report found afterwards can only
/// have been written by our test command.
fn clear_report(work_dir: &Path, location: &ReportLocation) -> Result<(), EngineError> {
    match location {
        ReportLocation::File(rel) => {
            let path = work_dir.join(rel);
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(EngineError::new(format!("clear {}: {e}", path.display()))),
            }
        }
        ReportLocation::Glob(pattern) => {
            let full = work_dir.join(pattern);
            let full = full.to_str().ok_or_else(|| EngineError::new("non-utf8 glob"))?;
            for entry in glob::glob(full).map_err(|e| EngineError::new(e.to_string()))? {
                let path = entry.map_err(|e| EngineError::new(e.to_string()))?;
                std::fs::remove_file(&path)
                    .map_err(|e| EngineError::new(format!("clear {}: {e}", path.display())))?;
            }
            Ok(())
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
    use std::cell::RefCell;
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

    /// What a phase should do when the fake runner reaches it.
    #[derive(Clone, Default)]
    struct PhaseScript {
        /// A file to write into the workspace: (relative path, contents).
        writes: Option<(String, String)>,
        outcome: RunOutcome,
    }

    impl PhaseScript {
        /// A phase that succeeds and writes a JUnit report.
        fn reports(rel: &str, xml: &str) -> Self {
            PhaseScript {
                writes: Some((rel.into(), xml.into())),
                outcome: RunOutcome::default(),
            }
        }
        fn ok() -> Self {
            PhaseScript::default()
        }
        fn exits(code: i32, stderr: &str) -> Self {
            PhaseScript {
                writes: None,
                outcome: RunOutcome {
                    exit_code: code,
                    stderr_tail: stderr.into(),
                    ..Default::default()
                },
            }
        }
        fn times_out() -> Self {
            PhaseScript {
                writes: None,
                outcome: RunOutcome {
                    timed_out: true,
                    ..Default::default()
                },
            }
        }
    }

    /// Runner that plays a script per phase and records which phases ran, with what network.
    struct ScriptedRunner {
        install: PhaseScript,
        test: PhaseScript,
        seen: RefCell<Vec<(Phase, bool)>>,
    }

    impl ScriptedRunner {
        fn new(install: PhaseScript, test: PhaseScript) -> Self {
            ScriptedRunner {
                install,
                test,
                seen: RefCell::new(Vec::new()),
            }
        }
        /// Install is a no-op; the tests write `report`.
        fn testing(test: PhaseScript) -> Self {
            ScriptedRunner::new(PhaseScript::ok(), test)
        }
    }

    impl ProjectRunner for ScriptedRunner {
        fn run(
            &self,
            _id: &str,
            phase: Phase,
            work_dir: &Path,
            home_dir: &Path,
            _c: &StackConfig,
        ) -> Result<RunOutcome, EngineError> {
            assert!(home_dir.is_dir(), "$HOME must exist and be a real directory");
            self.seen.borrow_mut().push((phase, phase.network()));

            let script = match phase {
                Phase::Install => &self.install,
                Phase::Test => &self.test,
            };
            if let Some((rel, contents)) = &script.writes {
                let p = work_dir.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, contents).unwrap();
            }
            Ok(script.outcome.clone())
        }
    }

    fn python_config() -> StackConfig {
        StackConfig {
            stack: Stack::Python,
            merge: MergeMode::SolutionFiles {
                files: vec!["src/solution.py".into()],
            },
            install: vec!["pip".into(), "install".into()],
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

    fn good_student() -> SolutionFetcher {
        SolutionFetcher {
            rel: "src/solution.py".into(),
            contents: "real".into(),
        }
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
    fn happy_path_installs_then_tests_and_grades_from_the_report() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            good_student(),
            ScriptedRunner::testing(PhaseScript::reports("report.xml", REPORT_2_PASS_1_FAIL)),
            &root,
        );

        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Graded);
        assert_eq!(result.total, 3);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.failing_tests, vec!["c"]);
    }

    /// The security property the whole two-phase split exists for: dependencies are downloaded
    /// with the network up, and the student's code then runs with it gone.
    #[test]
    fn install_runs_online_and_tests_run_offline() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let runner = ScriptedRunner::testing(PhaseScript::reports("report.xml", REPORT_2_PASS_1_FAIL));
        let eng = engine(&tpl, python_config(), good_student(), runner, &root);

        eng.grade(&submission());

        let seen = eng.runner.seen.borrow().clone();
        assert_eq!(
            seen,
            vec![(Phase::Install, true), (Phase::Test, false)],
            "install must run first and online; tests second and offline"
        );
    }

    #[test]
    fn a_failing_install_is_an_install_error_carrying_the_output() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            good_student(),
            ScriptedRunner::new(
                PhaseScript::exits(1, "ERROR: No matching distribution found for nosuchpkg==9.9"),
                PhaseScript::reports("report.xml", REPORT_2_PASS_1_FAIL),
            ),
            &root,
        );

        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::InstallError);
        let error = result.error.unwrap();
        assert!(error.contains("dependency install failed"), "{error}");
        assert!(error.contains("nosuchpkg"), "the resolver's message must reach the student");
        // The tests must never have run — their report would otherwise have been graded.
        assert_eq!(eng.runner.seen.borrow().len(), 1);
    }

    #[test]
    fn an_install_that_times_out_is_a_timeout() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            good_student(),
            ScriptedRunner::new(PhaseScript::times_out(), PhaseScript::ok()),
            &root,
        );

        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Timeout);
        assert!(result.error.unwrap().contains("install"));
    }

    /// A dependency's postinstall script can write anything it likes into the workspace — including
    /// a report full of passing tests. It must not survive into the grade.
    #[test]
    fn a_report_planted_during_install_is_discarded() {
        const ALL_PASSING: &str = r#"<testsuite name="fake" tests="1">
          <testcase name="i_am_perfect" time="0"/>
        </testsuite>"#;

        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            good_student(),
            ScriptedRunner::new(
                PhaseScript::reports("report.xml", ALL_PASSING),
                // The real test phase then fails to produce anything (say, a compile error).
                PhaseScript::exits(1, "SyntaxError"),
            ),
            &root,
        );

        let result = eng.grade(&submission());
        assert_eq!(
            result.status,
            GradeStatus::BuildError,
            "the planted report must not be graded"
        );
        assert!(result.failing_tests.is_empty());
        assert_eq!(result.passed, 0);
    }

    #[test]
    fn unknown_assignment_is_internal_error() {
        let root = TempDir::new().unwrap();
        let eng = Engine::new(
            MissingStore,
            good_student(),
            ScriptedRunner::testing(PhaseScript::ok()),
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
            ScriptedRunner::testing(PhaseScript::ok()),
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::FetchError);
        assert_eq!(result.error.as_deref(), Some("404 not found"));
    }

    #[test]
    fn missing_solution_file_maps_to_merge_error() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        // The student pushed something, but not the file the assignment declares.
        let eng = engine(
            &tpl,
            python_config(),
            SolutionFetcher {
                rel: "notes.txt".into(),
                contents: "hi".into(),
            },
            ScriptedRunner::testing(PhaseScript::ok()),
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::MergeError);
        assert!(result.error.unwrap().contains("src/solution.py"));
        // Nothing was ever run.
        assert!(eng.runner.seen.borrow().is_empty());
    }

    #[test]
    fn tests_timing_out_maps_to_timeout_status() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            good_student(),
            ScriptedRunner::testing(PhaseScript::times_out()),
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Timeout);
        assert!(result.error.unwrap().contains("tests"));
    }

    #[test]
    fn no_report_after_the_tests_is_a_build_error_with_the_output() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let eng = engine(
            &tpl,
            python_config(),
            good_student(),
            ScriptedRunner::testing(PhaseScript::exits(
                1,
                "ModuleNotFoundError: No module named 'numpy'",
            )),
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::BuildError);
        assert!(result.error.unwrap().contains("numpy"));
    }

    #[test]
    fn an_empty_install_command_skips_the_install_phase() {
        let tpl = template();
        let root = TempDir::new().unwrap();
        let mut config = python_config();
        config.install = vec![];
        let eng = engine(
            &tpl,
            config,
            good_student(),
            ScriptedRunner::testing(PhaseScript::reports("report.xml", REPORT_2_PASS_1_FAIL)),
            &root,
        );

        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Graded);
        assert_eq!(eng.runner.seen.borrow().clone(), vec![(Phase::Test, false)]);
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
            good_student(),
            ScriptedRunner::testing(PhaseScript::reports(
                "target/surefire-reports/TEST-a.xml",
                REPORT_2_PASS_1_FAIL,
            )),
            &root,
        );
        let result = eng.grade(&submission());
        assert_eq!(result.status, GradeStatus::Graded);
        assert_eq!(result.total, 3);
    }
}
