use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPayload {
    pub task_id: String,
    pub student_id: String,
    pub task_language: TaskLanguage,
    /// Git URL of the student's checkout of the assignment template.
    pub student_link: String,
    /// Git URL of the hidden test repo for this assignment.
    pub test_link: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskLanguage {
    #[serde(alias = "js", alias = "node", alias = "nodejs")]
    Javascript,
    #[serde(alias = "python", alias = "python3")]
    Py,
    Java,
}

impl TaskLanguage {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskLanguage::Javascript => "javascript",
            TaskLanguage::Py => "py",
            TaskLanguage::Java => "java",
        }
    }
}
