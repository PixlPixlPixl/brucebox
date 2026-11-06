//! What this plugin knows about Git, and how it learns it.
//!
//! The Git plugin publishes a repository snapshot and the selected diff on the
//! host's contract bus. This plugin subscribes to both and keeps the latest value
//! in a shared cell the panel reads. It links only the *contract* crate — never the
//! Git plugin's implementation — so it cannot depend on how Git is implemented, and
//! it keeps working when the Git plugin is not installed at all.
//!
//! Availability is therefore a normal state, not an error: no publication means no
//! snapshot, and the panel says so.

use std::sync::{Arc, Mutex, OnceLock};

use brucebox_git_contract::{
    RepositorySnapshot, Selection, Subscription, repository, selection, subscribe,
    subscribe_selection,
};

/// The Git context this plugin currently sees.
#[derive(Default)]
pub struct GitContext {
    snapshot: Mutex<Option<RepositorySnapshot>>,
    selection: Mutex<Option<Selection>>,
    subscriptions: Mutex<Vec<Subscription>>,
}

impl GitContext {
    /// The process-wide context.
    pub fn shared() -> Arc<GitContext> {
        static CONTEXT: OnceLock<Arc<GitContext>> = OnceLock::new();
        CONTEXT.get_or_init(|| Arc::new(Self::default())).clone()
    }

    /// The latest repository snapshot, if a Git plugin has published one.
    pub fn snapshot(&self) -> Option<RepositorySnapshot> {
        self.lock_snapshot().clone()
    }

    /// The latest selection, if a Git plugin has published one.
    pub fn selection(&self) -> Option<Selection> {
        self.lock_selection().clone()
    }

    /// Whether anything Git is currently publishing.
    pub fn is_connected(&self) -> bool {
        self.lock_snapshot().is_some()
    }

    /// Start following the Git contract.
    ///
    /// Called once from the plugin's `apply`. Re-running it (a plugin restart)
    /// replaces the previous subscriptions rather than doubling them.
    pub fn connect(&self) {
        let mut subscriptions = self
            .subscriptions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        subscriptions.clear();

        // Read what is already published, so a consumer that starts after the
        // publisher has something to show before the first change.
        *self.lock_snapshot() = repository();
        *self.lock_selection() = selection();

        let snapshots = self.shared_handle();
        if let Ok(subscription) = subscribe(move |snapshot| {
            *snapshots.lock_snapshot() = Some(snapshot);
        }) {
            subscriptions.push(subscription);
        }

        let selections = self.shared_handle();
        if let Ok(subscription) = subscribe_selection(move |selection| {
            *selections.lock_selection() = Some(selection);
        }) {
            subscriptions.push(subscription);
        }
    }

    /// Stop following the contract and forget what was seen.
    pub fn disconnect(&self) {
        self.subscriptions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        *self.lock_snapshot() = None;
        *self.lock_selection() = None;
    }

    /// A one-line description of the repository, for the panel header.
    pub fn summary(&self) -> Option<String> {
        self.snapshot().map(|snapshot| snapshot.summary())
    }

    /// What to attach to a conversation: the repository state and, when a file is
    /// selected, that file's diff.
    pub fn attachment(&self) -> Option<String> {
        let snapshot = self.snapshot()?;
        let mut text = format!("Repository: {}\n{}", snapshot.root, snapshot.summary());
        if !snapshot.changes.is_empty() {
            text.push_str("\nChanged:");
            for change in snapshot.changes.iter().take(8) {
                text.push_str(&format!("\n  {} {}", change.letter(), change.path));
            }
            if snapshot.changes.len() > 8 {
                text.push_str(&format!("\n  … and {} more", snapshot.changes.len() - 8));
            }
        }
        if let Some(selection) = self.selection()
            && !selection.diff.is_empty()
        {
            text.push_str(&format!(
                "\n\nDiff of {}:\n{}",
                selection.summary(),
                selection.diff
            ));
        }
        Some(text)
    }

    fn shared_handle(&self) -> Arc<Self> {
        // The subscribers need an owned handle; the context itself is the
        // process-wide one, so a weak-free clone of the `Arc` is correct here.
        GitContext::shared()
    }

    fn lock_snapshot(&self) -> std::sync::MutexGuard<'_, Option<RepositorySnapshot>> {
        self.snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn lock_selection(&self) -> std::sync::MutexGuard<'_, Option<Selection>> {
        self.selection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brucebox_git_contract::{Change, ChangeStatus};

    #[test]
    fn without_a_publisher_there_is_nothing_to_show() {
        let context = GitContext::default();

        assert!(!context.is_connected());
        assert!(context.summary().is_none());
        assert!(context.attachment().is_none());
    }

    #[test]
    fn an_attachment_names_the_repository_and_its_changes() {
        let context = GitContext::default();
        *context.lock_snapshot() = Some(RepositorySnapshot {
            root: "/work/project".to_owned(),
            branch: Some("main".to_owned()),
            head: Some("abc1234".to_owned()),
            changes: vec![
                Change {
                    path: "src/main.rs".to_owned(),
                    status: ChangeStatus::Modified,
                },
                Change {
                    path: "README.md".to_owned(),
                    status: ChangeStatus::Untracked,
                },
            ],
            truncated: false,
        });

        let text = context
            .attachment()
            .expect("a snapshot produces an attachment");

        assert!(text.contains("/work/project"), "{text}");
        assert!(text.contains("main · abc1234 · 2 changed files"), "{text}");
        assert!(text.contains("M src/main.rs"), "{text}");
        assert!(text.contains("? README.md"), "{text}");
    }

    #[test]
    fn a_selected_diff_is_part_of_the_attachment() {
        let context = GitContext::default();
        *context.lock_snapshot() = Some(RepositorySnapshot {
            root: "/work/project".to_owned(),
            branch: Some("main".to_owned()),
            head: None,
            changes: Vec::new(),
            truncated: false,
        });
        *context.lock_selection() = Some(Selection {
            root: "/work/project".to_owned(),
            path: Some("src/main.rs".to_owned()),
            diff: "+added\n-removed\n".to_owned(),
            truncated: false,
        });

        let text = context
            .attachment()
            .expect("a snapshot produces an attachment");

        assert!(text.contains("Diff of src/main.rs"), "{text}");
        assert!(text.contains("+added"), "{text}");
    }

    #[test]
    fn disconnecting_forgets_what_was_seen() {
        let context = GitContext::default();
        *context.lock_snapshot() = Some(RepositorySnapshot {
            root: "/work/project".to_owned(),
            branch: None,
            head: None,
            changes: Vec::new(),
            truncated: false,
        });

        context.disconnect();

        assert!(!context.is_connected());
    }
}
