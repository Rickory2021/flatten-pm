// src-tauri/src/error.rs
//
// Tauri command error type. Maps flatten-core errors to the design doc's
// error model (docs/design/1_INFRASTRUCTURE.md) and serializes to the
// frontend as {"error": "...", "kind": "..."}.

use serde::Serialize;

/// Error type for Tauri commands. Serialized across IPC as JSON with
/// an `error` message and a `kind` discriminator.
///
/// Maps to the error model categories: `domain`, `database`, `io`,
/// `transform`, `usage`. Phase 3 uses `domain` and `database` only.
#[derive(Debug, Serialize)]
pub struct CommandError {
    /// Human-readable error message.
    pub error: String,
    /// Error category per the error model in `1_INFRASTRUCTURE.md`.
    pub kind: &'static str,
}

impl From<flatten_core::db::error::Error> for CommandError {
    fn from(e: flatten_core::db::error::Error) -> Self {
        CommandError {
            error: e.to_string(),
            kind: "database",
        }
    }
}

impl From<rusqlite::Error> for CommandError {
    fn from(e: rusqlite::Error) -> Self {
        CommandError {
            error: e.to_string(),
            kind: "database",
        }
    }
}

impl From<flatten_core::ingest::error::Error> for CommandError {
    fn from(e: flatten_core::ingest::error::Error) -> Self {
        use flatten_core::ingest::error::Error as IE;
        match &e {
            IE::NonExistentPath { .. }
            | IE::DuplicateName { .. }
            | IE::RepoNotFound(_)
            | IE::InvalidPattern { .. }
            | IE::InvalidLineEndingPolicy { .. } => CommandError {
                error: e.to_string(),
                kind: "domain",
            },
            IE::Io { .. } => CommandError {
                error: e.to_string(),
                kind: "io",
            },
            IE::Json(_) | IE::Trie(_) | IE::Database(_) => CommandError {
                error: e.to_string(),
                kind: "database",
            },
        }
    }
}

impl CommandError {
    /// Create a domain error (bad input, unknown key, validation failure).
    pub fn domain(msg: impl Into<String>) -> Self {
        CommandError {
            error: msg.into(),
            kind: "domain",
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Wire shape test: CommandError serializes to {"error": ..., "kind": ...}.
    /// This is the contract the frontend's isCommandError() guard depends on.
    #[test]
    fn command_error_serializes_correctly() {
        let err = CommandError::domain("test message");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["error"], "test message", "error field should match");
        assert_eq!(json["kind"], "domain", "kind field should be domain");
        // Confirm no extra fields
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 2, "should have exactly two fields: error and kind");
    }

    /// From<db::error::Error> maps to kind "database" with the source message.
    #[test]
    fn command_error_from_db_error() {
        let db_err = flatten_core::db::error::Error::Writer("test writer error".into());
        let cmd_err = CommandError::from(db_err);
        assert_eq!(cmd_err.kind, "database", "db errors should map to database kind");
        assert!(
            cmd_err.error.contains("test writer error"),
            "error message should contain the source message"
        );
    }
}
