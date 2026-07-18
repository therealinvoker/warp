use std::path::Path;

use anyhow::{anyhow, Result};

/// Runs a git command and returns the output as a string.
/// Thin wrapper over [`run_git_command_with_env`] with no `PATH` override.
#[cfg(not(target_family = "wasm"))]
pub async fn run_git_command(repo_path: &Path, args: &[&str]) -> Result<String> {
    run_git_command_with_env(repo_path, args, None).await
}

/// Returns a sane default `PATH` to spawn `git` with **only** when the current
/// process has an empty or absent `PATH`; otherwise `None` (inherit the process
/// PATH unchanged). This guards against launches that leave the app with no
/// `PATH` (a macOS GUI launch not inheriting a login PATH), which would make
/// every bare-name `git` spawn fail with ENOENT. On non-unix targets we never
/// override (the unix system dirs don't apply, and an empty PATH is unlikely).
#[cfg(all(not(target_family = "wasm"), unix))]
fn default_path_if_process_path_empty() -> Option<&'static str> {
    match std::env::var_os("PATH") {
        Some(path) if !path.is_empty() => None,
        _ => Some("/usr/bin:/bin:/usr/sbin:/sbin"),
    }
}

#[cfg(all(not(target_family = "wasm"), not(unix)))]
fn default_path_if_process_path_empty() -> Option<&'static str> {
    None
}

/// Like [`run_git_command`] but sets `PATH` on the child when `path_env` is
/// `Some`. Used by callers whose hooks need user-installed binaries (e.g.
/// the LFS `pre-push` hook → `git-lfs`). See `specs/APP-4188/TECH.md`.
#[cfg(not(target_family = "wasm"))]
pub async fn run_git_command_with_env(
    repo_path: &Path,
    args: &[&str],
    path_env: Option<&str>,
) -> Result<String> {
    use command::r#async::Command;
    use command::Stdio;

    log::debug!(
        "[GIT OPERATION] git.rs run_git_command git {}",
        args.join(" ")
    );
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("diff.autoRefreshIndex=false")
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .kill_on_drop(true);
    // `git` is spawned by bare name, so it's resolved via the child's PATH. An
    // explicit `path_env` override always wins. Otherwise, if the process was
    // launched with an empty/absent PATH (e.g. a macOS GUI launch that didn't
    // inherit a login PATH), a bare-name spawn fails with ENOENT ("No such file
    // or directory (os error 2)") even though git is installed — so fall back to
    // the standard system directories rather than inherit an unusable PATH.
    match path_env {
        Some(path_env) => {
            cmd.env("PATH", path_env);
        }
        None => {
            if let Some(fallback) = default_path_if_process_path_empty() {
                cmd.env("PATH", fallback);
            }
        }
    }
    let output = cmd.output().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "Failed to execute git command: `git` could not be found. Is git installed and on \
                 PATH? (process PATH is {:?}). Underlying error: {e}",
                std::env::var("PATH").unwrap_or_default(),
            )
        } else {
            anyhow!("Failed to execute git command: {e}")
        }
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Handle git diff specific behavior:
    // - Exit code 0: no differences
    // - Exit code 1: differences found (this is normal for diff commands)
    // - Exit code > 1: actual error
    if output.status.success() || (output.status.code() == Some(1) && !stdout.is_empty()) {
        Ok(stdout)
    } else {
        Err(anyhow!("Git command failed: {}, {}", stderr, stdout))
    }
}

#[cfg(target_family = "wasm")]
pub async fn run_git_command(_repo_path: &Path, _args: &[&str]) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}

#[cfg(target_family = "wasm")]
pub async fn run_git_command_with_env(
    _repo_path: &Path,
    _args: &[&str],
    _path_env: Option<&str>,
) -> Result<String> {
    Err(anyhow!("Not supported on wasm"))
}
