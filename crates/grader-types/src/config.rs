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

/// Per-stack recipe: how the submission is merged, how to install offline, how to run the
/// tests, and where the JUnit XML ends up. An assignment's grader config may override any of
/// these; [`StackConfig::default_for`] provides sane defaults.
///
/// The pipeline always runs *its own* `test` command, never the student's scripts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct StackConfig {
    pub stack: Stack,
    /// How to combine the student's submission with the template.
    pub merge: MergeMode,
    /// Offline dependency install command (argv). Empty = nothing to install.
    pub install: Vec<String>,
    /// Test command (argv) that must emit JUnit XML.
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
                install: vec![
                    "pip".into(),
                    "install".into(),
                    "--no-index".into(),
                    "--find-links=/wheels".into(),
                    "-r".into(),
                    "requirements.txt".into(),
                ],
                test: vec!["pytest".into(), "--junitxml=report.xml".into(), "-q".into()],
                report: ReportLocation::File("report.xml".into()),
            },
            // Whole MERN project: student's repo is the base; the hidden tests are protected.
            Stack::JavaScript => StackConfig {
                stack,
                merge: MergeMode::WholeProject {
                    protected_paths: vec!["tests".into()],
                },
                // Offline: deps resolved from bun's global cache baked into the rootfs.
                install: vec!["bun".into(), "install".into(), "--frozen-lockfile".into()],
                test: vec![
                    "bun".into(),
                    "test".into(),
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
                // Maven resolves from the baked ~/.m2 repository; no separate install step.
                install: vec![],
                test: vec!["mvn".into(), "-o".into(), "-q".into(), "test".into()],
                report: ReportLocation::Glob("target/surefire-reports/*.xml".into()),
            },
        }
    }
}
