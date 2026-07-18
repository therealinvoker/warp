use super::*;

fn temp_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
    dir
}

#[test]
fn appends_entry_creates_file_and_agents_reference() {
    let repo = temp_repo();
    let root = repo.path();

    let path = append_memory_entry(root, "Use tabs, not spaces.").expect("append");
    assert_eq!(path, memory_file_path(root));

    let memory = std::fs::read_to_string(&path).expect("read memory");
    assert!(memory.contains("# Project memory"));
    assert!(memory.contains("- Use tabs, not spaces. _(added"));

    let agents = std::fs::read_to_string(root.join("AGENTS.md")).expect("read agents");
    assert!(agents.contains(AGENTS_REFERENCE));
}

#[test]
fn second_entry_appends_and_does_not_duplicate_agents_reference() {
    let repo = temp_repo();
    let root = repo.path();

    append_memory_entry(root, "First fact.").expect("append 1");
    append_memory_entry(root, "Second fact.").expect("append 2");

    let memory = std::fs::read_to_string(memory_file_path(root)).expect("read memory");
    assert!(memory.contains("- First fact."));
    assert!(memory.contains("- Second fact."));

    let agents = std::fs::read_to_string(root.join("AGENTS.md")).expect("read agents");
    assert_eq!(agents.matches(AGENTS_REFERENCE).count(), 1);
}

#[test]
fn multiline_entry_is_flattened_to_single_bullet() {
    let repo = temp_repo();
    let root = repo.path();

    append_memory_entry(root, "line one\n   line two").expect("append");

    let memory = std::fs::read_to_string(memory_file_path(root)).expect("read memory");
    assert!(memory.contains("- line one line two _(added"));
}

#[test]
fn empty_entry_is_rejected() {
    let repo = temp_repo();
    let result = append_memory_entry(repo.path(), "   ");
    assert!(result.is_err());
}

#[test]
fn repo_root_for_walks_up_to_git_dir() {
    let repo = temp_repo();
    let nested = repo.path().join("a").join("b");
    std::fs::create_dir_all(&nested).expect("create nested");

    let resolved = repo_root_for(&nested);
    // Canonicalize both sides: macOS temp dirs are symlinked (/var -> /private/var).
    assert_eq!(
        std::fs::canonicalize(&resolved).unwrap(),
        std::fs::canonicalize(repo.path()).unwrap()
    );
}

#[test]
fn existing_agents_reference_is_preserved_untouched() {
    let repo = temp_repo();
    let root = repo.path();
    let agents_path = root.join("AGENTS.md");
    std::fs::write(
        &agents_path,
        "# Custom\n\nSee @.bang/memory.md for memory.\n",
    )
    .expect("seed");

    append_memory_entry(root, "A fact.").expect("append");

    let agents = std::fs::read_to_string(&agents_path).expect("read agents");
    assert_eq!(agents.matches(AGENTS_REFERENCE).count(), 1);
    assert!(agents.contains("# Custom"));
}
