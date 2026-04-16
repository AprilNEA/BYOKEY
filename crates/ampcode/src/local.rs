//! Read Amp CLI thread files from `~/.local/share/amp/threads/` without
//! network access.
//!
//! ```rust,no_run
//! # async fn example() -> ampcode::error::Result<()> {
//! let summaries = ampcode::local::list_thread_summaries().await?;
//! println!("{} threads", summaries.len());
//! # Ok(())
//! # }
//! ```

use crate::error::{AmpcodeError, Result};
use crate::types::thread::{Thread, ThreadSummary};
use std::io::BufReader;
use std::path::{Path, PathBuf};

// ── Directory resolution ──────────────────────────────────────────────────────

/// Resolve the Amp threads directory.
///
/// Uses `~/.local/share/amp/threads/` on both macOS and Linux.
#[must_use]
pub fn threads_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/share/amp/threads")
}

/// Validate a thread ID to prevent path traversal.
///
/// Valid IDs match `T-` followed by hex digits and hyphens (UUID format).
#[must_use]
pub fn is_valid_thread_id(id: &str) -> bool {
    id.starts_with("T-")
        && id.len() > 2
        && id[2..].chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

// ── Single-thread operations ──────────────────────────────────────────────────

/// Read and parse a full thread from the given file path.
///
/// # Errors
///
/// Returns [`AmpcodeError::Io`] if the file cannot be opened, or
/// [`AmpcodeError::Json`] if the content is not valid thread JSON.
pub fn read_thread(path: &Path) -> Result<Thread> {
    let file = std::fs::File::open(path)?;
    let thread = serde_json::from_reader(BufReader::new(file))?;
    Ok(thread)
}

/// Read a thread by its ID from the standard threads directory.
///
/// # Errors
///
/// Returns [`AmpcodeError::Io`] if the thread file does not exist.
pub fn read_thread_by_id(id: &str) -> Result<Thread> {
    let path = threads_dir().join(format!("{id}.json"));
    read_thread(&path)
}

/// Read a thread summary (metadata only) from a file path.
///
/// Returns `None` on any parse or I/O error — intended for bulk scanning
/// where individual failures are silently skipped.
#[must_use]
pub fn read_summary(path: &Path) -> Option<ThreadSummary> {
    let file = std::fs::File::open(path).ok()?;
    serde_json::from_reader(BufReader::new(file)).ok()
}

// ── Directory scan ────────────────────────────────────────────────────────────

/// Scan the threads directory and return summaries of all threads,
/// sorted by `created` descending (newest first).
///
/// Unparseable files are silently skipped.
///
/// # Errors
///
/// Returns `Err` only if the OS refuses to read the directory itself.
/// A non-existent directory yields an empty `Vec`.
pub async fn list_thread_summaries() -> Result<Vec<ThreadSummary>> {
    let dir = threads_dir();
    tokio::task::spawn_blocking(move || scan_summaries_sync(&dir))
        .await
        .map_err(|e| AmpcodeError::Io(std::io::Error::other(e.to_string())))
}

/// Synchronous directory scan — usable from blocking contexts.
#[must_use]
pub fn scan_summaries_sync(dir: &Path) -> Vec<ThreadSummary> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut summaries: Vec<ThreadSummary> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("T-")
                || !Path::new(&name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                return None;
            }
            read_summary(&entry.path())
        })
        .collect();
    summaries.sort_unstable_by(|a, b| b.created.cmp(&a.created));
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_thread_ids() {
        assert!(is_valid_thread_id("T-019d38dd-45f9-7617-8e7f-03b730ba197a"));
        assert!(is_valid_thread_id("T-abcdef0123456789"));
        assert!(is_valid_thread_id("T-fc68e9f5-9621-4ee2-b8d9-d954ba656de4"));
    }

    #[test]
    fn invalid_thread_ids() {
        assert!(!is_valid_thread_id(""));
        assert!(!is_valid_thread_id("T-"));
        assert!(!is_valid_thread_id("../etc/passwd"));
        assert!(!is_valid_thread_id("T-../../foo"));
        assert!(!is_valid_thread_id("T-abc def"));
        assert!(!is_valid_thread_id("not-a-thread"));
    }

    #[test]
    fn threads_dir_returns_sensible_path() {
        let dir = threads_dir();
        assert!(dir.ends_with(".local/share/amp/threads"));
    }

    #[test]
    fn scan_nonexistent_dir() {
        let summaries = scan_summaries_sync(Path::new("/tmp/nonexistent_ampcode_test_dir"));
        assert!(summaries.is_empty());
    }
}
