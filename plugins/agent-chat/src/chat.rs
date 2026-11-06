//! The conversation panel.
//!
//! This is real GPUI: the plugin owns the entity, the layout and its own
//! persisted state. Input arrives through the host, as plain data — GPUI's own
//! click and key listeners cannot run inside a plugin library, because the same
//! event type has a different `TypeId` in two separately compiled builds (see
//! `brucebox_plugin_api::PluginPanelInput`). The host wraps this panel in its own
//! element, reads the events there, and forwards them here.

use std::time::Duration;

use brucebox_sdk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::git::GitContext;

/// The panel type id this build registers.
///
/// Persisted layout entries and the dock's restore registry use this name, so it
/// is written out literally here rather than assembled from the plugin id.
pub const PANEL_NAME: &str = "brucebox.agent-chat.chat";

/// One line in the conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who said it.
    pub author: Author,
    /// What they said.
    pub text: String,
}

/// Who authored a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Author {
    /// The person at the keyboard.
    You,
    /// The panel's local stand-in for an agent.
    Agent,
    /// Context another plugin published, attached at the user's request.
    Context,
}

/// The panel's own persisted state.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
struct Saved {
    messages: Vec<Message>,
}

/// A native conversation panel: history, composer, and a send path.
pub struct ChatPanel {
    instance: u64,
    messages: Vec<Message>,
    draft: String,
    /// Diagnostic: the last keystroke the host forwarded.
    last_key: Option<String>,
    /// What the Git plugin is currently publishing, if anything.
    git: std::sync::Arc<GitContext>,
    /// Keeps the panel repainting while that context changes.
    _tick: Task<()>,
}

impl ChatPanel {
    /// Build the panel, restoring the conversation it saved last time.
    pub fn new(open: &PanelOpen<'_>, cx: &mut App) -> Entity<Self> {
        let saved = open
            .restored
            .and_then(|text| serde_json::from_str::<Saved>(text).ok())
            .unwrap_or_default();

        let git = GitContext::shared();
        cx.new(|cx| Self {
            instance: open.instance,
            messages: saved.messages,
            draft: String::new(),
            last_key: None,
            git,
            // The Git snapshot arrives on the runtime thread; this only decides
            // how quickly the panel notices it.
            _tick: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(600))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
        })
    }

    /// Handle one event forwarded by the host.
    pub fn on_input(
        &mut self,
        input: &PluginInput<'_>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match input {
            PluginInput::Key(key) => {
                self.last_key = Some(format!("{}/{:?}", key.key, key.key_char));
                // Ctrl/alt combinations belong to the host's key bindings, not
                // the composer, so the workspace shortcuts keep working.
                if key.control || key.alt {
                    return;
                }
                match key.key {
                    "f2" => {
                        if self.attach_git_context() {
                            cx.notify();
                        }
                    }
                    "enter" => {
                        if self.send() {
                            cx.notify();
                        }
                    }
                    "backspace" => {
                        if self.draft.pop().is_some() {
                            cx.notify();
                        }
                    }
                    _ => {
                        if let Some(text) = key.key_char {
                            self.draft.push_str(text);
                            cx.notify();
                        }
                    }
                }
            }
            PluginInput::Pointer(_) => {
                // Clicking the panel is what focuses it, and the host does that
                // on its own side. Nothing in this panel needs a pointer yet.
            }
        }
    }

    /// Attach what the Git plugin publishes to the conversation.
    ///
    /// Nothing about this reads the Git plugin's code: it asks the shared context,
    /// which is filled from the contract bus.
    fn attach_git_context(&mut self) -> bool {
        let Some(attachment) = self.git.attachment() else {
            return false;
        };
        self.messages.push(Message {
            author: Author::Context,
            text: attachment,
        });
        true
    }

    /// Append what the composer holds and clear it.
    ///
    /// The reply is a local echo: this is the send/receive seam, not an agent
    /// orchestrator. An agent provider, a session protocol or ACP integration
    /// replaces [`Self::reply_to`] without touching the panel's UI.
    fn send(&mut self) -> bool {
        let text = self.draft.trim().to_owned();
        if text.is_empty() {
            return false;
        }

        self.draft.clear();
        self.messages.push(Message {
            author: Author::You,
            text: text.clone(),
        });
        if let Some(reply) = Self::reply_to(&text) {
            self.messages.push(reply);
        }
        true
    }

    /// The stand-in for an agent: acknowledge what was said.
    fn reply_to(text: &str) -> Option<Message> {
        let first_line = text.lines().next().unwrap_or_default();
        Some(Message {
            author: Author::Agent,
            text: format!("You said: \"{first_line}\"."),
        })
    }
}

impl ChatPanel {
    /// The one-line Git status: what another plugin is publishing right now.
    fn git_line(&self) -> Div {
        match (self.git.summary(), self.git.is_connected()) {
            (Some(summary), _) => div()
                .flex()
                .flex_row()
                .gap(px(8.))
                .child(div().text_color(rgb(0xe2c08d)).child("Git"))
                .child(div().text_color(rgb(0x8b93a1)).child(summary))
                .child(div().text_color(rgb(0x6f7684)).child("F2 to attach")),
            (None, false) => div()
                .text_color(rgb(0x6f7684))
                .child("Git: no plugin publishing a repository"),
            (None, true) => div().text_color(rgb(0x6f7684)).child("Git: connecting"),
        }
    }
}

impl Panel for ChatPanel {
    fn panel_name(&self) -> &'static str {
        PANEL_NAME
    }

    fn dump(&self, _: &App) -> PanelState {
        let saved = Saved {
            messages: self.messages.clone(),
        };
        let mut state = PanelState::new(PANEL_NAME);
        // The host passes this payload straight back on restore; it never parses
        // it, so the schema here is the plugin's alone.
        state.info =
            PanelInfo::Panel(serde_json::to_value(saved).unwrap_or(serde_json::Value::Null));
        state
    }
}

impl EventEmitter<PanelEvent> for ChatPanel {}

impl Focusable for ChatPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // The host wrapper holds the keyboard for a plugin panel; handing out a
        // fresh handle keeps the dock's focus queries harmless.
        cx.focus_handle()
    }
}

impl Render for ChatPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let lines: Vec<Div> = self
            .messages
            .iter()
            .map(|message| {
                let (label, color) = match message.author {
                    Author::You => ("You", rgb(0x9ecbff)),
                    Author::Agent => ("Agent", rgb(0x7dd3a0)),
                    Author::Context => ("Git context", rgb(0xe2c08d)),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_color(color).child(label))
                    .child(div().child(message.text.clone()))
            })
            .collect();

        let empty = self.messages.is_empty().then(|| {
            div()
                .text_color(rgb(0x6f7684))
                .child("No messages yet. Type below and press Enter.")
        });

        let footer = if self.draft.is_empty() {
            "Message the agent…".to_owned()
        } else {
            format!("{}▏", self.draft)
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Agent Chat"))
                    .child(div().text_color(rgb(0x6f7684)).child(format!(
                        "instance {} · key {}",
                        self.instance,
                        self.last_key.clone().unwrap_or_else(|| "-".to_owned())
                    ))),
            )
            .child(self.git_line())
            .child(
                div()
                    .id("transcript")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .overflow_y_scroll()
                    .children(lines)
                    .children(empty),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .bg(rgb(0x1c2027))
                            .border_1()
                            .border_color(rgb(0x333a45))
                            .text_color(if self.draft.is_empty() {
                                rgb(0x6f7684)
                            } else {
                                rgb(0xe4e7ec)
                            })
                            .child(footer),
                    )
                    .child(div().text_color(rgb(0x6f7684)).child("Enter to send")),
            )
    }
}
