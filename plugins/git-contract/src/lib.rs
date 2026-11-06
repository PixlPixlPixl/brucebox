//! The Git plugin's published contract.
//!
//! Two plugins share this crate and neither imports the other: the Git plugin
//! publishes a repository snapshot and the current selection's diff, and any other
//! plugin can read or subscribe to them through the host's contract bus.
//!
//! ```ignore
//! // the Git plugin
//! brucebox_git_contract::publish_snapshot(&snapshot)?;
//!
//! // a consumer
//! if let Some(snapshot) = brucebox_git_contract::repository() {
//!     println!("{} on {}", snapshot.changed_count(), snapshot.branch_label());
//! }
//! let _live = brucebox_git_contract::subscribe(|snapshot| { /* … */ })?;
//! ```
//!
//! # Why data and not a service
//!
//! Cordis services are keyed by name but *stored* as `Arc<dyn Any>` and handed
//! back through a `TypeId` downcast, and two separately compiled plugins do not
//! agree on `TypeId` (ADR 0007 has the measurement: one extra feature on a
//! transitive dependency is enough). The types here still give both sides the
//! same Rust value; the bus carries the bytes.
//!
//! # Versioning
//!
//! The version is part of the topic name, so a consumer built against `0.1` never
//! reads a `0.2` value and silently misinterprets it: it simply finds nothing.

use brucebox_sdk::contracts;
use serde::{Deserialize, Serialize};

/// The handle a live subscription keeps. Dropping it stops delivery.
pub use brucebox_sdk::contracts::Subscription;

/// The contract version this crate describes, also embedded in every topic name.
pub const CONTRACT_VERSION: &str = "0.1";

/// The topic carrying the current [`RepositorySnapshot`].
pub const REPOSITORY_TOPIC: &str = "brucebox.git.repository/0.1";

/// The topic carrying the current [`Selection`] (the file whose diff is shown).
pub const SELECTION_TOPIC: &str = "brucebox.git.selection/0.1";

/// The most changed files one snapshot carries.
///
/// A status listing is workspace-scale data; a repository with more than this is
/// reported as truncated rather than slowing the bus down.
pub const MAX_CHANGES: usize = 500;

/// The most diff bytes one selection carries.
pub const MAX_DIFF_BYTES: usize = 256 * 1024;

/// What a repository looks like right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    /// Absolute path of the repository root (the work tree).
    pub root: String,
    /// The checked-out branch, or `None` for a detached head.
    pub branch: Option<String>,
    /// The abbreviated head commit, when the repository has one.
    pub head: Option<String>,
    /// The changed files, in path order.
    pub changes: Vec<Change>,
    /// Whether [`Self::changes`] was cut at [`MAX_CHANGES`].
    pub truncated: bool,
}

impl RepositorySnapshot {
    /// How many files changed.
    pub fn changed_count(&self) -> usize {
        self.changes.len()
    }

    /// The branch, or a phrase for a detached head.
    pub fn branch_label(&self) -> &str {
        self.branch.as_deref().unwrap_or("detached")
    }

    /// One line describing the state, for a status view or a chat message.
    pub fn summary(&self) -> String {
        let files = match self.changed_count() {
            0 => "clean".to_owned(),
            1 => "1 changed file".to_owned(),
            count => format!("{count} changed files"),
        };
        match &self.head {
            Some(head) => format!("{} · {head} · {files}", self.branch_label()),
            None => format!("{} · {files}", self.branch_label()),
        }
    }

    /// The same snapshot with its changes cut to [`MAX_CHANGES`].
    pub fn truncate(mut self) -> Self {
        if self.changes.len() > MAX_CHANGES {
            self.changes.truncate(MAX_CHANGES);
            self.truncated = true;
        }
        self
    }
}

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// The path, relative to the repository root.
    pub path: String,
    /// What happened to it.
    pub status: ChangeStatus,
}

impl Change {
    /// The letter a status view shows.
    pub fn letter(&self) -> char {
        match self.status {
            ChangeStatus::Added => 'A',
            ChangeStatus::Modified => 'M',
            ChangeStatus::Deleted => 'D',
            ChangeStatus::Renamed => 'R',
            ChangeStatus::TypeChanged => 'T',
            ChangeStatus::Untracked => '?',
            ChangeStatus::Conflicted => 'U',
            ChangeStatus::Unknown => ' ',
        }
    }
}

/// How a file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeStatus {
    /// Newly tracked.
    Added,
    /// Content changed.
    Modified,
    /// Removed.
    Deleted,
    /// Moved.
    Renamed,
    /// Mode or type changed.
    TypeChanged,
    /// Present but not tracked.
    Untracked,
    /// A merge or rebase left it conflicted.
    Conflicted,
    /// A status this contract does not name.
    Unknown,
}

/// The file whose diff is currently shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Absolute path of the repository root.
    pub root: String,
    /// The selected path, relative to the root.
    pub path: Option<String>,
    /// The diff text, cut to [`MAX_DIFF_BYTES`].
    pub diff: String,
    /// Whether [`Self::diff`] was cut.
    pub truncated: bool,
}

impl Selection {
    /// A one-line description of the selection.
    pub fn summary(&self) -> String {
        match &self.path {
            Some(path) if self.diff.is_empty() => format!("{path} (no diff)"),
            Some(path) => format!("{path} ({} lines)", self.diff.lines().count()),
            None => "nothing selected".to_owned(),
        }
    }
}

/// Publish the current repository state.
pub fn publish_snapshot(snapshot: &RepositorySnapshot) -> Result<(), contracts::ContractError> {
    contracts::publish(REPOSITORY_TOPIC, snapshot)
}

/// The current repository state, if a Git plugin has published one.
pub fn repository() -> Option<RepositorySnapshot> {
    contracts::read(REPOSITORY_TOPIC)
}

/// Subscribe to repository changes.
pub fn subscribe<F>(handler: F) -> Result<Subscription, contracts::ContractError>
where
    F: FnMut(RepositorySnapshot) + Send + 'static,
{
    contracts::subscribe(REPOSITORY_TOPIC, handler)
}

/// Publish the current selection.
pub fn publish_selection(selection: &Selection) -> Result<(), contracts::ContractError> {
    contracts::publish(SELECTION_TOPIC, selection)
}

/// The current selection, if a Git plugin has published one.
pub fn selection() -> Option<Selection> {
    contracts::read(SELECTION_TOPIC)
}

/// Subscribe to selection changes.
pub fn subscribe_selection<F>(handler: F) -> Result<Subscription, contracts::ContractError>
where
    F: FnMut(Selection) + Send + 'static,
{
    contracts::subscribe(SELECTION_TOPIC, handler)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(changes: usize) -> RepositorySnapshot {
        RepositorySnapshot {
            root: "/work/project".to_owned(),
            branch: Some("main".to_owned()),
            head: Some("abc1234".to_owned()),
            changes: (0..changes)
                .map(|index| Change {
                    path: format!("src/file{index}.rs"),
                    status: ChangeStatus::Modified,
                })
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn a_snapshot_summarises_itself_for_a_status_line() {
        assert_eq!(snapshot(0).summary(), "main · abc1234 · clean");
        assert_eq!(snapshot(1).summary(), "main · abc1234 · 1 changed file");
        assert_eq!(snapshot(3).summary(), "main · abc1234 · 3 changed files");
    }

    #[test]
    fn a_detached_head_is_described_rather_than_left_empty() {
        let mut detached = snapshot(0);
        detached.branch = None;
        assert_eq!(detached.branch_label(), "detached");
    }

    #[test]
    fn truncation_is_reported_not_silent() {
        let many = snapshot(MAX_CHANGES + 5).truncate();

        assert_eq!(many.changes.len(), MAX_CHANGES);
        assert!(many.truncated);
    }

    #[test]
    fn a_snapshot_round_trips_through_json() {
        let original = snapshot(2);
        let encoded = serde_json::to_vec(&original).expect("the snapshot encodes");
        let decoded: RepositorySnapshot = serde_json::from_slice(&encoded).expect("it decodes");

        assert_eq!(decoded, original);
    }

    #[test]
    fn the_contract_version_is_part_of_every_topic() {
        assert!(REPOSITORY_TOPIC.ends_with(CONTRACT_VERSION));
        assert!(SELECTION_TOPIC.ends_with(CONTRACT_VERSION));
    }

    #[test]
    fn change_letters_are_distinct() {
        let letters: Vec<char> = [
            ChangeStatus::Added,
            ChangeStatus::Modified,
            ChangeStatus::Deleted,
            ChangeStatus::Renamed,
            ChangeStatus::TypeChanged,
            ChangeStatus::Untracked,
            ChangeStatus::Conflicted,
        ]
        .into_iter()
        .map(|status| {
            Change {
                path: "x".to_owned(),
                status,
            }
            .letter()
        })
        .collect();

        assert_eq!(letters, vec!['A', 'M', 'D', 'R', 'T', '?', 'U']);
    }
}
