//! Shared vocabulary for the grader pipeline.
//!
//! These types are the contract between every stage: the Kafka worker deserializes a
//! [`Submission`], the sandbox runs the stack's commands, [`report::TestReport`] normalizes
//! the JUnit XML the test runner produced, and a [`GradeResult`] is serialized back.
//!
//! This crate is intentionally dependency-light (serde only) and side-effect free so it can
//! be unit-tested and shared without pulling in IO, Kafka, or the sandbox.

mod config;
mod report;
mod result;
mod stack;
mod submission;

pub use config::{ReportLocation, StackConfig};
pub use report::{CaseStatus, TestCase, TestReport};
pub use result::{GradeResult, GradeStatus};
pub use stack::Stack;
pub use submission::Submission;
