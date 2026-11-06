//! The Git plugin's panels: repository status and diff.
//!
//! Two panel types over one shared store. The Diff panel never learns which
//! panel chose the file — it reads the plugin's selection — so the two can be
//! docked, split, floated or closed independently, and a workspace with no Diff
//! panel behaves the same as one with it.
//!
//! Input arrives through the host (`PluginPanelInput`): GPUI's own key listeners
//! cannot run inside a plugin library (ADR 0005). Typing edits the repository
//! path, Enter opens it, and the arrow keys move the file selection.

use std::path::PathBuf;
use std::time::Duration;

use brucebox_git_contract::ChangeStatus;
use brucebox_sdk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::store::GitStore;

/// The panel type id of the repository panel.
pub const REPOSITORY_PANEL: &str = "brucebox.git.repository";

/// The panel type id of the diff panel.
pub const DIFF_PANEL: &str = "brucebox.git.diff";

/// How often a panel repaints from the shared store.
///
/// The poller refreshes the store on its own schedule; this only decides how
/// quickly the interface notices.
const TICK: Duration = Duration::from_millis(600);

const MUTED: u32 = 0x8b93a1;
const FAINT: u32 = 0x6f7684;
const HIGHLIGHT: u32 = 0x2a3140;
const ADDED: u32 = 0x7dd3a0;
const MODIFIED: u32 = 0xe2c08d;
const DELETED: u32 = 0xe08d8d;

/// The panel's own persisted state.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
struct SavedRepository {
    path_input: String,
}

/// Repository status: the path field, the branch, and the changed files.
pub struct RepositoryPanel {
    #[allow(dead_code)]
    focus_handle: FocusHandle,
    store: std::sync::Arc<GitStore>,
    path_input: String,
    _tick: Task<()>,
}

impl RepositoryPanel {
    /// Build the panel, restoring the path it last showed.
    pub fn new(open: &PanelOpen<'_>, cx: &mut App) -> Entity<Self> {
        let saved = open
            .restored
            .and_then(|text| serde_json::from_str::<SavedRepository>(text).ok())
            .unwrap_or_default();
        let store = GitStore::shared();
        let _ = open.instance;

        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            store,
            path_input: saved.path_input,
            _tick: repaint_tick(cx),
        })
    }

    /// Handle one event forwarded by the host.
    pub fn on_input(
        &mut self,
        input: &PluginInput<'_>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PluginInput::Key(key) = input else {
            return;
        };
        if key.control || key.alt {
            return;
        }

        match key.key {
            // Arrow keys are consumed by the dock engine before a plugin panel
            // sees them, so the list also walks with `k`/`j`; both paths are live
            // wherever the host does forward the arrows.
            "up" | "k" => self.store.step_selection(-1),
            "down" | "j" => self.store.step_selection(1),
            "enter" => {
                let path = PathBuf::from(self.path_input.trim());
                if path.as_os_str().is_empty() {
                    return;
                }
                if self.store.open(&path).is_ok() {
                    // Show the resolved root, so the field says what is open
                    // rather than what was typed.
                    if let Some(root) = self.store.state().root {
                        self.path_input = root.display().to_string();
                    }
                }
            }
            "backspace" => {
                self.path_input.pop();
            }
            _ => {
                if let Some(text) = key.key_char {
                    self.path_input.push_str(text);
                }
            }
        }
        cx.notify();
    }
}

impl Panel for RepositoryPanel {
    fn panel_name(&self) -> &'static str {
        REPOSITORY_PANEL
    }

    fn dump(&self, _: &App) -> PanelState {
        let mut state = PanelState::new(REPOSITORY_PANEL);
        state.info = PanelInfo::Panel(
            serde_json::to_value(SavedRepository {
                path_input: self.path_input.clone(),
            })
            .unwrap_or(serde_json::Value::Null),
        );
        state
    }
}

impl EventEmitter<PanelEvent> for RepositoryPanel {}

impl Focusable for RepositoryPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // The host wrapper holds the keyboard for a plugin panel.
        let _ = &self.focus_handle;
        cx.focus_handle()
    }
}

impl Render for RepositoryPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.state();
        let (label, colour) = match (&state.snapshot, &state.error) {
            (_, Some(error)) => (error.clone(), DELETED),
            (Some(snapshot), _) => (snapshot.summary(), MUTED),
            (None, None) => ("no repository open".to_owned(), FAINT),
        };

        let rows: Vec<Div> = state
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .changes
                    .iter()
                    .map(|change| {
                        let selected = state.selected.as_deref() == Some(change.path.as_str());
                        let row = div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .px(px(4.))
                            .child(
                                div()
                                    .w(px(14.))
                                    .text_color(rgb(status_colour(change.status)))
                                    .child(change.letter().to_string()),
                            )
                            .child(div().flex_1().child(change.path.clone()));
                        if selected {
                            row.bg(rgb(HIGHLIGHT))
                        } else {
                            row
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let truncated = state
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.truncated)
            .then(|| {
                div()
                    .text_color(rgb(DELETED))
                    .child("… more changes than shown")
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(16.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Repository"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(div().text_color(rgb(FAINT)).child("path"))
                    .child(div().flex_1().child(if self.path_input.is_empty() {
                        "type a repository path and press Enter".to_owned()
                    } else {
                        self.path_input.clone()
                    })),
            )
            .child(div().text_color(rgb(colour)).child(label))
            .children(rows)
            .children(truncated)
            .child(
                div()
                    .mt(px(8.))
                    .text_color(rgb(FAINT))
                    .child("j / k select a file · Enter open path"),
            )
    }
}

/// The selected file's diff.
pub struct DiffPanel {
    #[allow(dead_code)]
    focus_handle: FocusHandle,
    store: std::sync::Arc<GitStore>,
    _tick: Task<()>,
}

impl DiffPanel {
    /// Build the panel.
    pub fn new(open: &PanelOpen<'_>, cx: &mut App) -> Entity<Self> {
        let store = GitStore::shared();
        let _ = open.instance;

        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            store,
            _tick: repaint_tick(cx),
        })
    }
}

impl Panel for DiffPanel {
    fn panel_name(&self) -> &'static str {
        DIFF_PANEL
    }
}

impl EventEmitter<PanelEvent> for DiffPanel {}

impl Focusable for DiffPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let _ = &self.focus_handle;
        cx.focus_handle()
    }
}

impl Render for DiffPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.state();
        let heading = match (&state.selected, &state.root) {
            (Some(path), _) => path.clone(),
            (None, Some(_)) => "select a file in the Repository panel".to_owned(),
            (None, None) => "no repository open".to_owned(),
        };

        let mut lines: Vec<Div> = Vec::new();
        for line in state.diff.lines() {
            let colour = if line.starts_with('+') && !line.starts_with("+++") {
                ADDED
            } else if line.starts_with('-') && !line.starts_with("---") {
                DELETED
            } else if line.starts_with("@@") {
                MODIFIED
            } else {
                MUTED
            };
            lines.push(div().text_color(rgb(colour)).child(line.to_owned()));
        }
        if lines.is_empty() && state.selected.is_some() {
            lines.push(div().text_color(rgb(FAINT)).child("no diff for this file"));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(16.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Diff"))
            .child(div().text_color(rgb(FAINT)).child(heading))
            .children(lines)
    }
}

/// Keep a panel repainting while the shared store changes.
fn repaint_tick<T: 'static>(cx: &mut Context<T>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }
    })
}

fn status_colour(status: ChangeStatus) -> u32 {
    match status {
        ChangeStatus::Added | ChangeStatus::Untracked | ChangeStatus::Renamed => ADDED,
        ChangeStatus::Modified | ChangeStatus::TypeChanged => MODIFIED,
        ChangeStatus::Deleted | ChangeStatus::Conflicted => DELETED,
        ChangeStatus::Unknown => MUTED,
    }
}
