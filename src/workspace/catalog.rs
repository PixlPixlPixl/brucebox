//! The host's panel catalog: what kinds of panel exist, who owns them, and how
//! to create one.
//!
//! The catalog is the only place the workspace learns about panels. It knows a
//! panel through a [`PanelDescriptor`] — a stable type id, the owning plugin,
//! presentation metadata, a multiplicity policy, and a factory — and never
//! through a concrete panel implementation. Plugins register descriptors;
//! the workspace resolves type ids from layouts against them.
//!
//! Two registries exist and they are deliberately distinct:
//!
//! - This catalog is the Brucebox-level registry: ownership, metadata,
//!   multiplicity, plugin failure and missing-panel policy.
//! - GPUI Base Dock's own [`PanelRegistry`](gpui_kit::base::dock::PanelRegistry)
//!   maps a persisted `panel_name` to a builder. [`register_contribution`]
//!   bridges one into the other, so a panel restored from a layout comes back
//!   through exactly the same factory as a freshly opened one.
//!
//! Duplicate ids are rejected, not resolved: a plugin may not shadow another
//! plugin's panel type, and a panel type may only be registered once. A
//! contribution whose plugin is not registered is rejected too, which is what
//! makes "the plugin failed to load" visible as absent panels rather than as
//! panels from nowhere.

use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{PanelState, PanelView, register_panel as register_dock_panel};
use gpui_kit::{App, Global, SharedString, Window};

pub use super::ids::{PanelIdentity, PanelInstanceId, PanelTypeId, PluginId};

use super::declaration::PluginDeclaration;

/// How many live instances one panel type supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelMultiplicity {
    /// At most one live instance; a second placement is refused.
    #[default]
    Single,
    /// Any number of independent instances.
    Multiple,
}

impl PanelMultiplicity {
    /// The largest number of live instances allowed, or `None` for unbounded.
    pub fn max_instances(self) -> Option<usize> {
        match self {
            Self::Single => Some(1),
            Self::Multiple => None,
        }
    }

    /// Whether one more instance may be created while `open` are live.
    pub fn allows(self, open: usize) -> bool {
        self.max_instances().is_none_or(|max| open < max)
    }
}

/// Everything a [`PanelFactory`] is told about the panel it is creating.
pub struct PanelCreateRequest {
    identity: PanelIdentity,
    restored: Option<PanelState>,
}

impl PanelCreateRequest {
    /// The type and instance id the new panel must report.
    pub fn identity(&self) -> &PanelIdentity {
        &self.identity
    }

    /// The type of panel being created.
    pub fn panel_type(&self) -> &PanelTypeId {
        self.identity.panel_type()
    }

    /// The unique id of the panel being created.
    pub fn instance(&self) -> PanelInstanceId {
        self.identity.instance()
    }

    /// Persisted state handed back by GPUI Base Dock, when this panel is being
    /// restored rather than opened fresh.
    ///
    /// A panel that has nothing to restore from gets `None` and starts from
    /// its own defaults.
    pub fn restored_state(&self) -> Option<&PanelState> {
        self.restored.as_ref()
    }
}

/// Why a panel could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelCreateError {
    /// No descriptor is registered for this panel type. This is the shape a
    /// missing or disabled plugin takes by the time it reaches the workspace.
    UnknownPanelType(PanelTypeId),
    /// The owning plugin's factory refused or panicked its way out of building
    /// the panel.
    FactoryFailed {
        panel: PanelTypeId,
        reason: SharedString,
    },
}

impl PanelCreateError {
    /// Report a failure from inside a plugin's factory.
    pub fn factory_failed(panel: &PanelTypeId, reason: impl Into<SharedString>) -> Self {
        Self::FactoryFailed {
            panel: panel.clone(),
            reason: reason.into(),
        }
    }

    /// The panel type that failed.
    pub fn panel_type(&self) -> &PanelTypeId {
        match self {
            Self::UnknownPanelType(panel) => panel,
            Self::FactoryFailed { panel, .. } => panel,
        }
    }
}

impl fmt::Display for PanelCreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPanelType(panel) => {
                write!(f, "no contribution is registered for panel type `{panel}`")
            }
            Self::FactoryFailed { panel, reason } => {
                write!(f, "panel `{panel}` could not be created: {reason}")
            }
        }
    }
}

impl std::error::Error for PanelCreateError {}

/// Builds one live panel.
///
/// Implemented for any closure with the right shape, so a contribution is a
/// factory call plus a descriptor. The result is handed to GPUI Base Dock as a
/// [`PanelView`]; see [`crate::workspace::panels::host::HostPanel`] for the
/// handle that keeps the catalog's identity and the panel's own entity
/// together.
pub trait PanelFactory: 'static {
    fn create(
        &self,
        request: &PanelCreateRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Arc<dyn PanelView>, PanelCreateError>;
}

impl<F> PanelFactory for F
where
    F: Fn(
            &PanelCreateRequest,
            &mut Window,
            &mut App,
        ) -> Result<Arc<dyn PanelView>, PanelCreateError>
        + 'static,
{
    fn create(
        &self,
        request: &PanelCreateRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Arc<dyn PanelView>, PanelCreateError> {
        self(request, window, cx)
    }
}

/// Everything the host knows about one kind of panel.
pub struct PanelDescriptor {
    plugin: PluginId,
    panel_type: PanelTypeId,
    title: SharedString,
    icon: Option<SharedString>,
    multiplicity: PanelMultiplicity,
    factory: Rc<dyn PanelFactory>,
}

impl PanelDescriptor {
    /// Describe a panel type owned by `plugin`.
    ///
    /// `name` is the panel segment only; the type id is namespaced under the
    /// plugin id. The panel starts out single-instance; call
    /// [`Self::with_multiplicity`] to allow more.
    pub fn new(
        plugin: PluginId,
        name: &str,
        title: impl Into<SharedString>,
        factory: Rc<dyn PanelFactory>,
    ) -> Result<Self, crate::workspace::ids::IdError> {
        let panel_type = PanelTypeId::new(&plugin, name)?;
        Ok(Self {
            plugin,
            panel_type,
            title: title.into(),
            icon: None,
            multiplicity: PanelMultiplicity::default(),
            factory,
        })
    }

    /// The plugin that owns this contribution.
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// The stable panel type id.
    pub fn panel_type(&self) -> &PanelTypeId {
        &self.panel_type
    }

    /// Default tab title.
    pub fn title(&self) -> &SharedString {
        &self.title
    }

    /// Optional icon reference, for a presentation layer that wants one.
    pub fn icon(&self) -> Option<&SharedString> {
        self.icon.as_ref()
    }

    /// How many live instances this panel type supports.
    pub fn multiplicity(&self) -> PanelMultiplicity {
        self.multiplicity
    }

    /// Whether one more instance may be created while `open` are live.
    pub fn allows_additional_instance(&self, open: usize) -> bool {
        self.multiplicity.allows(open)
    }

    /// Attach an icon reference.
    pub fn with_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Change how many instances this panel type supports.
    pub fn with_multiplicity(mut self, multiplicity: PanelMultiplicity) -> Self {
        self.multiplicity = multiplicity;
        self
    }

    /// Create one live panel through the plugin's factory.
    pub fn create(
        &self,
        instance: PanelInstanceId,
        restored: Option<PanelState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Arc<dyn PanelView>, PanelCreateError> {
        let request = PanelCreateRequest {
            identity: PanelIdentity::new(self.panel_type.clone(), instance),
            restored,
        };
        self.factory.create(&request, window, cx)
    }
}

/// A plugin the host has accepted contributions from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRegistration {
    id: PluginId,
    display_name: SharedString,
}

impl PluginRegistration {
    pub fn new(id: PluginId, display_name: impl Into<SharedString>) -> Self {
        Self {
            id,
            display_name: display_name.into(),
        }
    }

    pub fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn display_name(&self) -> &SharedString {
        &self.display_name
    }
}

/// Why a registration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The plugin id is already registered.
    DuplicatePlugin(PluginId),
    /// A panel named a plugin that has not been registered, so the panel cannot
    /// be owned. A failed plugin load looks like this.
    UnknownPlugin(PluginId),
    /// The panel type id is already registered by this or another plugin.
    DuplicatePanel(PanelTypeId),
    /// Nothing is registered for this panel type.
    UnknownPanel(PanelTypeId),
    /// A declared panel has no factory on the application thread.
    ///
    /// A declaration names a panel; the code that builds it is supplied by
    /// whoever runs the plugin (today the host, from the plugin loader once it
    /// exists). Without a factory the contribution cannot be accepted, and
    /// failing here is better than accepting a panel nobody can create.
    MissingPanelFactory(PanelTypeId),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePlugin(plugin) => {
                write!(f, "plugin `{plugin}` is already registered")
            }
            Self::UnknownPlugin(plugin) => {
                write!(f, "plugin `{plugin}` is not registered")
            }
            Self::DuplicatePanel(panel) => {
                write!(f, "panel type `{panel}` is already registered")
            }
            Self::UnknownPanel(panel) => {
                write!(f, "panel type `{panel}` is not registered")
            }
            Self::MissingPanelFactory(panel) => {
                write!(f, "panel type `{panel}` has no factory in this build")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// The host-wide registry of plugin contributions.
///
/// Stored as a GPUI global so the workspace, the tab bar, and (later) the
/// plugin manager all see the same catalog. Registration happens on the GPUI
/// thread before the workspace is built.
#[derive(Default)]
pub struct PanelCatalog {
    plugins: BTreeMap<PluginId, PluginRegistration>,
    panels: BTreeMap<PanelTypeId, Rc<PanelDescriptor>>,
}

impl PanelCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept a plugin. Registering the same id twice is an error rather than
    /// an update: replacing contributions underneath a live layout would make
    /// persisted panel state ambiguous.
    pub fn register_plugin(&mut self, plugin: PluginRegistration) -> Result<(), CatalogError> {
        if self.plugins.contains_key(plugin.id()) {
            return Err(CatalogError::DuplicatePlugin(plugin.id().clone()));
        }
        self.plugins.insert(plugin.id().clone(), plugin);
        Ok(())
    }

    /// Remove a plugin and every panel type it contributed.
    ///
    /// Panels already placed in a layout are not destroyed here; the workspace
    /// shows them as unavailable on the next resolution and keeps their
    /// persisted state. Returns the panel types that were removed.
    pub fn unregister_plugin(
        &mut self,
        plugin: &PluginId,
    ) -> Result<Vec<PanelTypeId>, CatalogError> {
        if !self.plugins.contains_key(plugin) {
            return Err(CatalogError::UnknownPlugin(plugin.clone()));
        }
        self.plugins.remove(plugin);

        let removed: Vec<PanelTypeId> = self
            .panels
            .values()
            .filter(|descriptor| descriptor.plugin() == plugin)
            .map(|descriptor| descriptor.panel_type().clone())
            .collect();
        for panel_type in &removed {
            self.panels.remove(panel_type);
        }
        Ok(removed)
    }

    /// Add one panel type to the catalog.
    ///
    /// The owning plugin must already be registered, and the type id must not
    /// be known yet.
    pub fn register_panel(&mut self, descriptor: Rc<PanelDescriptor>) -> Result<(), CatalogError> {
        if !self.plugins.contains_key(descriptor.plugin()) {
            return Err(CatalogError::UnknownPlugin(descriptor.plugin().clone()));
        }
        let panel_type = descriptor.panel_type().clone();
        if self.panels.contains_key(&panel_type) {
            return Err(CatalogError::DuplicatePanel(panel_type));
        }
        self.panels.insert(panel_type, descriptor);
        Ok(())
    }

    /// A registered plugin.
    pub fn plugin(&self, plugin: &PluginId) -> Option<&PluginRegistration> {
        self.plugins.get(plugin)
    }

    /// Every registered plugin, ordered by id.
    pub fn plugins(&self) -> impl Iterator<Item = &PluginRegistration> {
        self.plugins.values()
    }

    /// The descriptor for a panel type.
    pub fn panel(&self, panel_type: &PanelTypeId) -> Option<&Rc<PanelDescriptor>> {
        self.panels.get(panel_type)
    }

    /// The descriptor for a persisted `panel_name`, as GPUI Base Dock writes it.
    pub fn panel_by_name(&self, panel_name: &str) -> Option<&Rc<PanelDescriptor>> {
        self.panels.get(panel_name)
    }

    /// Every panel type, ordered by id.
    pub fn panels(&self) -> impl Iterator<Item = &Rc<PanelDescriptor>> {
        self.panels.values()
    }

    /// How many panel types are registered.
    pub fn len(&self) -> usize {
        self.panels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }
}

impl Global for PanelCatalog {}

/// Install an empty catalog if the host has not done so yet.
pub fn init(cx: &mut App) {
    if cx.try_global::<PanelCatalog>().is_none() {
        cx.set_global(PanelCatalog::new());
    }
}

/// The host catalog. Panics if [`init`] has not run; the workspace asserts this
/// at startup instead of silently building an empty layout.
pub fn catalog(cx: &App) -> &PanelCatalog {
    cx.global::<PanelCatalog>()
}

/// The host catalog, if one has been installed.
pub fn try_catalog(cx: &App) -> Option<&PanelCatalog> {
    cx.try_global::<PanelCatalog>()
}

/// The host catalog, mutably.
pub fn catalog_mut(cx: &mut App) -> &mut PanelCatalog {
    cx.global_mut::<PanelCatalog>()
}

/// The host's panel factories, keyed by panel type id.
///
/// A plugin *declares* a panel by id and metadata; the code that builds it
/// lives on the application thread, because a factory produces GPUI entities
/// and cannot be moved off it (SPEC §6). This table is that side of the
/// boundary: whoever can build a plugin's panels puts them here, and
/// [`register_declaration`] pairs a declaration up with them.
#[derive(Default)]
pub struct PanelFactories {
    factories: BTreeMap<PanelTypeId, Rc<dyn PanelFactory>>,
}

impl PanelFactories {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer a factory for one panel type; a later one replaces it.
    pub fn insert(
        &mut self,
        panel_type: PanelTypeId,
        factory: Rc<dyn PanelFactory>,
    ) -> Option<Rc<dyn PanelFactory>> {
        self.factories.insert(panel_type, factory)
    }

    /// The factory for one panel type.
    pub fn get(&self, panel_type: &PanelTypeId) -> Option<&Rc<dyn PanelFactory>> {
        self.factories.get(panel_type)
    }

    /// Drop one factory, for a plugin whose code is going away.
    pub fn remove(&mut self, panel_type: &PanelTypeId) -> Option<Rc<dyn PanelFactory>> {
        self.factories.remove(panel_type)
    }

    /// How many factories the host holds.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Whether the host holds no factories at all.
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl Global for PanelFactories {}

/// Install an empty factory table if the host has not done so yet.
pub fn init_factories(cx: &mut App) {
    if cx.try_global::<PanelFactories>().is_none() {
        cx.set_global(PanelFactories::new());
    }
}

/// The host's factory table. Panics if [`init_factories`] has not run.
pub fn factories(cx: &App) -> &PanelFactories {
    cx.global::<PanelFactories>()
}

/// The host's factory table, mutably.
pub fn factories_mut(cx: &mut App) -> &mut PanelFactories {
    cx.global_mut::<PanelFactories>()
}

/// Register a plugin and one of its panels, and bridge that panel into GPUI
/// Base Dock's restore registry.
///
/// This is the whole contribution path: after it returns, the workspace can
/// place the panel by type id, and a persisted layout naming that type can be
/// restored through the same factory.
pub fn register_contribution(
    cx: &mut App,
    descriptor: PanelDescriptor,
) -> Result<PanelTypeId, CatalogError> {
    init(cx);
    let descriptor = Rc::new(descriptor);
    let panel_type = descriptor.panel_type().clone();
    catalog_mut(cx).register_panel(descriptor)?;

    let restore_type = panel_type.clone();
    let panel_name = panel_type.as_static_str();
    register_dock_panel(cx, panel_name, move |build, window, cx| {
        let state = build.state().clone();
        let (instance, panel_state, recorded_owner) =
            crate::workspace::panels::host::restore_state(&state);

        // Resolve the contribution now, not from a descriptor captured here:
        // the plugin that owned it may have been unloaded since the layout was
        // written. A withdrawn contribution restores as a placeholder that
        // keeps the saved state, exactly like a plugin that never loaded.
        let Some(restore) =
            try_catalog(cx).and_then(|catalog| catalog.panel(&restore_type).cloned())
        else {
            return crate::workspace::panels::missing::view_for_state(
                &state,
                format!("the `{panel_name}` contribution is not loaded"),
                cx,
            );
        };

        // A leaf that says it came from another plugin is not ours to restore:
        // handing that payload to this factory would interpret another plugin's
        // state with this plugin's schema.
        if let Some(owner) = recorded_owner
            && owner != restore.plugin().as_str()
        {
            return crate::workspace::panels::missing::view_for_state(
                &state,
                format!(
                    "this panel was saved by plugin `{owner}` but is now provided by `{}`",
                    restore.plugin()
                ),
                cx,
            );
        }

        match restore.create(instance, Some(panel_state), window, cx) {
            Ok(view) => view,
            Err(error) => {
                crate::workspace::panels::missing::view_for_state(&state, error.to_string(), cx)
            }
        }
    });

    Ok(panel_type)
}

/// Accept a plugin's declaration, pairing each declared panel with a factory
/// the host already holds.
///
/// The whole declaration is checked before anything is registered, so a plugin
/// whose build is missing one factory is refused entire rather than landing
/// half-contributed — a partial registration would put panels in the catalog
/// that a restored layout could name and nothing could build.
pub fn register_declaration(
    cx: &mut App,
    declaration: &PluginDeclaration,
) -> Result<(), CatalogError> {
    init(cx);
    init_factories(cx);

    let mut descriptors = Vec::with_capacity(declaration.panels().len());
    for panel in declaration.panels() {
        let panel_type = panel.panel_type();
        if catalog(cx).panel(panel_type).is_some() {
            return Err(CatalogError::DuplicatePanel(panel_type.clone()));
        }
        let Some(factory) = factories(cx).get(panel_type).cloned() else {
            return Err(CatalogError::MissingPanelFactory(panel_type.clone()));
        };
        let descriptor = PanelDescriptor::new(
            declaration.plugin().clone(),
            panel_type.name(),
            panel.title(),
            factory,
        )
        .map_err(|_| CatalogError::UnknownPanel(panel_type.clone()))?
        .with_multiplicity(panel.multiplicity());
        let descriptor = match panel.icon() {
            Some(icon) => descriptor.with_icon(icon),
            None => descriptor,
        };
        descriptors.push(descriptor);
    }

    catalog_mut(cx).register_plugin(PluginRegistration::new(
        declaration.plugin().clone(),
        declaration.display_name(),
    ))?;
    for descriptor in descriptors {
        register_contribution(cx, descriptor)?;
    }

    Ok(())
}

/// Withdraw a plugin and every panel it declared.
///
/// Live panels are not destroyed: the next layout resolution reports them as
/// unavailable and their saved state is kept, which is the same policy a plugin
/// that failed to load gets. GPUI Base Dock's own builder for the panel name is
/// left in place and is harmless, because [`register_contribution`]'s builder
/// resolves through this catalog every time it runs.
pub fn withdraw_declaration(
    cx: &mut App,
    plugin: &PluginId,
) -> Result<Vec<PanelTypeId>, CatalogError> {
    let removed = catalog_mut(cx).unregister_plugin(plugin)?;
    for panel_type in &removed {
        factories_mut(cx).remove(panel_type);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::AnyView;
    use gpui_kit::base::dock::PanelView;

    fn stub_factory() -> Rc<dyn PanelFactory> {
        Rc::new(
            |request: &PanelCreateRequest,
             _: &mut Window,
             _: &mut App|
             -> Result<Arc<dyn PanelView>, PanelCreateError> {
                Err(PanelCreateError::factory_failed(
                    request.panel_type(),
                    "stub factory",
                ))
            },
        )
    }

    fn plugin(id: &str) -> PluginId {
        PluginId::parse(id).expect("test plugin id is valid")
    }

    fn descriptor(plugin: &PluginId, name: &str) -> PanelDescriptor {
        PanelDescriptor::new(plugin.clone(), name, name, stub_factory())
            .expect("test panel descriptor is valid")
    }

    fn catalog_with(plugin_id: &str, panels: &[&str]) -> PanelCatalog {
        let mut catalog = PanelCatalog::new();
        let owner = plugin(plugin_id);
        catalog
            .register_plugin(PluginRegistration::new(owner.clone(), "Test Plugin"))
            .expect("plugin registers once");
        for name in panels {
            catalog
                .register_panel(Rc::new(descriptor(&owner, name)))
                .expect("panel registers once");
        }
        catalog
    }

    #[test]
    fn registration_is_looked_up_by_type_id_and_by_dock_panel_name() {
        let catalog = catalog_with("brucebox.git", &["status", "diff"]);
        let status = PanelTypeId::parse("brucebox.git.status").unwrap();

        assert_eq!(catalog.len(), 2);
        assert_eq!(
            catalog.panel(&status).unwrap().panel_type(),
            &status,
            "a registered panel keeps its namespaced type id"
        );
        assert_eq!(
            catalog
                .panel_by_name("brucebox.git.status")
                .unwrap()
                .title(),
            "status"
        );
        assert!(
            catalog
                .panel(&PanelTypeId::parse("brucebox.git.log").unwrap())
                .is_none()
        );
        assert_eq!(
            catalog
                .panels()
                .map(|descriptor| descriptor.panel_type().as_str())
                .collect::<Vec<_>>(),
            vec!["brucebox.git.diff", "brucebox.git.status"],
            "enumeration is stable and ordered by type id"
        );
    }

    #[test]
    fn duplicate_plugins_and_panel_types_are_rejected() {
        let mut catalog = catalog_with("brucebox.git", &["status"]);
        let owner = plugin("brucebox.git");

        assert_eq!(
            catalog.register_plugin(PluginRegistration::new(owner.clone(), "Again")),
            Err(CatalogError::DuplicatePlugin(owner.clone()))
        );
        assert_eq!(
            catalog.register_panel(Rc::new(descriptor(&owner, "status"))),
            Err(CatalogError::DuplicatePanel(
                PanelTypeId::parse("brucebox.git.status").unwrap()
            ))
        );
    }

    #[test]
    fn a_contribution_needs_its_plugin_to_be_registered_first() {
        let mut catalog = PanelCatalog::new();
        let orphan = plugin("brucebox.agent-chat");

        assert_eq!(
            catalog.register_panel(Rc::new(descriptor(&orphan, "chat"))),
            Err(CatalogError::UnknownPlugin(orphan.clone())),
            "a plugin that failed to load contributes nothing"
        );

        catalog
            .register_plugin(PluginRegistration::new(orphan.clone(), "Agent Chat"))
            .unwrap();
        assert!(
            catalog
                .register_panel(Rc::new(descriptor(&orphan, "chat")))
                .is_ok()
        );
    }

    #[test]
    fn unregistering_a_plugin_removes_only_its_panels() {
        let mut catalog = catalog_with("brucebox.git", &["status", "diff"]);
        let other = plugin("brucebox.agent-chat");
        catalog
            .register_plugin(PluginRegistration::new(other.clone(), "Agent Chat"))
            .unwrap();
        catalog
            .register_panel(Rc::new(descriptor(&other, "chat")))
            .unwrap();

        let removed = catalog
            .unregister_plugin(&plugin("brucebox.git"))
            .expect("the plugin was registered");

        assert_eq!(
            removed,
            vec![
                PanelTypeId::parse("brucebox.git.diff").unwrap(),
                PanelTypeId::parse("brucebox.git.status").unwrap(),
            ]
        );
        assert_eq!(
            catalog
                .panels()
                .map(|descriptor| descriptor.panel_type().as_str())
                .collect::<Vec<_>>(),
            vec!["brucebox.agent-chat.chat"]
        );
        assert_eq!(
            catalog.unregister_plugin(&plugin("brucebox.git")),
            Err(CatalogError::UnknownPlugin(plugin("brucebox.git")))
        );
    }

    #[test]
    fn multiplicity_defaults_to_single_and_can_be_relaxed() {
        let owner = plugin("brucebox.git");
        let single = descriptor(&owner, "status");
        let multiple = descriptor(&owner, "diff").with_multiplicity(PanelMultiplicity::Multiple);

        assert_eq!(single.multiplicity(), PanelMultiplicity::Single);
        assert_eq!(PanelMultiplicity::Single.max_instances(), Some(1));
        assert!(single.allows_additional_instance(0));
        assert!(!single.allows_additional_instance(1));
        assert!(multiple.allows_additional_instance(0));
        assert!(multiple.allows_additional_instance(7));
    }

    #[test]
    fn a_missing_panel_type_is_a_typed_error_not_a_panic() {
        let catalog = catalog_with("brucebox.git", &[]);
        let unknown = PanelTypeId::parse("brucebox.git.status").unwrap();

        assert!(catalog.panel(&unknown).is_none());
        let error = PanelCreateError::UnknownPanelType(unknown.clone());
        assert_eq!(error.panel_type(), &unknown);
        assert_eq!(
            error.to_string(),
            "no contribution is registered for panel type `brucebox.git.status`"
        );
    }

    #[test]
    fn descriptors_expose_owner_metadata_and_identity() {
        let owner = plugin("brucebox.git");
        let descriptor = descriptor(&owner, "status").with_icon("icons/git.svg");
        let identity = PanelIdentity::new(descriptor.panel_type().clone(), PanelInstanceId::next());

        assert_eq!(descriptor.plugin(), &owner);
        assert_eq!(descriptor.panel_type().plugin(), owner);
        assert_eq!(
            descriptor.icon().map(|icon| icon.as_ref()),
            Some("icons/git.svg")
        );
        assert_eq!(identity.panel_type(), descriptor.panel_type());
        assert_ne!(
            identity.instance(),
            PanelIdentity::new(descriptor.panel_type().clone(), PanelInstanceId::next()).instance(),
            "each instance gets its own id"
        );
    }

    #[test]
    fn the_panel_view_handle_typing_is_used_only_behind_a_factory() {
        // Guards the shape the workspace relies on: factories hand back a
        // `PanelView` trait object, not a concrete entity.
        fn _assert_factory_shape(_: Rc<dyn PanelFactory>) {}
        let _ = std::any::TypeId::of::<AnyView>();
    }
}
