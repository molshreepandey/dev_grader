use serde::{Deserialize, Serialize};

use crate::stack::Stack;

/// How the student's submission is combined with the assignment template.
///
/// Two assignment shapes are supported:
/// * [`MergeMode::SolutionFiles`] — "implement these files": the base is the trusted template
///   and only the declared student files are copied in. Strongest anti-cheat; best for
///   function-level exercises.
/// * [`MergeMode::WholeProject`] — the student submits a whole project repository; the base is
///   their repo and the template's `protected_paths` (hidden tests + locked build/test config)
///   are stamped on top, always winning. Best for real MERN / Maven / package assignments.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MergeMode {
    SolutionFiles {
        /// Repo-relative paths copied FROM the student INTO the template.
        files: Vec<String>,
    },
    WholeProject {
        /// Repo-relative files/dirs that always come from the template, overwriting whatever
        /// the student put there (e.g. `tests`, `pom.xml`). The student's version is removed
        /// first so no extra files linger inside a protected directory.
        protected_paths: Vec<String>,
    },
}

/// Where the JUnit XML report lands after the test command runs.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportLocation {
    /// A single report file at this workspace-relative path.
    File(String),
    /// A glob matching one-or-more report files (surefire writes one per class).
    Glob(String),
}

/// Per-stack recipe: how the submission is merged, how its dependencies are installed, how the
/// tests are run, and where the JUnit XML ends up. An assignment's grader config may override any
/// of these; [`StackConfig::default_for`] provides sane defaults.
///
/// The pipeline always runs *its own* `test` command, never the student's scripts.
///
/// # The two phases, and what each may assume
///
/// `install` and `test` run as **two separate sandboxed processes** over the same workspace, and
/// they do not get the same sandbox:
///
/// | | `install` | `test` |
/// |---|---|---|
/// | Network | **yes** — this is where dependencies are downloaded | **no** — the netns has no interface |
/// | Writable | `/work` (cwd), `$HOME`, `/tmp` | the same paths, with whatever install left there |
/// | Time budget | generous (a cold Maven or npm resolve is slow) | tight |
///
/// So `install` may reach a registry, and anything it writes — `node_modules/`, `.venv/`,
/// `~/.m2/repository` — is still there when the tests run offline. Because dependencies are
/// resolved per submission at grade time, each assignment (and, where the manifest is not a
/// protected path, each student) can bring whatever dependencies it wants: nothing is baked into
/// the image.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct StackConfig {
    pub stack: Stack,
    /// How to combine the student's submission with the template.
    pub merge: MergeMode,
    /// Dependency install command (argv), run **with network**. Empty = nothing to install.
    /// A non-zero exit here is a [`crate::GradeStatus::InstallError`].
    pub install: Vec<String>,
    /// Test command (argv), run **without network**; must emit JUnit XML.
    pub test: Vec<String>,
    /// Where to read the JUnit XML from afterwards.
    pub report: ReportLocation,
}

impl StackConfig {
    pub fn default_for(stack: Stack) -> Self {
        match stack {
            // Function-level exercise: template is the base, student fills in one file.
            Stack::Python => StackConfig {
                stack,
                merge: MergeMode::SolutionFiles {
                    files: vec!["src/solution.py".into()],
                },
                // A venv inside /work, so the packages land in the workspace and survive into the
                // offline test phase. requirements.txt must include pytest.
                install: vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "python3 -m venv .venv && .venv/bin/pip install --no-input -r requirements.txt"
                        .into(),
                ],
                test: vec![
                    ".venv/bin/pytest".into(),
                    "--junitxml=report.xml".into(),
                    "-q".into(),
                ],
                report: ReportLocation::File("report.xml".into()),
            },
            // Whole MERN project: student's repo is the base; the hidden tests and the dependency
            // manifest are protected. `bun install` resolves whatever that manifest declares.
            Stack::JavaScript => StackConfig {
                stack,
                merge: MergeMode::WholeProject {
                    protected_paths: vec!["tests".into(), "package.json".into()],
                },
                install: vec!["bun".into(), "install".into()],
                test: vec![
                    "bun".into(),
                    "test".into(),
                    "tests".into(),
                    "--reporter=junit".into(),
                    "--reporter-outfile=report.xml".into(),
                ],
                report: ReportLocation::File("report.xml".into()),
            },
            // Whole Maven project: protect the test sources and the build config.
            Stack::Java => StackConfig {
                stack,
                merge: MergeMode::WholeProject {
                    protected_paths: vec!["src/test".into(), "pom.xml".into()],
                },
                // Resolve every dependency and plugin into ~/.m2 while the network is up, without
                // compiling anything — so a compile error stays a build_error, not an
                // install_error. The pom must declare junit-platform-launcher explicitly, or
                // surefire would try to fetch its provider during the offline test phase.
                install: vec![
                    "mvn".into(),
                    "-B".into(),
                    "-q".into(),
                    "dependency:go-offline".into(),
                ],
                test: vec![
                    "mvn".into(),
                    "-o".into(),
                    "-q".into(),
                    "test".into(),
                ],
                report: ReportLocation::Glob("target/surefire-reports/*.xml".into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every default command must live on a path the sandbox actually mounts: the read-only
    /// rootfs (`/usr`, `/bin`) or the writable `/work` / `$HOME` / `/tmp`. A command referring to
    /// anything else (an un-mounted `/wheels`, say) would fail with "not found" at grade time.
    #[test]
    fn default_commands_only_touch_mounted_paths() {
        let mounted = ["/usr/", "/bin/", "/opt/", "/home/", "/work", "/tmp"];
        for stack in [Stack::Python, Stack::JavaScript, Stack::Java] {
            let config = StackConfig::default_for(stack);
            for word in config.install.iter().chain(config.test.iter()) {
                for abs in word.split_whitespace().filter(|w| w.starts_with('/')) {
                    assert!(
                        mounted.iter().any(|m| abs.starts_with(m)),
                        "{stack:?}: `{abs}` is not under a path the sandbox mounts"
                    );
                }
            }
        }
    }

    /// Dependencies are resolved per submission, at grade time — nothing is pre-installed in the
    /// image — so every stack must actually have an install step to run with the network up.
    #[test]
    fn every_stack_installs_its_dependencies_at_grade_time() {
        for stack in [Stack::Python, Stack::JavaScript, Stack::Java] {
            assert!(
                !StackConfig::default_for(stack).install.is_empty(),
                "{stack:?}: no install command — dependencies would never be fetched"
            );
        }
    }

    #[test]
    fn defaults_round_trip_through_the_grader_json_shape() {
        for stack in [Stack::Python, Stack::JavaScript, Stack::Java] {
            let config = StackConfig::default_for(stack);
            let json = serde_json::to_string(&config).unwrap();
            assert_eq!(serde_json::from_str::<StackConfig>(&json).unwrap(), config);
        }
    }
}
