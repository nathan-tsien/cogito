//! Load-time errors for `FsStrategyRegistry`. These surface from
//! `from_roots` / `from_conventional_scopes` and are strictly richer
//! than `cogito_protocol::StrategyError`, which surfaces from `get`/`list`.

use std::path::PathBuf;

use thiserror::Error;

/// Registry-build error. Any variant is fatal at startup — operators
/// learn about broken strategies immediately, not at session-open.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// I/O error reading a strategy file or its referenced prompt file.
    #[error("I/O error reading {path}: {source}")]
    Io {
        /// File path that triggered the error.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// YAML frontmatter failed to deserialize.
    #[error("parse error in {path}: {source}")]
    Parse {
        /// File path that triggered the error.
        path: PathBuf,
        /// Underlying serde error.
        #[source]
        source: serde_yaml::Error,
    },

    /// File has no `---`-delimited frontmatter block.
    #[error("frontmatter missing or malformed in {path}")]
    Frontmatter {
        /// File path that triggered the error.
        path: PathBuf,
    },

    /// Two strategy files in the same scope declare the same `name`.
    #[error("duplicate strategy name `{name}` in scope: {files:?}")]
    DuplicateName {
        /// Conflicting strategy name.
        name: String,
        /// All files in the same scope that declare the name.
        files: Vec<PathBuf>,
    },

    /// Filename basename does not match the `name` field in frontmatter.
    #[error("filename / name mismatch: {path} declares `name: {declared}`")]
    NameMismatch {
        /// Strategy file path.
        path: PathBuf,
        /// `name` field as declared in frontmatter.
        declared: String,
    },

    /// Both body and frontmatter `system_prompt` are empty.
    #[error("strategy {name} has empty system_prompt (body and frontmatter both empty)")]
    EmptyPrompt {
        /// Strategy name.
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_renders_each_variant() {
        // Sanity check that thiserror formatting compiles for every variant.
        let cases: Vec<LoadError> = vec![
            LoadError::Frontmatter {
                path: PathBuf::from("foo.md"),
            },
            LoadError::DuplicateName {
                name: "coder".into(),
                files: vec![PathBuf::from("a.md"), PathBuf::from("b.md")],
            },
            LoadError::NameMismatch {
                path: PathBuf::from("coder.md"),
                declared: "planner".into(),
            },
            LoadError::EmptyPrompt {
                name: "coder".into(),
            },
        ];
        for case in cases {
            assert!(!format!("{case}").is_empty());
        }
    }
}
