use task_types::consumer::TaskLanguage;

/// One shell command as it will be run *inside* the sandbox. Paths here are
/// sandbox-absolute (`/project`, `/out`, `/cache`), not host paths.
#[derive(Debug, Clone)]
pub struct RunCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl RunCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        RunCommand {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }
}

/// Where JUnit XML lands after the test command runs.
#[derive(Debug, Clone)]
pub enum ReportLocation {
    /// A single file the runner was told to write.
    File(&'static str),
    /// Every `*.xml` the runner dropped in this directory (Maven Surefire).
    Dir(&'static str),
}

#[derive(Debug, Clone)]
pub struct LanguageSpec {
    /// Paths inside the student checkout that are wiped before the hidden test
    /// repo is overlaid. This is what makes the hidden suite authoritative: the
    /// student cannot keep a doctored copy of the tests around.
    pub purge_paths: &'static [&'static str],
    /// Dependency fetch / compile. Failures here are `InstallFailed`.
    pub install: RunCommand,
    /// The suite itself. A non-zero exit is expected when tests fail, so the
    /// exit code is *not* what we grade on — the JUnit report is.
    pub test: RunCommand,
    pub report: ReportLocation,
    /// Extra env, applied on top of the sandbox base env.
    pub env: Vec<(String, String)>,
}

/// `cache_dir` is the sandbox-side mount point of the shared package cache, or
/// `None` when each task gets a cold cache.
pub fn spec_for(language: TaskLanguage, cache_dir: Option<&str>) -> LanguageSpec {
    match language {
        // Bun: `bun install` understands package.json, and `bun test` speaks the
        // jest API the templates are written against while emitting JUnit XML
        // natively — no extra reporter dependency the student could remove.
        TaskLanguage::Javascript => {
            let mut env = vec![("BUN_INSTALL".to_string(), "/home/.bun".to_string())];
            if let Some(cache) = cache_dir {
                env.push(("BUN_INSTALL_CACHE_DIR".to_string(), format!("{cache}/bun")));
            }
            LanguageSpec {
                purge_paths: &["tests", "test", "__tests__"],
                install: RunCommand::new("/bin/sh", &["-c", "bun install --no-progress"]),
                test: RunCommand::new(
                    "/bin/sh",
                    &["-c", "bun test --reporter=junit --reporter-outfile=/out/junit.xml"],
                ),
                report: ReportLocation::File("/out/junit.xml"),
                env,
            }
        }

        // pip must not try to write into the read-only /usr bind, hence --user
        // plus a PYTHONUSERBASE inside the writable home.
        TaskLanguage::Py => {
            let mut env = vec![
                ("PYTHONUSERBASE".to_string(), "/home/.local".to_string()),
                ("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string()),
            ];
            if let Some(cache) = cache_dir {
                env.push(("PIP_CACHE_DIR".to_string(), format!("{cache}/pip")));
            }
            LanguageSpec {
                purge_paths: &["tests", "test"],
                install: RunCommand::new(
                    "/bin/sh",
                    &[
                        "-c",
                        "if [ -f requirements.txt ]; then python3 -m pip install --user \
                         --no-input --disable-pip-version-check -r requirements.txt; \
                         else echo 'no requirements.txt, skipping install'; fi",
                    ],
                ),
                test: RunCommand::new(
                    "/bin/sh",
                    &["-c", "python3 -m pytest -q --junitxml=/out/junit.xml"],
                ),
                report: ReportLocation::File("/out/junit.xml"),
                env,
            }
        }

        // Maven writes JUnit XML to target/surefire-reports by default.
        // -Dmaven.test.failure.ignore keeps a failing assertion from aborting
        // the build before the reports are flushed.
        TaskLanguage::Java => {
            let repo = cache_dir
                .map(|c| format!("{c}/m2"))
                .unwrap_or_else(|| "/home/.m2/repository".to_string());
            // The sandbox clears the environment, so the JDK the base image put
            // on PATH/JAVA_HOME is gone by the time mvn runs. Take the worker's
            // own JAVA_HOME (the runtime image sets it) and hand it back in.
            let java_home = std::env::var("JAVA_HOME")
                .unwrap_or_else(|_| "/opt/java/openjdk".to_string());
            let mvn = |goal: &str| {
                format!(
                    "mvn -B --no-transfer-progress -Dmaven.repo.local={repo} -Duser.home=/home {goal}"
                )
            };
            LanguageSpec {
                purge_paths: &["src/test"],
                install: RunCommand::new("/bin/sh", &["-c", &mvn("-DskipTests test-compile")]),
                test: RunCommand::new(
                    "/bin/sh",
                    &["-c", &mvn("test -Dmaven.test.failure.ignore=true")],
                ),
                report: ReportLocation::Dir("/project/target/surefire-reports"),
                env: vec![
                    ("MAVEN_OPTS".to_string(), "-Xmx1g".to_string()),
                    ("JAVA_HOME".to_string(), java_home.clone()),
                    (
                        "PATH".to_string(),
                        format!(
                            "{java_home}/bin:/usr/local/sbin:/usr/local/bin:\
                             /usr/sbin:/usr/bin:/sbin:/bin"
                        ),
                    ),
                ],
            }
        }
    }
}
