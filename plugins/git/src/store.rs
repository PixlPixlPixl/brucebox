//! The plugin's shared repository state.
//!
//! Both panels read this store, the poller refreshes it, and every meaningful
//! change is published on the contract bus. Keeping it in one place is what makes
//! the panels independent of each other: the Diff panel does not know which panel
//! chose the file, and the poller does not know whether any panel is open.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use brucebox_git_contract::{RepositorySnapshot, Selection, publish_selection, publish_snapshot};

use crate::repository;

/// A snapshot of what the plugin currently knows.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The open repository's root, when one is open.
    pub root: Option<PathBuf>,
    /// Its current status.
    pub snapshot: Option<RepositorySnapshot>,
    /// The file whose diff is shown.
    pub selected: Option<String>,
    /// That file's diff.
    pub diff: String,
    /// The last thing that went wrong, if the last action did.
    pub error: Option<String>,
}

/// The plugin's one piece of shared state.
#[derive(Default)]
pub struct GitStore {
    state: Mutex<State>,
}

impl GitStore {
    /// The process-wide store.
    ///
    /// One plugin instance, one repository: a workspace with two Repository panels
    /// shows the same repository in both, which is what a user opening the same
    /// thing twice expects.
    pub fn shared() -> Arc<GitStore> {
        static STORE: OnceLock<Arc<GitStore>> = OnceLock::new();
        STORE.get_or_init(|| Arc::new(Self::default())).clone()
    }

    /// A copy of the current state.
    pub fn state(&self) -> State {
        self.lock().clone()
    }

    /// The repository to open when the plugin starts.
    pub fn default_root() -> Option<PathBuf> {
        if let Some(configured) = std::env::var_os("BRUCEBOX_GIT_REPO")
            && !configured.is_empty()
        {
            return Some(PathBuf::from(configured));
        }
        std::env::current_dir().ok()
    }

    /// Open (or switch to) the repository containing `path`.
    ///
    /// Failure is reported in the state, not thrown away: the panel shows the
    /// reason and the previous repository stays open.
    pub fn open(&self, path: &Path) -> Result<(), String> {
        match repository::open(path) {
            Ok(root) => {
                {
                    let mut state = self.lock();
                    state.root = Some(root.clone());
                    state.error = None;
                    state.selected = None;
                    state.diff.clear();
                }
                self.refresh();
                Ok(())
            }
            Err(error) => {
                self.lock().error = Some(error.message().to_owned());
                Err(error.message().to_owned())
            }
        }
    }

    /// Re-read the open repository and publish the result.
    ///
    /// A failure keeps the last good snapshot and surfaces the reason: a
    /// repository that briefly disappears (a rebase, a deleted work tree) should
    /// not blank the interface.
    pub fn refresh(&self) {
        let root = {
            let state = self.lock();
            match &state.root {
                Some(root) => root.clone(),
                None => return,
            }
        };

        match repository::snapshot(&root) {
            Ok(snapshot) => {
                let previous = {
                    let mut state = self.lock();
                    state.error = None;
                    let previous = state.snapshot.clone();
                    state.snapshot = Some(snapshot.clone());
                    previous
                };
                // Publish on every refresh: the bus drops byte-identical repeats,
                // so a quiet repository costs nothing and a changed one always
                // reaches consumers.
                let _ = previous;
                let _ = publish_snapshot(&snapshot);
            }
            Err(error) => {
                self.lock().error = Some(error.message().to_owned());
            }
        }
    }

    /// Select a file and publish its diff.
    pub fn select(&self, path: Option<String>) {
        let (root, change) = {
            let state = self.lock();
            let Some(root) = state.root.clone() else {
                return;
            };
            let change = path.as_deref().and_then(|path| {
                state.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .changes
                        .iter()
                        .find(|change| change.path == path)
                        .cloned()
                })
            });
            (root, change)
        };

        let (path, diff) = match &change {
            Some(change) => {
                let text =
                    repository::diff(&root, change).unwrap_or_else(|error| error.to_string());
                (Some(change.path.clone()), text)
            }
            None => (None, String::new()),
        };

        {
            let mut state = self.lock();
            state.selected = path.clone();
            state.diff = diff.clone();
        }

        let selection = Selection {
            root: root.display().to_string(),
            path,
            diff,
            truncated: false,
        };
        let _ = publish_selection(&selection);
    }

    /// Move the selection by `delta` files, wrapping at the ends.
    pub fn step_selection(&self, delta: isize) {
        let state = self.lock();
        let Some(snapshot) = &state.snapshot else {
            return;
        };
        if snapshot.changes.is_empty() {
            return;
        }
        let count = snapshot.changes.len() as isize;
        let current = state
            .selected
            .as_ref()
            .and_then(|selected| {
                snapshot
                    .changes
                    .iter()
                    .position(|change| &change.path == selected)
            })
            .map(|index| index as isize)
            .unwrap_or(-1);
        let next = (current + delta).rem_euclid(count);
        let path = snapshot.changes[next as usize].path.clone();
        drop(state);
        self.select(Some(path));
    }

    /// Forget everything and publish that nothing is open.
    pub fn clear(&self) {
        let mut state = self.lock();
        state.root = None;
        state.snapshot = None;
        state.selected = None;
        state.diff.clear();
        state.error = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unopened_store_has_no_repository_and_no_selection() {
        let store = GitStore::default();
        let state = store.state();

        assert!(state.root.is_none());
        assert!(state.snapshot.is_none());
        assert!(state.selected.is_none());
        assert!(state.diff.is_empty());
    }

    #[test]
    fn opening_something_that_is_not_a_repository_reports_why() {
        let store = GitStore::default();
        let error = store
            .open(Path::new("/definitely/not/a/repository"))
            .expect_err("a missing path is not a repository");

        assert!(!error.is_empty());
        assert_eq!(store.state().error.as_deref(), Some(error.as_str()));
        assert!(store.state().root.is_none(), "nothing is opened on failure");
    }

    #[test]
    fn stepping_the_selection_without_a_snapshot_does_nothing() {
        let store = GitStore::default();
        store.step_selection(1);

        assert!(store.state().selected.is_none());
    }
}
