//! Orchestrates the grading pipeline: resolve assignment → fetch student repo → merge →
//! run install+test in the sandbox → parse the JUnit report → produce a [`grader_types::GradeResult`].
//!
//! The [`Engine`](pipeline::Engine) is generic over three [`ports`], so the whole flow is
//! unit-tested against fakes. [`adapters`] provides the real filesystem/HTTP/sandbox wirings.

mod adapters;
mod pipeline;
mod ports;

pub use adapters::{FsAssignmentStore, HttpRepoFetcher, SandboxProjectRunner};
pub use pipeline::Engine;
pub use ports::{
    Assignment, AssignmentStore, EngineError, Phase, ProjectRunner, RepoFetcher, RunOutcome,
};
