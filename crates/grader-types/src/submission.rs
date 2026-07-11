use serde::{Deserialize, Serialize};

use crate::stack::Stack;

/// An incoming grading request — the payload the Kafka worker consumes.
///
/// The worker resolves `assignment_id` to the private template + hidden tests + grader
/// config, fetches `repo_url`, merges, runs, and grades.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Submission {
    pub submission_id: String,
    /// Selects the private template repo, hidden tests, and grader config.
    pub assignment_id: String,
    pub stack: Stack,
    /// Public GitHub URL of the student's solution repository.
    pub repo_url: String,
    /// Optional pinned commit/ref; when absent the default branch head is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}
