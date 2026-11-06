//! The versioned workspace document: the one persisted shape of a workspace.
//!
//! The document holds workspace identity, GPUI Base Dock's state for the docked
//! workspace, and one such state per floating surface. It deliberately does
//! **not** mirror the dock trees: a tree has one owner, and duplicating it would
//! mean two things to keep in step and a migration problem in two places. Panel
//! identity and per-panel state ride inside the dock's leaf states (see
//! [`crate::workspace::panels::host`]).
//!
//! Everything here is data: parsing, validation, and serialization only. Where
//! the document lives and when it is written belongs to
//! [`crate::workspace::store`] and [`crate::workspace::dock`].
//!
//! # Schema history
//!
//! - 1 — workspace id and the docked workspace.
//! - 2 — adds floating surfaces. Scheme 1 documents are read by upgrading them
//!   in memory; a *newer* schema is refused rather than guessed at.

use std::fmt;

use gpui_kit::base::dock::DockAreaState;
use serde::{Deserialize, Serialize};

use crate::workspace::surface::FloatingSurfaceState;

/// Schema version of the workspace document.
///
/// Bumped when the document shape changes in a way an older build cannot read.
/// The dock state inside carries its own version, which the dock checks itself.
pub const WORKSPACE_DOCUMENT_SCHEMA: u32 = 2;

/// A saved workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceDocument {
    schema: u32,
    workspace: String,
    dock: DockAreaState,
    /// Floating surfaces, in stacking order. Absent in a schema 1 document.
    #[serde(default)]
    surfaces: Vec<FloatingSurfaceState>,
}

impl WorkspaceDocument {
    /// A document for `workspace` at the current schema version.
    pub fn current(workspace: impl Into<String>, dock: DockAreaState) -> Self {
        Self::with_surfaces(workspace, dock, Vec::new())
    }

    /// A document holding the docked workspace and its floating surfaces.
    pub fn with_surfaces(
        workspace: impl Into<String>,
        dock: DockAreaState,
        surfaces: Vec<FloatingSurfaceState>,
    ) -> Self {
        Self {
            schema: WORKSPACE_DOCUMENT_SCHEMA,
            workspace: workspace.into(),
            dock,
            surfaces,
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

    /// The floating surfaces, in stacking order.
    pub fn surfaces(&self) -> &[FloatingSurfaceState] {
        &self.surfaces
    }

    /// Take the floating surfaces, for rebuilding them.
    pub fn into_surfaces(self) -> Vec<FloatingSurfaceState> {
        self.surfaces
    }

    /// Split the document into the docked state and the floating surfaces.
    pub fn into_parts(self) -> (DockAreaState, Vec<FloatingSurfaceState>) {
        (self.dock, self.surfaces)
    }

    /// Check that this document can be restored into `workspace`.
    ///
    /// A document from an older schema is accepted: every change so far has
    /// been additive, so the older shape is a subset this build reads directly
    /// (see [`Self::upgraded`]). A newer schema is refused — it may describe
    /// surfaces this build has no way to reproduce.
    pub fn validate(&self, workspace: &str) -> Result<(), DocumentError> {
        if self.schema > WORKSPACE_DOCUMENT_SCHEMA {
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

    /// Bring an older document up to the current schema.
    ///
    /// Called after [`Self::validate`] accepted it. The only migration so far
    /// is schema 1 → 2: a document written before floating surfaces existed
    /// simply has none.
    pub fn upgraded(mut self) -> Self {
        self.schema = WORKSPACE_DOCUMENT_SCHEMA;
        self
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
    use crate::workspace::surface::{FloatingSurfaceState, SurfaceBounds, SurfaceId};
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

    #[test]
    fn a_schema_one_document_still_loads_and_gains_no_surfaces() {
        // Written before floating surfaces existed: no `surfaces` key at all.
        let raw = format!(
            r#"{{"schema":1,"workspace":"brucebox.default","dock":{}}}"#,
            serde_json::to_string(&dock_state()).unwrap()
        );

        let document = WorkspaceDocument::from_json(&raw).expect("an older document parses");
        document
            .validate("brucebox.default")
            .expect("an older schema is readable");

        let upgraded = document.upgraded();
        assert_eq!(upgraded.schema(), WORKSPACE_DOCUMENT_SCHEMA);
        assert!(upgraded.surfaces().is_empty());
        assert_eq!(upgraded.dock().center.children.len(), 1);
    }

    #[test]
    fn surfaces_round_trip_with_the_docked_workspace() {
        let surface = FloatingSurfaceState::new(
            SurfaceId::next(),
            SurfaceBounds::new(64., 48., 500., 320.),
            3,
            dock_state(),
        );
        let document = WorkspaceDocument::with_surfaces(
            "brucebox.default",
            dock_state(),
            vec![surface.clone()],
        );

        let raw = document.to_json().expect("the document serializes");
        let parsed = WorkspaceDocument::from_json(&raw).expect("the document parses");

        assert_eq!(parsed.surfaces(), &[surface.clone()]);
        assert_eq!(parsed.surfaces()[0].bounds(), surface.bounds());
        assert_eq!(parsed.surfaces()[0].z(), 3);
    }

    #[test]
    fn a_document_without_a_surfaces_key_reads_as_no_surfaces() {
        let raw = format!(
            r#"{{"schema":{},"workspace":"brucebox.default","dock":{}}}"#,
            WORKSPACE_DOCUMENT_SCHEMA,
            serde_json::to_string(&dock_state()).unwrap()
        );

        let document = WorkspaceDocument::from_json(&raw).expect("a missing list is empty");

        assert!(document.surfaces().is_empty());
    }
}
