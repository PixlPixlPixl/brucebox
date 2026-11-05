//! The versioned workspace document: the one persisted shape of a workspace.
//!
//! The document holds workspace identity and GPUI Base Dock's own state. It
//! deliberately does **not** mirror the dock tree: the tree has one owner, and
//! duplicating it would mean two things to keep in step and a migration problem
//! in two places. Panel identity and per-panel state ride inside the dock's leaf
//! states (see [`crate::workspace::panels::host`]).
//!
//! Everything here is data: parsing, validation, and serialization only. Where
//! the document lives and when it is written belongs to
//! [`crate::workspace::store`] and [`crate::workspace::dock`].

use std::fmt;

use gpui_kit::base::dock::DockAreaState;
use serde::{Deserialize, Serialize};

/// Schema version of the workspace document.
///
/// Bumped when the document shape changes in a way an older build cannot read.
/// The dock state inside carries its own version, which the dock checks itself.
pub const WORKSPACE_DOCUMENT_SCHEMA: u32 = 1;

/// A saved workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceDocument {
    schema: u32,
    workspace: String,
    dock: DockAreaState,
}

impl WorkspaceDocument {
    /// A document for `workspace` at the current schema version.
    pub fn current(workspace: impl Into<String>, dock: DockAreaState) -> Self {
        Self {
            schema: WORKSPACE_DOCUMENT_SCHEMA,
            workspace: workspace.into(),
            dock,
        }
    }

    /// The schema version this document was written with.
    pub fn schema(&self) -> u32 {
        self.schema
    }

    /// The workspace this document belongs to.
    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    /// GPUI Base Dock's state.
    pub fn dock(&self) -> &DockAreaState {
        &self.dock
    }

    /// Take the dock state, for handing it to the dock.
    pub fn into_dock(self) -> DockAreaState {
        self.dock
    }

    /// Check that this document can be restored into `workspace`.
    pub fn validate(&self, workspace: &str) -> Result<(), DocumentError> {
        if self.schema != WORKSPACE_DOCUMENT_SCHEMA {
            return Err(DocumentError::Schema {
                found: self.schema,
                expected: WORKSPACE_DOCUMENT_SCHEMA,
            });
        }
        if self.workspace != workspace {
            return Err(DocumentError::Workspace {
                found: self.workspace.clone(),
                expected: workspace.to_owned(),
            });
        }
        Ok(())
    }

    /// Serialize for storage.
    pub fn to_json(&self) -> Result<String, DocumentError> {
        serde_json::to_string_pretty(self).map_err(DocumentError::Json)
    }

    /// Parse a stored document. Call [`Self::validate`] before restoring it.
    pub fn from_json(raw: &str) -> Result<Self, DocumentError> {
        serde_json::from_str(raw).map_err(DocumentError::Json)
    }
}

/// Why a workspace document could not be read or used.
#[derive(Debug)]
pub enum DocumentError {
    /// The stored document is not valid JSON for this schema.
    Json(serde_json::Error),
    /// The document was written by an incompatible build.
    Schema { found: u32, expected: u32 },
    /// The document belongs to a different workspace.
    Workspace { found: String, expected: String },
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "the workspace document is not readable: {error}"),
            Self::Schema { found, expected } => write!(
                f,
                "the workspace document uses schema {found}, but this build reads schema {expected}"
            ),
            Self::Workspace { found, expected } => write!(
                f,
                "the workspace document belongs to `{found}`, not `{expected}`"
            ),
        }
    }
}

impl std::error::Error for DocumentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::base::dock::{PanelInfo, PanelState};

    fn dock_state() -> DockAreaState {
        DockAreaState {
            version: Some(1),
            center: PanelState {
                panel_name: "TabPanel".to_owned(),
                children: vec![PanelState::new("brucebox.git.status")],
                info: PanelInfo::tabs(0),
            },
            left_dock: None,
            right_dock: None,
            bottom_dock: None,
        }
    }

    #[test]
    fn a_document_round_trips_through_json_with_the_dock_tree_intact() {
        let document = WorkspaceDocument::current("brucebox.default", dock_state());

        let raw = document.to_json().expect("the document serializes");
        let parsed = WorkspaceDocument::from_json(&raw).expect("the document parses");

        assert_eq!(parsed, document);
        assert_eq!(parsed.dock().center.children.len(), 1);
        assert_eq!(
            parsed.dock().center.children[0].panel_name,
            "brucebox.git.status"
        );
        assert_eq!(parsed.workspace(), "brucebox.default");
        assert_eq!(parsed.schema(), WORKSPACE_DOCUMENT_SCHEMA);
    }

    #[test]
    fn a_document_from_another_schema_is_rejected_politely() {
        let raw = format!(
            r#"{{"schema":{},"workspace":"brucebox.default","dock":{}}}"#,
            WORKSPACE_DOCUMENT_SCHEMA + 1,
            serde_json::to_string(&dock_state()).unwrap()
        );
        let document = WorkspaceDocument::from_json(&raw).expect("the shape still parses");

        let error = document
            .validate("brucebox.default")
            .expect_err("an unknown schema must not be restored");

        assert!(matches!(error, DocumentError::Schema { .. }));
        assert!(error.to_string().contains("schema"));
    }

    #[test]
    fn a_document_for_another_workspace_is_rejected() {
        let document = WorkspaceDocument::current("brucebox.other", dock_state());

        let error = document
            .validate("brucebox.default")
            .expect_err("a document belongs to one workspace");

        assert!(matches!(error, DocumentError::Workspace { .. }));
    }

    #[test]
    fn unreadable_json_is_an_error_not_a_panic() {
        let error = WorkspaceDocument::from_json("{ not json").expect_err("garbage fails");

        assert!(matches!(error, DocumentError::Json(_)));
        assert!(error.to_string().contains("not readable"));
    }
}
