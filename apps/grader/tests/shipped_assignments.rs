//! The shipped assignments must be loadable by the worker, and their `grader.json` must agree
//! with what is actually in the template. An assignment that fails these checks would only show
//! up in production as an `internal_error` on a student's submission.

use std::path::{Path, PathBuf};

use grader_engine::{AssignmentStore, FsAssignmentStore};
use grader_types::{MergeMode, ReportLocation};

fn assignments_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assignments")
}

/// Every `<assignments>/<id>` directory, by id.
fn assignment_ids() -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(assignments_root())
        .expect("assignments dir")
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    assert!(!ids.is_empty(), "no assignments found");
    ids
}

#[test]
fn every_assignment_resolves_and_declares_paths_that_exist() {
    let store = FsAssignmentStore::new(assignments_root());

    for id in assignment_ids() {
        let assignment = store.resolve(&id).unwrap_or_else(|e| panic!("{id}: {e}"));
        let template = &assignment.template_dir;

        match &assignment.config.merge {
            // The student overwrites these, so the template must carry a stub of each: it is what
            // documents the signature the hidden tests call.
            MergeMode::SolutionFiles { files } => {
                assert!(!files.is_empty(), "{id}: no solution files declared");
                for file in files {
                    assert!(
                        template.join(file).is_file(),
                        "{id}: template has no stub for solution file `{file}`"
                    );
                }
            }
            // merge() refuses to run when a protected path is missing from the template.
            MergeMode::WholeProject { protected_paths } => {
                assert!(!protected_paths.is_empty(), "{id}: nothing is protected");
                for path in protected_paths {
                    assert!(
                        template.join(path).exists(),
                        "{id}: protected path `{path}` is missing from the template"
                    );
                }
            }
        }

        assert!(
            !assignment.config.test.is_empty(),
            "{id}: no test command — nothing would ever produce a report"
        );

        // A relative report path is resolved against the merged workspace, so an absolute one
        // (or one climbing out of it) would read from outside the sandbox's /work.
        let report = match &assignment.config.report {
            ReportLocation::File(p) | ReportLocation::Glob(p) => p,
        };
        assert!(
            !report.starts_with('/') && !report.contains(".."),
            "{id}: report location `{report}` must stay inside the workspace"
        );
    }
}

/// Each assignment needs a reference solution to smoke-test against (and, for Java, to warm
/// Maven's cache during the image build) plus a broken one to prove failures are reported.
#[test]
fn every_assignment_ships_good_and_bad_examples() {
    let students = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/students");

    for id in assignment_ids() {
        for variant in ["good", "bad", "starter"] {
            let example = students.join(format!("{id}-{variant}"));
            assert!(
                example.is_dir(),
                "{id}: missing example submission {}",
                example.display()
            );
        }
    }
}
