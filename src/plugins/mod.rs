//! Native plugin packages: what the host loads, and how it enters the workspace.
//!
//! A plugin is a folder with a manifest and a native library (SPEC §5.2). Three
//! modules split the work:
//!
//! - [`config`] — which plugins the user switched off, in a small file;
//! - [`manager`] — the roots that are searched, and which packages may run;
//! - this module — loading the libraries that were cleared, and turning their
//!   registrations into workspace declarations.
//!
//! # Why the loader registers panels itself
//!
//! Everything else in the host learns about panels from the composition runtime:
//! a plugin's apply body runs off the application thread and *declares* what it
//! contributes (ADR 0004). The loader runs on the application thread at startup,
//! before the runtime has anything to admit, and it already holds the panel
//! factories the declaration has to be paired with. So it pairs them directly —
//! the same `catalog::register_declaration` call, from the one caller that has
//! an application context at load time.
//!
//! The plugin's *own* code still joins the composition runtime, through
//! [`PluginSet::runtime_contributions`]: that is where its services, events and
//! tasks live, and it is admitted by the runtime like any other plugin.
//!
//! # Library lifetime, and what enable/disable means
//!
//! [`LoadedPlugin`] owns the library handle and never unloads it. The
//! [`PluginSet`] is kept in a GPUI global for the lifetime of the application,
//! because every function pointer, panel entity and Cordis plugin the package
//! created points into that library.
//!
//! That is why switching a plugin off writes the config and asks for a restart
//! instead of unloading the library: unloading code that the workspace may still
//! be holding a pointer into is exactly the kind of promise this boundary does not
//! make. The next start honours the config before any library is opened.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use brucebox_plugin_api::RuntimeContribution;
use brucebox_plugin_loader::{LoadedPlugin, load};
use gpui_kit::base::dock::PanelView;
use gpui_kit::{App, Window};

use crate::runtime::PluginRuntime;
use crate::runtime::bus::PluginBusLink;
use crate::workspace::catalog::{
    self, PanelCreateError, PanelCreateRequest, PanelFactories, PanelFactory, PanelMultiplicity,
    PanelTypeId, PluginId,
};
use crate::workspace::declaration::{PanelDeclaration, PluginDeclaration};
use crate::workspace::panels::host::HostPanel;
use crate::workspace::status::{self, PluginPhase};

pub use brucebox_plugin_install::{
    CONFIG_FILE_NAME, PluginConfig, PluginRoot, RootKind, Scan, config_path, data_dir,
    user_plugin_dir,
};
pub use brucebox_plugin_install::{PackageReport, PackageState, roots};

/// One plugin whose library is loaded and whose panels are registered.
pub struct LoadedPackage {
    plugin: LoadedPlugin,
    declaration: PluginDeclaration,
    /// The plugin's link to the contract bus. Kept alive for the process: the
    /// context the library was loaded with points at it, so this field is what
    /// makes every later bus call valid.
    _bus: std::sync::Arc<PluginBusLink>,
}

impl LoadedPackage {
    /// The loader's handle: keeps the library loaded.
    pub fn plugin(&self) -> &LoadedPlugin {
        &self.plugin
    }

    /// What this plugin contributes, as the workspace sees it.
    pub fn declaration(&self) -> &PluginDeclaration {
        &self.declaration
    }

    /// The plugin's own Cordis contribution, if it has one.
    pub fn runtime(&self) -> Option<RuntimeContribution> {
        self.plugin.runtime()
    }
}

/// Every plugin the host looked at, and which of them it loaded.
#[derive(Default)]
pub struct PluginSet {
    loaded: Vec<LoadedPackage>,
    reports: Vec<PackageReport>,
    notes: Vec<String>,
    config: PluginConfig,
    config_path: PathBuf,
}

impl PluginSet {
    /// Load every package the scan cleared.
    ///
    /// One broken package never stops the others: a library that cannot be opened
    /// or a registration that cannot be used turns that package's report into the
    /// reason, and the workspace keeps its saved layout with placeholders where
    /// the missing panels were.
    pub fn activate(scan: Scan) -> Self {
        let (packages, mut reports, notes, config, config_path) = scan.into_parts();
        let mut loaded = Vec::new();

        for package in packages {
            let folder = package.root().to_path_buf();
            let bus = PluginBusLink::new(package.plugin_id());
            let context = bus.host_context();
            match load(package, &context) {
                Ok(plugin) => match declaration_for(&plugin) {
                    Ok(declaration) => loaded.push(LoadedPackage {
                        plugin,
                        declaration,
                        _bus: bus,
                    }),
                    Err((panel_type, error)) => refuse(
                        &mut reports,
                        &folder,
                        format!("panel `{panel_type}` cannot be registered: {error}"),
                    ),
                },
                Err(error) => refuse(&mut reports, &folder, error.reason()),
            }
        }

        Self {
            loaded,
            reports,
            notes,
            config,
            config_path,
        }
    }

    /// The plugins whose libraries are loaded.
    pub fn loaded(&self) -> &[LoadedPackage] {
        &self.loaded
    }

    /// One row per folder the host looked at, including the ones it skipped.
    pub fn reports(&self) -> &[PackageReport] {
        &self.reports
    }

    /// Diagnostics that are not about one package.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The config that produced this set.
    pub fn config(&self) -> &PluginConfig {
        &self.config
    }

    /// The file the config was read from.
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Adopt a config the user just wrote.
    ///
    /// The loaded packages do not change — switching a plugin off takes effect at
    /// the next start (ADR 0005: a library is never unloaded) — but the switch in
    /// the Plugins panel has to show what the file now says, not what startup read.
    pub fn apply_config(&mut self, config: PluginConfig) {
        self.config = config;
    }

    /// The plugins that contribute their own code to the composition runtime.
    pub fn runtime_contributions(&self) -> Vec<PluginRuntime> {
        self.loaded
            .iter()
            .filter_map(|package| {
                package.runtime().map(|contribution| PluginRuntime {
                    plugin_id: package.plugin.plugin_id(),
                    contribution,
                })
            })
            .collect()
    }

    /// Offer every plugin's panel factories to the application thread.
    ///
    /// A factory is what the workspace calls when it needs one live panel. It
    /// runs on the application thread and calls into the plugin's library, so a
    /// plugin panel is an ordinary GPUI entity — the host only wraps it to keep
    /// the catalog's identity and the persisted envelope.
    pub fn install_factories(&self, cx: &mut App) {
        catalog::init_factories(cx);
        for package in &self.loaded {
            let plugin_id = package.plugin.plugin_id();
            for contribution in package.plugin.panels() {
                let Ok(panel_type) = PanelTypeId::new(
                    &PluginId::parse(plugin_id).expect("a validated plugin id parses"),
                    contribution.panel_segment,
                ) else {
                    continue;
                };
                let create = contribution.create;
                let factory: Rc<dyn PanelFactory> = Rc::new(
                    move |request: &PanelCreateRequest,
                          window: &mut Window,
                          cx: &mut App|
                          -> Result<std::sync::Arc<dyn PanelView>, PanelCreateError> {
                        // The plugin gets its own payload back as text: the host
                        // neither parses nor validates a plugin's panel state,
                        // which is what keeps one plugin's schema out of another
                        // plugin's restore path.
                        let restored = restored_payload(request);
                        let open = brucebox_plugin_api::PanelOpen {
                            instance: request.instance().get(),
                            restored: restored.as_deref(),
                        };
                        let parts = create(&open, window, cx);
                        Ok(HostPanel::plugin(request.identity().clone(), parts, cx))
                    },
                );
                catalog::factories_mut(cx).insert(panel_type, factory);
            }
        }
    }

    /// Register every plugin's panels with the catalog and record its phase.
    ///
    /// Must run after [`Self::install_factories`] and before the first window
    /// resolves a layout.
    pub fn register(&self, cx: &mut App) {
        for package in &self.loaded {
            let plugin_id = package.plugin.plugin_id();
            match catalog::register_declaration(cx, package.declaration()) {
                Ok(()) => {
                    status::update(cx, |status| {
                        status.set_phase(plugin_id, PluginPhase::Active, None);
                        status.note(format!(
                            "{plugin_id} {} contributed {} panel type(s)",
                            package.plugin.version(),
                            package.declaration().panels().len()
                        ));
                    });
                }
                Err(error) => {
                    status::update(cx, |status| {
                        status.set_phase(plugin_id, PluginPhase::Failed, Some(error.to_string()));
                    });
                }
            }
        }

        for report in &self.reports {
            if report.state().is_ready() {
                continue;
            }
            let reason = report.state().reason().unwrap_or_default();
            let phase = match report.state() {
                PackageState::Disabled => PluginPhase::Disabled,
                _ => PluginPhase::Failed,
            };
            status::update(cx, |status| {
                status.set_phase(report.plugin_id(), phase, Some(reason.clone()));
            });
        }

        for note in &self.notes {
            status::update(cx, |status| status.note(note.clone()));
        }
    }
}

impl gpui_kit::Global for PluginSet {}

/// Replace one package's report state after loading it failed.
fn refuse(reports: &mut [PackageReport], folder: &Path, reason: String) {
    if let Some(report) = reports.iter_mut().find(|report| report.folder() == folder) {
        report.set_state(PackageState::Refused { reason });
    }
}

/// Keep the loaded packages, and with them their libraries, alive.
pub fn install(cx: &mut App, set: PluginSet) {
    set.install_factories(cx);
    set.register(cx);
    cx.set_global(set);
}

/// The loaded packages, if [`install`] has run.
pub fn loaded(cx: &App) -> Option<&PluginSet> {
    cx.try_global::<PluginSet>()
}

/// Switch one plugin on or off for the *next* start.
///
/// The config file is re-read first, so a change made by the developer tooling
/// while the host is running is not overwritten by a stale copy. Returns whether
/// the value changed.
pub fn set_enabled(cx: &mut App, plugin_id: &str, enabled: bool) -> Result<bool, String> {
    let path = config_path();
    let (mut config, diagnostic) = PluginConfig::load(&path);
    if let Some(diagnostic) = diagnostic {
        status::update(cx, |status| status.note(diagnostic));
    }

    let changed = config.set_enabled(plugin_id, enabled);
    if !changed {
        return Ok(false);
    }
    config
        .save(&path)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

    if cx.try_global::<PluginSet>().is_some() {
        cx.global_mut::<PluginSet>().apply_config(config);
    }

    let state = if enabled {
        PluginPhase::Pending
    } else {
        PluginPhase::Disabled
    };
    status::update(cx, |status| {
        status.set_phase(
            plugin_id,
            state,
            Some(if enabled {
                "switched on; restart Brucebox to load it".to_owned()
            } else {
                "switched off; restart Brucebox to unload it".to_owned()
            }),
        );
    });
    Ok(true)
}

/// Turn a loaded library's registration into the host's declaration type.
fn declaration_for(plugin: &LoadedPlugin) -> Result<PluginDeclaration, (String, String)> {
    let plugin_id = PluginId::parse(plugin.plugin_id())
        .map_err(|error| (plugin.plugin_id().to_owned(), error.to_string()))?;

    let mut declaration = PluginDeclaration::new(plugin_id.clone(), plugin.display_name());
    for contribution in plugin.panels() {
        let panel_type = PanelTypeId::new(&plugin_id, contribution.panel_segment)
            .map_err(|error| (contribution.panel_segment.to_owned(), error.to_string()))?;
        let declared = PanelDeclaration::new(panel_type, contribution.title).with_multiplicity(
            match contribution.multiplicity {
                brucebox_plugin_api::PanelMultiplicity::Single => PanelMultiplicity::Single,
                brucebox_plugin_api::PanelMultiplicity::Multiple => PanelMultiplicity::Multiple,
            },
        );
        declaration = declaration.with_panel(declared);
    }
    Ok(declaration)
}

/// The host's factory table, for a caller that wants to check what is installed.
pub fn factories(cx: &App) -> &PanelFactories {
    catalog::factories(cx)
}

/// The plugin's own saved payload, as the JSON text it wrote.
fn restored_payload(request: &PanelCreateRequest) -> Option<String> {
    let state = request.restored_state()?;
    match &state.info {
        gpui_kit::base::dock::PanelInfo::Panel(value) if !value.is_null() => {
            serde_json::to_string(value).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "brucebox-plugins-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Write a package whose declared library does not exist.
    ///
    /// The version is a parameter so a test can ask for a *future* one; the
    /// default is this build's, which is what a package that should merely fail
    /// to load needs.
    fn write_broken(root: &Path, folder: &str, id: &str, api_version: u32) {
        let folder = root.join(folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("plugin.toml"),
            format!(
                r#"
schema = 1

[plugin]
id = "{id}"
name = "{id}"
version = "0.1.0"
api_version = {api_version}
sdk_version = "0.1"

[library]
{platform} = "bin/{platform}/libmissing.so"
"#,
                platform = brucebox_plugin_manifest::platform_key(),
            ),
        )
        .unwrap();
    }

    fn activate(root: &Path) -> PluginSet {
        let roots = vec![PluginRoot::new(RootKind::User, root)];
        let scan = Scan::discover(&roots, PluginConfig::default(), root.join("plugins.toml"));
        PluginSet::activate(scan)
    }

    #[test]
    fn a_folder_without_a_manifest_is_not_a_plugin_and_is_skipped_quietly() {
        let root = temp_root("no-manifest");
        std::fs::create_dir(root.join("just-a-folder")).unwrap();

        let set = activate(&root);

        assert!(set.loaded().is_empty());
        assert!(set.reports().is_empty(), "a non-plugin folder is not a row");
        assert!(set.notes().is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_package_whose_library_is_missing_is_reported_and_does_not_stop_the_scan() {
        let root = temp_root("missing-library");
        write_broken(
            &root,
            "broken-plugin",
            "com.example.broken",
            brucebox_plugin_api::API_VERSION,
        );

        let set = activate(&root);

        assert!(set.loaded().is_empty());
        assert_eq!(set.reports().len(), 1);
        let reason = set.reports()[0].state().reason().unwrap();
        assert!(reason.contains("missing"), "{reason}");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_incompatible_api_version_is_refused_with_both_versions_named() {
        let root = temp_root("api-version");
        write_broken(&root, "future-plugin", "com.example.future", 99);

        let set = activate(&root);
        let reason = set.reports()[0].state().reason().unwrap();

        assert!(reason.contains("99"), "declared version is named: {reason}");
        assert!(
            reason.contains(&brucebox_plugin_api::API_VERSION.to_string()),
            "supported version is named: {reason}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn two_folders_declaring_one_id_are_refused_rather_than_both_loaded() {
        let root = temp_root("duplicate-id");
        write_broken(
            &root,
            "copy-a",
            "com.example.twice",
            brucebox_plugin_api::API_VERSION,
        );
        write_broken(
            &root,
            "copy-b",
            "com.example.twice",
            brucebox_plugin_api::API_VERSION,
        );

        let set = activate(&root);

        assert!(set.loaded().is_empty(), "neither copy is loaded");
        assert_eq!(set.reports().len(), 2);
        let reasons: Vec<String> = set
            .reports()
            .iter()
            .filter_map(|report| report.state().reason())
            .collect();
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("already installed")),
            "the second folder is refused by id: {reasons:?}"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
