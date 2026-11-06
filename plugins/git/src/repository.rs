//! Reading a repository through the `git` command.
//!
//! The plugin shells out to Git rather than linking `libgit2`: the question this
//! slice answers is whether two independent plugins can cooperate, and `git
//! status`/`git diff` are the real data with the least machinery. A Git library
//! can replace this module without touching the panels, the store or the contract.
//!
//! Every call runs with `-C <root>`, so the plugin never depends on the host's
//! working directory, and output is always read with `--no-color` so a user's
//! `color.ui` setting cannot leak into the interface.

use std::path::{Path, PathBuf};
use std::process::Command;

use brucebox_git_contract::{
    Change, ChangeStatus, MAX_CHANGES, MAX_DIFF_BYTES, RepositorySnapshot,
};

/// Why a repository could not be read.
#[derive(Debug, Clone)]
pub struct GitError(String);

impl GitError {
    /// A message for the interface.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

/// Resolve `path` to the work-tree root of its repository.
pub fn open(path: &Path) -> Result<PathBuf, GitError> {
    let root = run(path, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim();
    if root.is_empty() {
        return Err(GitError(format!(
            "{} is not inside a Git repository",
            path.display()
        )));
    }
    Ok(PathBuf::from(root))
}

/// Read the repository's current state.
pub fn snapshot(root: &Path) -> Result<RepositorySnapshot, GitError> {
    let branch = run(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|value| value.trim().to_owned())
        .ok()
        .filter(|value| !value.is_empty() && value != "HEAD");

    let head = run(root, &["rev-parse", "--short", "HEAD"])
        .map(|value| value.trim().to_owned())
        .ok()
        .filter(|value| !value.is_empty());

    let status = run(root, &["status", "--porcelain=v1", "--untracked-files=all"])?;

    let changes: Vec<Change> = status.lines().filter_map(parse_status_line).collect();
    let truncated = changes.len() > MAX_CHANGES;

    Ok(RepositorySnapshot {
        root: root.display().to_string(),
        branch,
        head,
        changes,
        truncated,
    }
    .truncate())
}

/// The diff for one changed file.
///
/// An untracked file has no diff against the index, so it is compared against
/// nothing — which is exactly what it is. Binary files report that instead of
/// producing noise.
pub fn diff(root: &Path, change: &Change) -> Result<String, GitError> {
    let text = match change.status {
        ChangeStatus::Untracked => run(
            root,
            &[
                "diff",
                "--no-index",
                "--no-color",
                "--",
                "/dev/null",
                &change.path,
            ],
        )
        .unwrap_or_default(),
        _ => run(root, &["diff", "--no-color", "--", &change.path])?,
    };

    let mut text = text;
    if text.len() > MAX_DIFF_BYTES {
        text.truncate(MAX_DIFF_BYTES);
        text.push_str("\n… diff truncated\n");
    }
    Ok(text)
}

/// Parse one `git status --porcelain=v1` line.
fn parse_status_line(line: &str) -> Option<Change> {
    let mut characters = line.chars();
    let index = characters.next()?;
    let working = characters.next()?;
    let rest = &line[index.len_utf8() + working.len_utf8()..];
    let path = rest.trim_start();

    if path.is_empty() || index == '!' && working == '!' {
        return None;
    }

    // A rename reads `old -> new`; the interface cares about where the file is now.
    let path = path
        .split_once(" -> ")
        .map(|(_, new)| new)
        .unwrap_or(path)
        .trim_matches('"');

    let status = match (index, working) {
        ('?', '?') => ChangeStatus::Untracked,
        ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => ChangeStatus::Conflicted,
        ('A', _) => ChangeStatus::Added,
        ('R', _) => ChangeStatus::Renamed,
        ('D', _) | (_, 'D') => ChangeStatus::Deleted,
        ('T', _) | (_, 'T') => ChangeStatus::TypeChanged,
        ('M', _) | (_, 'M') => ChangeStatus::Modified,
        (' ', ' ') => return None,
        _ => ChangeStatus::Unknown,
    };

    Some(Change {
        path: path.to_owned(),
        status,
    })
}

/// Run one Git command in `root` and return its standard output.
fn run(root: &Path, arguments: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| GitError(format!("cannot run `git`: {error}")))?;

    // `git diff --no-index` exits 1 when the files differ, which is the normal
    // case for it; anything else with a non-zero status is a real failure.
    if !output.status.success() && !arguments.contains(&"--no-index") {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(GitError(if detail.is_empty() {
            format!("`git {}` failed", arguments.join(" "))
        } else {
            detail.to_owned()
        }));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_paths_are_read_with_their_change() {
        assert_eq!(
            parse_status_line(" M src/main.rs"),
            Some(Change {
                path: "src/main.rs".to_owned(),
                status: ChangeStatus::Modified
            })
        );
        assert_eq!(
            parse_status_line("?? notes.txt"),
            Some(Change {
                path: "notes.txt".to_owned(),
                status: ChangeStatus::Untracked
            })
        );
        assert_eq!(
            parse_status_line("A  added.rs").map(|change| change.status),
            Some(ChangeStatus::Added)
        );
        assert_eq!(
            parse_status_line("UU conflict.rs").map(|change| change.status),
            Some(ChangeStatus::Conflicted)
        );
    }

    #[test]
    fn a_rename_is_reported_at_its_new_path() {
        let change = parse_status_line("R  old/name.rs -> new/name.rs").expect("a rename parses");

        assert_eq!(change.path, "new/name.rs");
        assert_eq!(change.status, ChangeStatus::Renamed);
    }

    #[test]
    fn a_clean_status_line_is_not_a_change() {
        assert_eq!(parse_status_line("  "), None);
        assert_eq!(parse_status_line("!! build/"), None);
    }

    #[test]
    fn asking_for_a_repository_outside_one_fails_with_a_useful_message() {
        let outside =
            std::env::temp_dir().join(format!("brucebox-not-a-repo-{}", std::process::id()));
        std::fs::create_dir_all(&outside).expect("the temp directory is created");

        let error = open(&outside).expect_err("a plain directory is not a repository");
        assert!(
            error.message().contains("not inside a Git repository")
                || error.message().contains("cannot run")
                || error.message().contains("not a git repository"),
            "the message explains what happened: {}",
            error.message()
        );
        std::fs::remove_dir_all(outside).ok();
    }
}
