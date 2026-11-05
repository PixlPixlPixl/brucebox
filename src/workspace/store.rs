//! Where a workspace document lives on disk.
//!
//! The store is deliberately dumb: one file per workspace, atomic writes, and
//! enough information for the caller to decide what to do when a file is
//! unreadable. Quarantining a bad file is a decision the caller makes, not
//! something the store does behind its back.

use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::document::{DocumentError, WorkspaceDocument};

/// Environment variable that overrides the Brucebox user data directory.
///
/// Used by tests and by developers who want separate workspace state per build.
pub const DATA_DIR_ENV: &str = "BRUCEBOX_DATA_DIR";

/// The persisted workspace document for one workspace id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStore {
    path: PathBuf,
}

impl WorkspaceStore {
    /// A store writing to an explicit file, for tests and non-default roots.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default store for `workspace`, in the platform's user data directory.
    pub fn for_workspace(workspace: &str) -> Self {
        Self::new(workspace_path(workspace))
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the stored document, if there is one.
    ///
    /// Returns `Ok(None)` only when nothing has been saved yet. A file that
    /// exists but cannot be parsed is an error, so the caller can quarantine it
    /// instead of silently overwriting the user's layout on the next save.
    pub fn load(&self) -> Result<Option<WorkspaceDocument>, StoreError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        WorkspaceDocument::from_json(&raw)
            .map(Some)
            .map_err(StoreError::Document)
    }

    /// Write the document, replacing any previous one atomically.
    pub fn save(&self, document: &WorkspaceDocument) -> Result<(), StoreError> {
        self.save_json(&document.to_json()?)
    }

    /// Write an already serialized document atomically.
    ///
    /// The write goes to a sibling temporary file and is renamed into place, so
    /// a crash mid-write cannot leave a half-written layout behind.
    pub fn save_json(&self, json: &str) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, json).map_err(|source| StoreError::Io {
            path: temporary.clone(),
            source,
        })?;
        fs::rename(&temporary, &self.path).map_err(|source| StoreError::Io {
            path: self.path.clone(),
            source,
        })
    }

    /// Delete the stored document, if any. Used by an explicit reset.
    pub fn remove(&self) -> Result<(), StoreError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io {
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// Move an unusable file aside, returning where it went.
    ///
    /// Called when a stored document cannot be restored. Keeping the file means
    /// a user can recover an old layout, and it cannot be overwritten by the
    /// save that follows a fallback to the default preset.
    pub fn quarantine(&self) -> Option<PathBuf> {
        let target = self.path.with_extension("json.invalid");
        fs::rename(&self.path, &target).ok().map(|_| target)
    }
}

/// The Brucebox user data directory for this platform.
pub fn user_data_dir() -> PathBuf {
    if let Some(dir) = env::var_os(DATA_DIR_ENV).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("brucebox");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = env::var_os("APPDATA") {
            return PathBuf::from(app_data).join("brucebox");
        }
    }

    if let Some(dir) = env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir).join("brucebox");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("brucebox");
    }

    // No home directory at all: keep state beside the process rather than
    // failing to start. Nothing in this foundation depends on the location.
    PathBuf::from(".brucebox")
}

/// The default document path for one workspace id.
pub fn workspace_path(workspace: &str) -> PathBuf {
    user_data_dir()
        .join("workspaces")
        .join(format!("{workspace}.json"))
}

/// Why a stored document could not be read or written.
#[derive(Debug)]
pub enum StoreError {
    /// Filesystem access failed.
    Io { path: PathBuf, source: io::Error },
    /// The file exists but is not a usable workspace document.
    Document(DocumentError),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Document(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Document(error) => Some(error),
        }
    }
}

impl From<DocumentError> for StoreError {
    fn from(error: DocumentError) -> Self {
        Self::Document(error)
    }
}

/// A unique, empty directory under the system temp dir for one test.
#[cfg(test)]
fn test_dir(label: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    let dir = env::temp_dir().join(format!("brucebox-{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("the test directory is creatable");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};

    fn document(workspace: &str) -> WorkspaceDocument {
        WorkspaceDocument::current(
            workspace,
            DockAreaState {
                version: Some(1),
                center: PanelState {
                    panel_name: "TabPanel".to_owned(),
                    children: vec![PanelState {
                        panel_name: "brucebox.git.status".to_owned(),
                        children: Vec::new(),
                        info: PanelInfo::panel(serde_json::json!({"branch": "main"})),
                    }],
                    info: PanelInfo::tabs(0),
                },
                left_dock: None,
                right_dock: None,
                bottom_dock: None,
            },
        )
    }

    #[test]
    fn a_saved_document_is_loaded_back_unchanged() {
        let dir = test_dir("store-roundtrip");
        let store = WorkspaceStore::new(dir.join("workspace.json"));
        let document = document("brucebox.default");

        assert!(store.load().unwrap().is_none(), "nothing is saved yet");
        store.save(&document).expect("the document saves");

        let loaded = store
            .load()
            .expect("the document loads")
            .expect("a document exists");
        assert_eq!(loaded, document);
        assert_eq!(
            loaded.dock().center.children[0].info,
            PanelInfo::panel(serde_json::json!({"branch": "main"}))
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_replaces_the_previous_document_and_leaves_no_temp_file() {
        let dir = test_dir("store-replace");
        let store = WorkspaceStore::new(dir.join("workspace.json"));

        store.save(&document("one")).unwrap();
        store.save(&document("two")).unwrap();

        assert_eq!(store.load().unwrap().unwrap().workspace(), "two");
        assert!(
            !store.path().with_extension("json.tmp").exists(),
            "the temporary file is renamed into place"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_file_is_an_error_and_can_be_quarantined() {
        let dir = test_dir("store-corrupt");
        let store = WorkspaceStore::new(dir.join("workspace.json"));
        fs::write(store.path(), "{ this is not json").unwrap();

        let error = store
            .load()
            .expect_err("a corrupt file is not silently ignored");
        assert!(matches!(error, StoreError::Document(_)));

        let quarantined = store.quarantine().expect("the corrupt file is moved aside");
        assert!(!store.path().exists());
        assert!(quarantined.exists());
        assert!(store.load().unwrap().is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removing_an_absent_document_is_not_an_error() {
        let dir = test_dir("store-remove");
        let store = WorkspaceStore::new(dir.join("workspace.json"));

        store.remove().expect("removing nothing succeeds");
        store.save(&document("one")).unwrap();
        store
            .remove()
            .expect("removing the saved document succeeds");
        assert!(store.load().unwrap().is_none());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn workspace_documents_live_in_their_own_directory() {
        let path = workspace_path("brucebox.default");

        assert_eq!(path.file_name().unwrap(), "brucebox.default.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "workspaces");
        assert!(path.starts_with(user_data_dir()));
    }
}
