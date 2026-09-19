// src-tauri/src/commands/settings.rs
//
// Settings commands: list all settings, update a setting with validation.
// Settings contract: docs/design/1_INFRASTRUCTURE.md.

use crate::error::CommandError;
use crate::state::AppState;

/// A single setting row (key-value pair).
#[derive(Debug, serde::Serialize)]
pub struct Setting {
    /// Setting key (e.g. `watch_poll_interval_ms`).
    pub key: String,
    /// Setting value as a string.
    pub value: String,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Setting value types for validation.
enum SettingType {
    /// Absolute path to an existing directory, or empty (unset).
    Path,
    /// Integer greater than zero.
    PositiveInt,
    /// Comma-separated list (any string accepted).
    CommaSeparatedList,
}

/// Known settings with their expected value types. Sorted alphabetically.
/// Matches the 9 keys seeded in `flatten-core/src/db/seed.rs`.
const VALID_SETTINGS: &[(&str, SettingType)] = &[
    ("binary_extensions", SettingType::CommaSeparatedList),
    ("copy_size_limit_mb", SettingType::PositiveInt),
    ("transform_memory_mb", SettingType::PositiveInt),
    ("transform_timeout_ms", SettingType::PositiveInt),
    ("trie_refresh_interval_ms", SettingType::PositiveInt),
    ("watch_debounce_ms", SettingType::PositiveInt),
    ("watch_poll_interval_ms", SettingType::PositiveInt),
    ("watch_settle_ms", SettingType::PositiveInt),
    ("watch_source_dir", SettingType::Path),
];

/// Validate a setting key and value against the settings contract.
/// Returns `CommandError::domain` on unknown key or invalid value.
fn validate_setting(key: &str, value: &str) -> Result<(), CommandError> {
    let (_, stype) = VALID_SETTINGS
        .iter()
        .find(|(k, _)| *k == key)
        .ok_or_else(|| CommandError::domain(format!("unknown setting: {key}")))?;

    match stype {
        SettingType::Path => {
            // Empty string is valid (means unset).
            // Non-empty must be an absolute path pointing to an existing directory.
            // Matches CLI-001 verification: "settings set watch_source_dir
            // /nonexistent exits 1 with a path error".
            if !value.is_empty() {
                let path = std::path::Path::new(value);
                if !path.is_absolute() {
                    return Err(CommandError::domain(format!(
                        "path must be absolute: {value}"
                    )));
                }
                if !path.is_dir() {
                    return Err(CommandError::domain(format!(
                        "path does not exist or is not a directory: {value}"
                    )));
                }
            }
            Ok(())
        }
        SettingType::PositiveInt => {
            let n = value.parse::<u64>().map_err(|_| {
                CommandError::domain(format!(
                    "{key} must be a positive integer, got: {value}"
                ))
            })?;
            if n == 0 {
                return Err(CommandError::domain(format!(
                    "{key} must be greater than zero"
                )));
            }
            Ok(())
        }
        SettingType::CommaSeparatedList => {
            // Any string is valid; the list may be empty.
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all settings with their current values.
///
/// Uses a per-call reader connection (per ADR-038, no connection pool).
#[tauri::command]
pub async fn settings_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Setting>, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    let mut stmt = conn.prepare("SELECT key, value FROM settings ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        Ok(Setting {
            key: row.get(0)?,
            value: row.get(1)?,
        })
    })?;
    let settings: Vec<Setting> = rows.collect::<std::result::Result<_, _>>()?;
    Ok(settings)
}

/// Update a setting value. Validates the key and value before writing.
/// Sets `updated_at` to match the CLI's `cmd_settings_set` behavior.
///
/// Uses the Writer channel (per ADR-038, single writer thread).
#[tauri::command]
pub async fn settings_set(
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    validate_setting(&key, &value)?;

    state
        .writer
        .call_write(move |conn| {
            let updated = conn.execute(
                "UPDATE settings SET value = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE key = ?2",
                rusqlite::params![value, key],
            )?;
            if updated == 0 {
                return Err(flatten_core::db::error::Error::Writer(format!(
                    "setting not found: {key}"
                )));
            }
            Ok(())
        })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_unknown_key() {
        let result = validate_setting("nonexistent_key", "123");
        assert!(result.is_err(), "unknown key should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "error kind should be domain");
    }

    #[test]
    fn validate_rejects_non_integer() {
        let result = validate_setting("watch_poll_interval_ms", "abc");
        assert!(result.is_err(), "non-integer should be rejected for int setting");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "error kind should be domain");
    }

    #[test]
    fn validate_rejects_zero() {
        let result = validate_setting("watch_poll_interval_ms", "0");
        assert!(result.is_err(), "zero should be rejected for positive int");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "error kind should be domain");
    }

    #[test]
    fn validate_accepts_positive_int() {
        let result = validate_setting("watch_poll_interval_ms", "5000");
        assert!(result.is_ok(), "valid positive integer should be accepted");
    }

    #[test]
    fn validate_rejects_missing_dir() {
        let missing = std::env::temp_dir().join("flatten-does-not-exist");
        let path_str = missing.to_str().expect("temp path should be valid UTF-8");
        let result = validate_setting("watch_source_dir", path_str);
        assert!(result.is_err(), "non-existent directory should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "error kind should be domain");
    }

    #[test]
    fn validate_accepts_empty_path() {
        let result = validate_setting("watch_source_dir", "");
        assert!(result.is_ok(), "empty string should be accepted (means unset)");
    }

    #[test]
    fn validate_accepts_existing_dir() {
        let tmp = std::env::temp_dir();
        let path_str = tmp.to_str().expect("temp path should be valid UTF-8");
        let result = validate_setting("watch_source_dir", path_str);
        assert!(result.is_ok(), "existing directory should be accepted");
    }

    #[test]
    fn validate_rejects_relative_path() {
        let result = validate_setting("watch_source_dir", "Downloads");
        assert!(result.is_err(), "relative path should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "error kind should be domain");
    }

    #[test]
    fn validate_accepts_comma_list() {
        let result = validate_setting("binary_extensions", ".png,.jpg,.gif");
        assert!(result.is_ok(), "comma-separated list should be accepted");
    }
}
