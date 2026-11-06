//! What a plugin says it contributes, as data.
//!
//! A declaration is the thing that crosses a thread boundary: ids, titles and
//! policies, never a factory, a GPUI entity, or a runtime type. A plugin's apply
//! body runs on the composition runtime's threads and cannot touch GPUI state
//! (SPEC §6), so it hands the workspace a description of its contributions and
//! lets the application thread turn that description into a
//! [`PanelDescriptor`](super::catalog::PanelDescriptor) with a factory it
//! already holds.
//!
//! Keeping contributions declarative has a second effect: unregistering is the
//! same object arriving in reverse. A plugin that unloads withdraws its plugin
//! id, and every panel it declared goes with it.

use super::catalog::{PanelMultiplicity, PanelTypeId, PluginId};

/// One panel a plugin can build.
///
/// `panel_type` is the full namespaced type id (`brucebox.git.status`), because
/// a declaration may be built by code that does not know the plugin's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelDeclaration {
    panel_type: PanelTypeId,
    title: String,
    icon: Option<String>,
    multiplicity: PanelMultiplicity,
}

impl PanelDeclaration {
    /// Declare one panel type, single-instance unless changed.
    pub fn new(panel_type: PanelTypeId, title: impl Into<String>) -> Self {
        Self {
            panel_type,
            title: title.into(),
            icon: None,
            multiplicity: PanelMultiplicity::default(),
        }
    }

    /// Attach an icon reference for a presentation layer.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Declare how many independent instances the panel supports.
    pub fn with_multiplicity(mut self, multiplicity: PanelMultiplicity) -> Self {
        self.multiplicity = multiplicity;
        self
    }

    /// The stable panel type id.
    pub fn panel_type(&self) -> &PanelTypeId {
        &self.panel_type
    }

    /// Default tab title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Optional icon reference.
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// How many live instances this panel type supports.
    pub fn multiplicity(&self) -> PanelMultiplicity {
        self.multiplicity
    }
}

/// Everything one plugin contributes, as one object.
///
/// A plugin with no panels still has a declaration: it registers its identity
/// and may publish services or events that other plugins consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDeclaration {
    plugin: PluginId,
    display_name: String,
    panels: Vec<PanelDeclaration>,
}

impl PluginDeclaration {
    /// Start a declaration for `plugin`.
    pub fn new(plugin: PluginId, display_name: impl Into<String>) -> Self {
        Self {
            plugin,
            display_name: display_name.into(),
            panels: Vec::new(),
        }
    }

    /// Add one panel contribution.
    pub fn with_panel(mut self, panel: PanelDeclaration) -> Self {
        self.panels.push(panel);
        self
    }

    /// The plugin that owns these contributions.
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// Human-readable plugin name, for diagnostics and a plugin list.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The declared panels, in declaration order.
    pub fn panels(&self) -> &[PanelDeclaration] {
        &self.panels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> PluginId {
        PluginId::parse("brucebox.git").unwrap()
    }

    fn panel(name: &str) -> PanelDeclaration {
        PanelDeclaration::new(PanelTypeId::new(&plugin(), name).unwrap(), name)
    }

    #[test]
    fn a_declaration_carries_ids_titles_and_policies_but_no_factory() {
        let declaration = PluginDeclaration::new(plugin(), "Git")
            .with_panel(panel("status"))
            .with_panel(panel("diff").with_multiplicity(PanelMultiplicity::Multiple));

        assert_eq!(declaration.plugin().as_str(), "brucebox.git");
        assert_eq!(declaration.display_name(), "Git");
        assert_eq!(declaration.panels().len(), 2);
        assert_eq!(declaration.panels()[0].title(), "status");
        assert_eq!(
            declaration.panels()[0].multiplicity(),
            PanelMultiplicity::Single
        );
        assert_eq!(
            declaration.panels()[1].multiplicity(),
            PanelMultiplicity::Multiple
        );
        assert_eq!(
            declaration.panels()[1].panel_type().as_str(),
            "brucebox.git.diff"
        );
    }

    #[test]
    fn a_declaration_is_send_and_clone_so_it_can_cross_to_the_ui_thread() {
        fn assert_send<T: Send + 'static>() {}
        assert_send::<PluginDeclaration>();

        let declaration = PluginDeclaration::new(plugin(), "Git").with_panel(panel("status"));
        assert_eq!(declaration.clone(), declaration);
    }
}
