//! The Brucebox plugin ABI boundary.
//!
//! This crate is the *only* place a native plugin and the host agree on a
//! layout. Everything in it is `#[repr(C)]` plain data plus function pointers
//! whose signatures name a small, explicitly pinned set of host types.
//!
//! # What is promised, and what is not
//!
//! Rust has no stable general ABI: a `struct`, a trait object or an `Arc` from
//! two different compiler builds are not guaranteed to agree. This boundary
//! therefore promises compatibility only for plugins built with:
//!
//! - the same pinned Rust toolchain as the host (`rust-toolchain.toml`),
//! - the same pinned `gpui-kit` / `cordis-rs` versions the host uses,
//! - the same plugin API version ([`API_VERSION`]).
//!
//! The host checks the declared API version before it calls into a library and
//! rejects a mismatch. It cannot check the toolchain or the dependency versions
//! of an arbitrary plugin, so those are a documented requirement of building
//! one, not a sandbox. A native plugin is trusted in-process code with the
//! privileges of Brucebox.
//!
//! # Why function pointers instead of a data protocol
//!
//! A panel is a real GPUI entity, and full native GPUI freedom is a requirement
//! (SPEC §5.4). Serializing UI descriptions would give up exactly the capability
//! the plugin system exists for. So [`PanelContribution::create`] is a function
//! in the plugin's library that the host calls on its own application thread,
//! and it returns the host's own panel trait object. The plugin's code and the
//! host's `App` are the same process, the same thread, and the same pinned
//! GPUI build; only the code that runs is separately compiled.
//!
//! # Lifetime rule
//!
//! Every pointer and `&'static` value a library hands the host outlives the
//! library handle: the loader keeps the handle loaded until process exit, and
//! the host disposes plugin-owned objects before that. Nothing here may be
//! copied out and used after its library is unloaded — see
//! `brucebox-plugin-loader`.

use std::fmt;

use cordis::PreparedPlugin;
use gpui_kit::base::dock::PanelView;
use gpui_kit::{App, Window};

/// The plugin API version this crate describes.
///
/// A plugin declares the version it was built against; the host refuses any
/// other version rather than calling into a library whose layout it does not
/// share.
pub const API_VERSION: u32 = 1;

/// The symbol a plugin library must export, including its terminating NUL.
pub const ENTRY_SYMBOL: &[u8] = b"brucebox_plugin_entry_v1\0";

/// The symbol name as a string, for diagnostics.
pub const ENTRY_SYMBOL_NAME: &str = "brucebox_plugin_entry_v1";

/// How many live instances one panel type supports.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelMultiplicity {
    /// At most one live instance.
    Single = 0,
    /// Any number of independent instances.
    Multiple = 1,
}

/// What the host tells a plugin when it builds one panel.
///
/// `restored` is the plugin's *own* saved payload — the JSON its `dump` put in
/// `PanelState::info` — handed back as text so the host does not need to
/// understand the plugin's schema, and so a corrupt or foreign payload can be
/// refused by the plugin that owns it.
#[repr(C)]
pub struct PanelOpen<'a> {
    /// The unique instance id the workspace assigned to this panel.
    pub instance: u64,
    /// The plugin's own saved payload, if this panel is being restored.
    pub restored: Option<&'a str>,
}

/// One key the host forwarded to a plugin panel.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KeyInput<'a> {
    /// The key name, as GPUI reports it (`enter`, `backspace`, `a`, `left`).
    pub key: &'a str,
    /// The printable text this key produces, if any.
    pub key_char: Option<&'a str>,
    /// `ctrl` was held.
    pub control: bool,
    /// `alt` was held.
    pub alt: bool,
    /// `shift` was held.
    pub shift: bool,
    /// The platform modifier (command/super) was held.
    pub platform: bool,
}

/// Which pointer button an input event is about.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// The primary button.
    Left,
    /// The secondary button.
    Right,
    /// The middle button.
    Middle,
}

/// Whether a pointer event is a press or a release.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    /// Pressed.
    Down,
    /// Released.
    Up,
}

/// One pointer event, in window coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PointerInput {
    /// Horizontal position in the window.
    pub x: f32,
    /// Vertical position in the window.
    pub y: f32,
    /// Which button.
    pub button: PointerButton,
    /// Press or release.
    pub phase: PointerPhase,
}

/// One input event the host forwards to a plugin panel.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum PluginInput<'a> {
    /// A key event.
    Key(KeyInput<'a>),
    /// A pointer event.
    Pointer(PointerInput),
}

/// Receives the input the host forwards to one plugin panel.
///
/// # Why a plugin panel does not use GPUI's own listeners
///
/// GPUI routes an event to a listener by downcasting it to its concrete type
/// inside `Any`. Across two separately compiled libraries the same type has two
/// `TypeId`s (the crate's build metadata differs because the dependency graph
/// differs), so that downcast fails and a plugin's own `on_key_down` or
/// `on_click` would silently never run. That is not something a plugin can work
/// around, and it is why the host owns the input handlers: it reads the event on
/// its side, turns it into the plain data below, and calls this method.
pub trait PluginPanelInput: Send + Sync {
    /// Handle one forwarded event.
    fn handle_input(&self, input: &PluginInput<'_>, window: &mut Window, cx: &mut App);
}

/// What a plugin's panel factory returns.
///
/// The view is what the workspace shows; `input`, when present, is how the panel
/// hears about keys and clicks (see [`PluginPanelInput`]).
#[repr(C)]
pub struct PanelParts {
    /// The plugin's own panel view.
    pub view: std::sync::Arc<dyn PanelView>,
    /// The panel's input receiver, if it wants input at all.
    pub input: Option<std::sync::Arc<dyn PluginPanelInput>>,
}

impl PanelParts {
    /// A panel that renders but wants no forwarded input.
    pub fn view_only(view: std::sync::Arc<dyn PanelView>) -> Self {
        Self { view, input: None }
    }
}

impl std::fmt::Debug for PanelParts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PanelParts")
            .field("input", &self.input.is_some())
            .finish_non_exhaustive()
    }
}

/// Builds one live panel.
///
/// The host calls this on its application thread, with its own window and
/// application context, and expects the plugin's panel and input receiver back.
/// `Arc` (not `Rc`) is what the dock engine uses, and `dyn PanelView` is that
/// engine's own trait — the plugin is free to render anything behind it.
pub type PanelFactory = fn(&PanelOpen<'_>, &mut Window, &mut App) -> PanelParts;

/// Builds the plugin's own contribution to the host's Cordis runtime.
///
/// Called once, on the host thread, after the library has loaded and before the
/// runtime admits it. The returned [`PreparedPlugin`] is the plugin *sealing*
/// its own behavior and input; the host owns the resulting fiber, so a plugin
/// never creates a runtime of its own (SPEC §3.B).
pub type RuntimeContribution = fn() -> PreparedPlugin;

/// One panel type a plugin contributes.
#[repr(C)]
pub struct PanelContribution {
    /// The panel segment, without the plugin namespace (`chat`, not
    /// `brucebox.agent-chat.chat`). Must be a single id segment.
    pub panel_segment: &'static str,
    /// Default tab title.
    pub title: &'static str,
    /// How many live instances are allowed.
    pub multiplicity: PanelMultiplicity,
    /// The code that builds one.
    pub create: PanelFactory,
}

/// Everything a plugin library tells the host about itself.
///
/// Returned by the library's entry point as a `&'static` value, so it stays
/// valid for as long as the library is loaded.
#[repr(C)]
pub struct PluginRegistration {
    /// Stable plugin id, namespaced (`brucebox.agent-chat`).
    pub plugin_id: &'static str,
    /// Human-readable name.
    pub display_name: &'static str,
    /// Plugin version, as a string the plugin chooses.
    pub version: &'static str,
    /// The [`API_VERSION`] the plugin was built against.
    pub api_version: u32,
    /// The panels this plugin contributes. May be empty: a plugin may be
    /// runtime-only.
    pub panels: &'static [PanelContribution],
    /// The plugin's own Cordis contribution, if it has one. A UI-only plugin
    /// contributes nothing here.
    pub runtime: Option<RuntimeContribution>,
}

impl fmt::Debug for PluginRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginRegistration")
            .field("plugin_id", &self.plugin_id)
            .field("version", &self.version)
            .field("api_version", &self.api_version)
            .field("panels", &self.panels.len())
            .field("runtime", &self.runtime.is_some())
            .finish()
    }
}

/// The exported entry point every plugin library defines.
pub type PluginEntry = extern "C" fn() -> *const PluginRegistration;

/// Why a plugin's registration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// The plugin was built against a different API version.
    ApiMismatch {
        /// The version the plugin declares.
        plugin: u32,
        /// The version this host implements.
        host: u32,
    },
    /// A required string is empty or malformed.
    InvalidField {
        /// Which field.
        field: &'static str,
        /// Why it was refused.
        reason: &'static str,
    },
    /// Two panels claim the same id.
    DuplicatePanel(String),
    /// A panel's segment is not a single id segment.
    InvalidPanelSegment {
        /// The offending segment.
        segment: &'static str,
        /// Why it was refused.
        reason: &'static str,
    },
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiMismatch { plugin, host } => write!(
                formatter,
                "plugin API version {plugin} does not match host API version {host}"
            ),
            Self::InvalidField { field, reason } => write!(formatter, "`{field}` {reason}"),
            Self::DuplicatePanel(panel) => {
                write!(formatter, "panel type `{panel}` is declared twice")
            }
            Self::InvalidPanelSegment { segment, reason } => {
                write!(formatter, "panel segment `{segment}` {reason}")
            }
        }
    }
}

impl std::error::Error for RegistrationError {}

/// Check one registration before the host trusts anything in it.
///
/// This is the ABI-level gate: version, identity and shape. The catalog applies
/// its own rules on top (namespaced ids, ownership, multiplicity), so a plugin
/// cannot get panels into the workspace by declaring them cleverly here.
pub fn validate(registration: &PluginRegistration) -> Result<(), RegistrationError> {
    if registration.api_version != API_VERSION {
        return Err(RegistrationError::ApiMismatch {
            plugin: registration.api_version,
            host: API_VERSION,
        });
    }

    validate_id(registration.plugin_id)?;
    validate_text("display_name", registration.display_name)?;
    validate_text("version", registration.version)?;

    let mut seen: Vec<&str> = Vec::with_capacity(registration.panels.len());
    for panel in registration.panels {
        validate_segment(panel.panel_segment)?;
        validate_text("title", panel.title)?;
        if seen.contains(&panel.panel_segment) {
            return Err(RegistrationError::DuplicatePanel(
                panel.panel_segment.to_owned(),
            ));
        }
        seen.push(panel.panel_segment);
    }

    Ok(())
}

/// A namespaced plugin id: lowercase id characters and at least one dot.
fn validate_id(id: &str) -> Result<(), RegistrationError> {
    if id.is_empty() {
        return Err(RegistrationError::InvalidField {
            field: "plugin_id",
            reason: "must not be empty",
        });
    }
    if !id.contains('.') {
        return Err(RegistrationError::InvalidField {
            field: "plugin_id",
            reason: "must be namespaced, for example `brucebox.agent-chat`",
        });
    }
    if !id.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || "._-".contains(character)
    }) {
        return Err(RegistrationError::InvalidField {
            field: "plugin_id",
            reason: "may only contain lowercase letters, digits, `.`, `_` and `-`",
        });
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str) -> Result<(), RegistrationError> {
    if value.trim().is_empty() {
        return Err(RegistrationError::InvalidField {
            field,
            reason: "must not be empty",
        });
    }
    Ok(())
}

fn validate_segment(segment: &'static str) -> Result<(), RegistrationError> {
    if segment.is_empty() {
        return Err(RegistrationError::InvalidPanelSegment {
            segment,
            reason: "must not be empty",
        });
    }
    if segment.contains('.') {
        return Err(RegistrationError::InvalidPanelSegment {
            segment,
            reason: "is a single segment: the plugin namespace is added for you",
        });
    }
    if !segment.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || "_-".contains(character)
    }) {
        return Err(RegistrationError::InvalidPanelSegment {
            segment,
            reason: "may only contain lowercase letters, digits, `_` and `-`",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(_: &PanelOpen<'_>, _: &mut Window, _: &mut App) -> PanelParts {
        unreachable!("registration validation never calls a factory")
    }

    static PANELS: [PanelContribution; 1] = [PanelContribution {
        panel_segment: "chat",
        title: "Chat",
        multiplicity: PanelMultiplicity::Single,
        create,
    }];

    fn registration() -> PluginRegistration {
        PluginRegistration {
            plugin_id: "brucebox.agent-chat",
            display_name: "Agent Chat",
            version: "0.1.0",
            api_version: API_VERSION,
            panels: &PANELS,
            runtime: None,
        }
    }

    #[test]
    fn a_well_formed_registration_is_accepted() {
        assert_eq!(validate(&registration()), Ok(()));
    }

    #[test]
    fn a_plugin_built_against_another_api_version_is_refused_before_anything_else() {
        let mut registration = registration();
        registration.api_version = API_VERSION + 1;
        registration.plugin_id = "not a valid id";

        assert_eq!(
            validate(&registration),
            Err(RegistrationError::ApiMismatch {
                plugin: API_VERSION + 1,
                host: API_VERSION,
            }),
            "the version gate comes first: the rest of the struct may not even be safe to read"
        );
    }

    #[test]
    fn plugin_ids_must_be_namespaced_and_well_formed() {
        for id in ["", "agent-chat", "brucebox.Agent-Chat", "brucebox agent"] {
            let mut registration = registration();
            registration.plugin_id = id;
            assert!(
                matches!(
                    validate(&registration),
                    Err(RegistrationError::InvalidField {
                        field: "plugin_id",
                        ..
                    })
                ),
                "`{id}` must be refused"
            );
        }
    }

    #[test]
    fn a_panel_segment_is_a_segment_not_a_type_id() {
        static BAD: [PanelContribution; 1] = [PanelContribution {
            panel_segment: "brucebox.agent-chat.chat",
            title: "Chat",
            multiplicity: PanelMultiplicity::Single,
            create,
        }];

        let mut registration = registration();
        registration.panels = &BAD;
        assert!(matches!(
            validate(&registration),
            Err(RegistrationError::InvalidPanelSegment { .. })
        ));
    }

    #[test]
    fn the_same_panel_declared_twice_is_refused() {
        static TWICE: [PanelContribution; 2] = [
            PanelContribution {
                panel_segment: "chat",
                title: "Chat",
                multiplicity: PanelMultiplicity::Single,
                create,
            },
            PanelContribution {
                panel_segment: "chat",
                title: "Chat again",
                multiplicity: PanelMultiplicity::Single,
                create,
            },
        ];

        let mut registration = registration();
        registration.panels = &TWICE;
        assert_eq!(
            validate(&registration),
            Err(RegistrationError::DuplicatePanel("chat".to_owned()))
        );
    }
}
