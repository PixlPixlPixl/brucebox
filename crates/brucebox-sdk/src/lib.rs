//! The Brucebox SDK: what a plugin project depends on.
//!
//! A plugin is a standalone Rust `cdylib` that depends on this crate, declares
//! its contributions with [`Plugin`], and exports exactly one symbol with
//! [`declare_plugin!`]. The host discovers the built library through a plugin
//! folder, loads it, checks the API version, and calls the entry point; nothing
//! about the plugin is compiled into Brucebox and no host source change is
//! needed to add one.
//!
//! ```ignore
//! use brucebox_sdk::prelude::*;
//!
//! struct Hello;
//! impl Render for Hello { /* an ordinary GPUI view */ }
//!
//! fn create(_: &mut Window, cx: &mut App) -> std::sync::Arc<dyn PanelView> {
//!     Arc::new(cx.new(|cx| Hello))
//! }
//!
//! declare_plugin!(Plugin::new("com.example.hello", "Hello", "0.1.0")
//!     .panel("main", "Hello", create));
//! ```
//!
//! # What the host gives a plugin
//!
//! - The pinned GPUI / GPUI Kit the host itself uses (`gpui_kit`), so a panel is
//!   a normal GPUI entity: custom `Render`, elements, painting, focus, animation,
//!   whatever the plugin needs. There is no component library requirement.
//! - The pinned Cordis types (`cordis`), so a runtime contribution joins the
//!   host's one composition runtime instead of creating its own.
//!
//! # What a plugin must match
//!
//! The toolchain in `rust-toolchain.toml`, the `gpui-kit` and `cordis-rs`
//! versions this crate pins, and [`api::API_VERSION`]. The host rejects a
//! mismatched API version before it calls anything; it cannot inspect the
//! toolchain of an arbitrary plugin, so matching it is a documented requirement
//! of building one. See `brucebox-plugin-api` for the full contract.

use std::sync::Arc;

use brucebox_plugin_api as api;

pub use brucebox_plugin_api as plugin_api;

pub mod contracts;

use std::sync::OnceLock;

use brucebox_plugin_api::HostContext;

/// The context the host handed the library when it loaded it.
///
/// Set once, by [`declare_plugin!`], before the registration is built. A plugin
/// that was loaded without a context (tooling, a test) simply has no bus.
static HOST: OnceLock<HostContext> = OnceLock::new();

/// Keep the host context for the life of the process.
///
/// Called by [`declare_plugin!`]; a plugin never calls this itself. A context from
/// another API version is ignored rather than half-used, and the plugin's bus calls
/// then report that the host did not provide one.
/// Keep the host context the entry point was called with.
///
/// # Safety
///
/// `context` must be null or point at a [`HostContext`] that is valid for the
/// duration of the call. This is the one dereference the entry point performs; it
/// lives here so the generated symbol itself stays a plain forwarding function.
pub unsafe fn capture_host_from(context: *const HostContext) {
    if context.is_null() {
        return;
    }
    // SAFETY: the caller guarantees the pointer is valid for this call, and the
    // context is copied out of it immediately.
    let context = unsafe { *context };
    capture_host(&context);
}

pub fn capture_host(context: &HostContext) {
    if context.api_version != brucebox_plugin_api::API_VERSION {
        return;
    }
    let _ = HOST.set(*context);
}
pub use cordis;
pub use gpui_kit;

/// The plugin API version this SDK builds against.
pub const API_VERSION: u32 = api::API_VERSION;

/// The SDK version a plugin declares in its manifest.
pub const SDK_VERSION: &str = "0.1";

/// A plugin under construction.
///
/// Contributions are collected here and sealed into the single
/// [`api::PluginRegistration`] the entry point returns.
pub struct Plugin {
    plugin_id: &'static str,
    display_name: &'static str,
    version: &'static str,
    panels: Vec<api::PanelContribution>,
    runtime: Option<api::RuntimeContribution>,
}

impl std::fmt::Debug for Plugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Plugin")
            .field("plugin_id", &self.plugin_id)
            .field("version", &self.version)
            .field("panels", &self.panels.len())
            .field("runtime", &self.runtime.is_some())
            .finish()
    }
}

impl Plugin {
    /// Start declaring a plugin.
    ///
    /// `plugin_id` must be namespaced and stable: it is what panel type ids and
    /// saved layouts are built from.
    pub fn new(plugin_id: &'static str, display_name: &'static str, version: &'static str) -> Self {
        Self {
            plugin_id,
            display_name,
            version,
            panels: Vec::new(),
            runtime: None,
        }
    }

    /// Contribute one single-instance panel.
    pub fn panel(
        self,
        panel_segment: &'static str,
        title: &'static str,
        create: api::PanelFactory,
    ) -> Self {
        self.panel_with(panel_segment, title, api::PanelMultiplicity::Single, create)
    }

    /// Contribute one multi-instance panel.
    pub fn multi_instance_panel(
        self,
        panel_segment: &'static str,
        title: &'static str,
        create: api::PanelFactory,
    ) -> Self {
        self.panel_with(
            panel_segment,
            title,
            api::PanelMultiplicity::Multiple,
            create,
        )
    }

    fn panel_with(
        mut self,
        panel_segment: &'static str,
        title: &'static str,
        multiplicity: api::PanelMultiplicity,
        create: api::PanelFactory,
    ) -> Self {
        self.panels.push(api::PanelContribution {
            panel_segment,
            title,
            multiplicity,
            create,
        });
        self
    }

    /// Contribute to the host's Cordis runtime.
    ///
    /// The function is called once, after the library loads. It returns the
    /// plugin's own [`cordis::PreparedPlugin`]; the host owns the resulting fiber,
    /// its services, events and cleanup. Never create a runtime here.
    pub fn runtime(mut self, contribution: api::RuntimeContribution) -> Self {
        self.runtime = Some(contribution);
        self
    }

    /// Seal the declaration for the host.
    ///
    /// The result is leaked on purpose: the host holds it for the lifetime of the
    /// process, and it must stay valid as long as the library is loaded. A
    /// malformed declaration panics here, at load time, with the reason — it is a
    /// bug in the plugin, and failing loudly at the boundary is better than
    /// registering something the host would misinterpret.
    #[must_use]
    pub fn registration(self) -> &'static api::PluginRegistration {
        let panels: &'static [api::PanelContribution] = if self.panels.is_empty() {
            &[]
        } else {
            Box::leak(self.panels.into_boxed_slice())
        };

        let registration = Box::leak(Box::new(api::PluginRegistration {
            plugin_id: self.plugin_id,
            display_name: self.display_name,
            version: self.version,
            api_version: api::API_VERSION,
            panels,
            runtime: self.runtime,
        }));

        if let Err(reason) = api::validate(registration) {
            panic!(
                "invalid plugin declaration for `{}`: {reason}",
                self.plugin_id
            );
        }

        registration
    }
}

/// Export the plugin entry point the host looks for.
///
/// Use it once, at the crate root of the plugin library:
///
/// ```ignore
/// declare_plugin!(Plugin::new("com.example.hello", "Hello", "0.1.0")
///     .panel("main", "Hello", create));
/// ```
#[macro_export]
macro_rules! declare_plugin {
    ($plugin:expr) => {
        /// The one symbol the host resolves in this library.
        ///
        /// The host passes its context once; the SDK keeps a copy for the bus and
        /// then answers with the registration this library describes.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn brucebox_plugin_entry_v2(
            context: *const $crate::plugin_api::HostContext,
        ) -> *const $crate::plugin_api::PluginRegistration {
            // SAFETY: the host passes a context that is valid for this call, and
            // the SDK copies what it needs out of it.
            unsafe { $crate::capture_host_from(context) };
            static REGISTRATION: ::std::sync::OnceLock<
                &'static $crate::plugin_api::PluginRegistration,
            > = ::std::sync::OnceLock::new();
            *REGISTRATION.get_or_init(|| ($plugin).registration())
        }
    };
}

/// Wraps a GPUI entity as the panel handle the host needs.
///
/// `Entity<T>` becomes a panel handle through GPUI Base's blanket `PanelView`
/// implementation for `Panel` entities, and this is the only conversion a
/// plugin usually needs. A view-only panel gets no forwarded input.
pub fn panel_view<T>(entity: gpui_kit::Entity<T>) -> api::PanelParts
where
    T: gpui_kit::base::dock::Panel,
{
    api::PanelParts::view_only(Arc::new(entity))
}

/// Wraps a GPUI entity as a panel that also receives forwarded input.
///
/// `on_input` runs on the application thread whenever the host forwards a key
/// or pointer event for this panel — the supported way for a plugin panel to be
/// interactive (see [`api::PluginPanelInput`] for why GPUI's own listeners do
/// not reach across the boundary).
pub fn interactive_panel<T, F>(entity: gpui_kit::Entity<T>, on_input: F) -> api::PanelParts
where
    T: gpui_kit::base::dock::Panel + Send + Sync + 'static,
    F: Fn(&mut T, &api::PluginInput<'_>, &mut gpui_kit::Window, &mut gpui_kit::Context<T>)
        + Send
        + Sync
        + 'static,
{
    /// The `Arc<dyn PluginPanelInput>` the host calls, holding the entity that
    /// owns the panel's state.
    struct Forwarded<T, F> {
        entity: gpui_kit::Entity<T>,
        on_input: F,
    }

    impl<T, F> api::PluginPanelInput for Forwarded<T, F>
    where
        T: gpui_kit::base::dock::Panel + Send + Sync + 'static,
        F: Fn(&mut T, &api::PluginInput<'_>, &mut gpui_kit::Window, &mut gpui_kit::Context<T>)
            + Send
            + Sync
            + 'static,
    {
        fn handle_input(
            &self,
            input: &api::PluginInput<'_>,
            window: &mut gpui_kit::Window,
            cx: &mut gpui_kit::App,
        ) {
            self.entity.update(cx, |panel, cx| {
                (self.on_input)(panel, input, window, cx);
            });
        }
    }

    api::PanelParts {
        view: Arc::new(entity.clone()),
        input: Some(Arc::new(Forwarded { entity, on_input })),
    }
}

/// The imports a plugin normally needs.
///
/// GPUI is re-exported by its real crate name too, because its derive macros
/// resolve `::gpui_kit` paths in the crate that uses them; keep `gpui-kit` as a
/// direct dependency of the plugin project, pinned to the same version.
pub mod prelude {
    pub use crate::contracts;
    pub use crate::{Plugin, declare_plugin, interactive_panel, panel_view};
    pub use brucebox_plugin_api::{
        KeyInput, PanelContribution, PanelMultiplicity, PanelOpen, PanelParts, PluginInput,
        PluginPanelInput, PluginRegistration, PointerButton, PointerInput, PointerPhase,
    };
    pub use cordis;
    pub use gpui_kit;
    pub use gpui_kit::base::dock::{Panel, PanelEvent, PanelInfo, PanelState, PanelView};
    pub use gpui_kit::*;
}

#[cfg(test)]
mod tests {
    use super::*;
    use brucebox_plugin_api::PanelMultiplicity;

    fn create(
        _: &brucebox_plugin_api::PanelOpen<'_>,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::App,
    ) -> brucebox_plugin_api::PanelParts {
        unreachable!("the test never builds a panel")
    }

    #[test]
    fn a_builder_seals_what_the_host_reads() {
        let registration = Plugin::new("com.example.hello", "Hello", "0.1.0")
            .panel("main", "Hello", create)
            .multi_instance_panel("notes", "Notes", create)
            .registration();

        assert_eq!(registration.plugin_id, "com.example.hello");
        assert_eq!(registration.display_name, "Hello");
        assert_eq!(registration.version, "0.1.0");
        assert_eq!(registration.api_version, API_VERSION);
        assert_eq!(registration.panels.len(), 2);
        assert_eq!(registration.panels[0].panel_segment, "main");
        assert_eq!(
            registration.panels[0].multiplicity,
            PanelMultiplicity::Single
        );
        assert_eq!(
            registration.panels[1].multiplicity,
            PanelMultiplicity::Multiple
        );
        assert!(registration.runtime.is_none());
    }

    #[test]
    fn a_runtime_only_plugin_declares_no_panels() {
        fn runtime() -> cordis::PreparedPlugin {
            unreachable!("the test never admits a plugin")
        }

        let registration = Plugin::new("com.example.headless", "Headless", "0.1.0")
            .runtime(runtime)
            .registration();

        assert!(registration.panels.is_empty());
        assert!(registration.runtime.is_some());
    }

    #[test]
    #[should_panic(expected = "invalid plugin declaration")]
    fn a_malformed_declaration_fails_at_the_boundary_rather_than_registering() {
        let _ = Plugin::new("not-namespaced", "Broken", "0.1.0").registration();
    }
}
