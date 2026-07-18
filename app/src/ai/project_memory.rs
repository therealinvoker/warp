//! Local per-repo project memory (`<repo>/.bang/memory.md`).
//!
//! This is the on-disk half of Bang's project memory: a plain-markdown file
//! that the standing-query discovery in `repo_metadata` picks up and injects as
//! a project rule on every request (see
//! `crates/ai/src/project_context/model.rs`). This module owns the *writes* —
//! appending user-approved entries and keeping an `@.bang/memory.md` reference
//! line in the repo's `AGENTS.md` so humans and other tools discover it.

use std::path::{Path, PathBuf};
use std::{fs, io};

/// Directory (relative to a repo root) that holds Bang's per-repo state.
pub const MEMORY_DIR: &str = ".bang";
/// File name of the auto-discovered per-repo memory file.
pub const MEMORY_FILE: &str = "memory.md";
/// Reference line kept in `AGENTS.md` so the memory file is discoverable.
const AGENTS_REFERENCE: &str = "@.bang/memory.md";

const MEMORY_HEADER: &str = "# Project memory\n\n\
Durable, project-specific facts and preferences remembered by Bang. Managed via \
`/remember` and the agent's memory suggestions.\n";

/// Absolute path of the memory file for a repo root.
pub fn memory_file_path(repo_root: &Path) -> PathBuf {
    repo_root.join(MEMORY_DIR).join(MEMORY_FILE)
}

/// Resolves the enclosing git repository root for `start`, falling back to
/// `start` itself when no `.git` directory is found in its ancestors.
pub fn repo_root_for(start: &Path) -> PathBuf {
    start
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start.to_path_buf())
}

/// Appends a memory entry to `<repo_root>/.bang/memory.md`, creating the `.bang`
/// directory and the file (with a header) as needed, and ensures `AGENTS.md`
/// references the memory file. Returns the memory file path on success.
pub fn append_memory_entry(repo_root: &Path, text: &str) -> io::Result<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot remember an empty entry",
        ));
    }

    fs::create_dir_all(repo_root.join(MEMORY_DIR))?;
    let path = memory_file_path(repo_root);

    let mut contents = fs::read_to_string(&path).unwrap_or_default();
    if contents.is_empty() {
        contents.push_str(MEMORY_HEADER);
    }
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    // Normalize a possibly multi-line entry into a single markdown bullet.
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let date = chrono::Local::now().format("%Y-%m-%d");
    contents.push_str(&format!("- {one_line} _(added {date})_\n"));
    fs::write(&path, contents)?;

    // Best-effort: a failure to touch AGENTS.md shouldn't lose the memory.
    if let Err(error) = ensure_agents_reference(repo_root) {
        log::warn!("Failed to add memory reference to AGENTS.md: {error}");
    }
    Ok(path)
}

/// Ensures the repo-root `AGENTS.md` contains a reference line pointing at the
/// memory file. Creates `AGENTS.md` if it does not exist. No-op when a
/// reference is already present.
pub fn ensure_agents_reference(repo_root: &Path) -> io::Result<()> {
    let path = repo_root.join("AGENTS.md");
    let mut contents = fs::read_to_string(&path).unwrap_or_default();
    if contents.contains(AGENTS_REFERENCE) {
        return Ok(());
    }
    if contents.is_empty() {
        contents.push_str("# AGENTS.md\n");
    }
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(&format!(
        "\n<!-- bang:memory --> Project memory (auto-managed by Bang): {AGENTS_REFERENCE}\n"
    ));
    fs::write(&path, contents)
}

#[cfg(test)]
#[path = "project_memory_tests.rs"]
mod tests;
