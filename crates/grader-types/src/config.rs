use serde::{Deserialize, Serialize};

use crate::stack::Stack;

/// Where the JUnit XML report lands after the test command runs.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReportLocation {
    /// A single report file at this workspace-relative path.
    File(String),
    /// A glob matching one-or-more report files (surefire writes one per class).
    Glob(String),
}

/// Per-stack recipe: what to pull from the student, how to install offline, how to run the
/// tests, and where the JUnit XML ends up. An assignment's grader config may override any of
/// these; [`StackConfig::default_for`] provides sane defaults.
///
/// Anti-cheat model: only `solution_files` are copied out of the student's repo into a fresh
/// copy of the template. Everything else — tests, lockfiles, config — comes from the template,
/// and the pipeline runs *its own* `test` command, never the student's scripts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StackConfig {
    pub stack: Stack,
    /// Repo-relative paths copied FROM the student INTO the template.
    pub solution_files: Vec<String>,
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
            Stack::Python => StackConfig {
                stack,
                solution_files: vec!["src/solution.py".into()],
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
            Stack::JavaScript => StackConfig {
                stack,
                solution_files: vec!["src/solution.js".into()],
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
            Stack::Java => StackConfig {
                stack,
                solution_files: vec!["src/main/java/Solution.java".into()],
                // Maven resolves from the baked ~/.m2 repository; no separate install step.
                install: vec![],
                test: vec!["mvn".into(), "-o".into(), "-q".into(), "test".into()],
                report: ReportLocation::Glob("target/surefire-reports/*.xml".into()),
            },
        }
    }
}
