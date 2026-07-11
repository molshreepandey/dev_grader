use serde::{Deserialize, Serialize};

/// The tech stack an assignment targets. Drives which template layout, install command,
/// test command, and baked rootfs the pipeline uses.
///
/// MERN/JavaScript assignments run on **Bun** (`bun install` + `bun test`) — it is fast,
/// Jest-compatible, and can emit JUnit XML directly, so every stack converges on a single
/// report format.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Stack {
    Python,
    /// MERN / Node, executed with Bun.
    #[serde(alias = "js", alias = "mern", alias = "node")]
    JavaScript,
    Java,
}

impl Stack {
    /// Stable lowercase identifier, e.g. used to pick the baked rootfs directory.
    pub fn as_str(self) -> &'static str {
        match self {
            Stack::Python => "python",
            Stack::JavaScript => "javascript",
            Stack::Java => "java",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_canonical_and_mern_aliases() {
        let cases = [
            (r#""python""#, Stack::Python),
            (r#""javascript""#, Stack::JavaScript),
            (r#""js""#, Stack::JavaScript),
            (r#""mern""#, Stack::JavaScript),
            (r#""node""#, Stack::JavaScript),
            (r#""java""#, Stack::Java),
        ];
        for (json, expected) in cases {
            let got: Stack = serde_json::from_str(json).unwrap();
            assert_eq!(got, expected, "deserializing {json}");
        }
    }

    #[test]
    fn serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&Stack::JavaScript).unwrap(),
            r#""javascript""#
        );
    }
}
