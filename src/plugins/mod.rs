//! Native plugin packages: what the host loads, and how it enters the workspace.
//!
//! A plugin is a folder with a manifest and a native library (SPEC §5.2). This
//! module is the host's side of that: it scans the configured roots, loads what
//! validates, turns each library's registration into a
//! [`PluginDeclaration`](crate::workspace::declaration::PluginDeclaration), and
//! records what happened so the Activity panel can say so.
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
//! # Library lifetime
//!
//! [`LoadedPlugin`] owns the library handle and never unloads it. The
//! [`PluginSet`] is kept in a GPUI global for the lifetime of the application,
//! because every function pointer, panel entity and Cordis plugin the package
//! created points into that library.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use brucebox_plugin_api::RuntimeContribution;
use brucebox_plugin_loader::{Discovery, LoadedPlugin, PluginPackage};
use gpui_kit::base::dock::PanelView;
use gpui_kit::{App, Global, Window};

use crate::runtime::PluginRuntime;
use crate::workspace::catalog::{
    self, PanelCreateError, PanelCreateRequest, PanelFactories, PanelFactory, PanelMultiplicity,
    PanelTypeId, PluginId,
};
use crate::workspace::declaration::{PanelDeclaration, PluginDeclaration};
use crate::workspace::panels::host::HostPanel;
use crate::workspace::status::{self, PluginPhase};
use crate::workspace::store;

/// Environment variable naming extra plugin roots, separated by `:` (`;` on
/// Windows). A development root usually lives here.
pub const PLUGIN_ROOTS_ENV: &str = "BRUCEBOX_PLUGIN_DIR";

/// Where the host looks for plugin folders, in order.
///
/// The user plugin directory comes last so a package installed by hand wins
/// nothing: discovery refuses a duplicate id and reports the second folder, and
/// the first root listed is the one that loads.
pub fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(configured) = std::env::var_os(PLUGIN_ROOTS_ENV) {
        roots.extend(std::env::split_paths(&configured));
    }
    roots.push(store::user_data_dir().join("plugins"));
    roots
}

/// One plugin whose library is loaded and whose panels are registered.
pub struct LoadedPackage {
    plugin: LoadedPlugin,
    declaration: PluginDeclaration,
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

/// The result of loading every configured plugin root.
#[derive(Default)]
pub struct PluginSet {
    loaded: Vec<LoadedPackage>,
    problems: Vec<LoadProblem>,
}

impl PluginSet {
    /// Scan `roots` and load every valid plugin package.
    ///
    /// One broken package never stops the others: each failure becomes a
    /// [`LoadProblem`] that the host reports, and the workspace keeps its saved
    /// layout with placeholders where the missing panels were.
    pub fn load(roots: &[PathBuf]) -> Self {
        let discovery = Discovery::scan(roots);
        let mut set = Self::default();

        for problem in discovery.problems() {
            set.problems.push(LoadProblem {
                path: problem.path().to_path_buf(),
                reason: problem.reason().to_owned(),
            });
        }

        for package in discovery.packages() {
            set.load_package(package.clone());
        }

        set
    }

    fn load_package(&mut self, package: PluginPackage) {
        let root = package.root().to_path_buf();
        match package.load() {
            Ok(plugin) => match declaration_for(&plugin) {
                Ok(declaration) => self.loaded.push(LoadedPackage {
                    plugin,
                    declaration,
                }),
                Err((panel_type, error)) => self.problems.push(LoadProblem {
                    path: root,
                    reason: format!("panel `{panel_type}` cannot be registered: {error}"),
                }),
            },
            Err(error) => self.problems.push(LoadProblem {
                path: root,
                reason: error.to_string(),
            }),
        }
    }

    /// The plugins whose libraries are loaded.
    pub fn loaded(&self) -> &[LoadedPackage] {
        &self.loaded
    }

    /// The packages that were skipped, and why.
    pub fn problems(&self) -> &[LoadProblem] {
        &self.problems
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

        for problem in &self.problems {
            status::update(cx, |status| {
                status.set_phase(
                    problem.plugin_hint(),
                    PluginPhase::Failed,
                    Some(problem.reason.clone()),
                );
                status.note(format!(
                    "{} was not loaded: {}",
                    problem.path.display(),
                    problem.reason
                ));
            });
        }
    }
}

impl Global for PluginSet {}

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

/// A plugin folder that was skipped.
#[derive(Debug, Clone)]
pub struct LoadProblem {
    path: PathBuf,
    reason: String,
}

impl LoadProblem {
    /// The folder that was skipped.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Why it was skipped, phrased for the status view.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// A stable key for the status row.
    ///
    /// The folder name stands in for the plugin id: the reason a package fails
    /// is often that its manifest cannot be read, so there is no id to use.
    pub fn plugin_hint(&self) -> String {
        self.path
            .file_name()
            .map(|name| format!("plugin: {}", name.to_string_lossy()))
            .unwrap_or_else(|| format!("plugin: {}", self.path.display()))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("brucebox-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the test root is created");
        root
    }

    #[test]
    fn a_folder_without_a_manifest_is_not_a_plugin_and_is_skipped_quietly() {
        let root = temp_root("no-manifest");
        std::fs::create_dir(root.join("just-a-folder")).unwrap();

        let set = PluginSet::load(std::slice::from_ref(&root));
        assert!(set.loaded().is_empty());
        assert!(
            set.problems().is_empty(),
            "a non-plugin folder is not a problem"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_package_whose_library_is_missing_is_reported_and_does_not_stop_the_scan() {
        let root = temp_root("missing-library");
        let broken = root.join("broken-plugin");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(
            broken.join("plugin.toml"),
            r#"
schema = 1

[plugin]
id = "com.example.broken"
name = "Broken"
version = "0.1.0"
api_version = 1
sdk_version = "0.1"

[library]
linux-x86_64 = "bin/linux-x86_64/libbroken.so"
"#,
        )
        .unwrap();

        let set = PluginSet::load(std::slice::from_ref(&root));
        assert!(set.loaded().is_empty());
        assert_eq!(set.problems().len(), 1);
        assert!(
            set.problems()[0].reason().contains("missing"),
            "the reason names the missing library: {}",
            set.problems()[0].reason()
        );
        assert!(set.problems()[0].plugin_hint().contains("broken-plugin"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_incompatible_api_version_is_refused_with_both_versions_named() {
        let root = temp_root("api-version");
        let folder = root.join("future-plugin");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("plugin.toml"),
            r#"
schema = 1

[plugin]
id = "com.example.future"
name = "Future"
version = "0.1.0"
api_version = 99
sdk_version = "0.1"

[library]
linux-x86_64 = "bin/linux-x86_64/libfuture.so"
"#,
        )
        .unwrap();

        let set = PluginSet::load(std::slice::from_ref(&root));
        let reason = &set.problems()[0].reason();
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
        for folder in ["copy-a", "copy-b"] {
            let folder = root.join(folder);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("plugin.toml"),
                r#"
schema = 1

[plugin]
id = "com.example.twice"
name = "Twice"
version = "0.1.0"
api_version = 1
sdk_version = "0.1"

[library]
linux-x86_64 = "bin/linux-x86_64/libtwice.so"
"#,
            )
            .unwrap();
        }

        let set = PluginSet::load(std::slice::from_ref(&root));
        assert!(set.loaded().is_empty(), "neither copy is loaded");

        let reasons: Vec<&str> = set.problems().iter().map(LoadProblem::reason).collect();
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("com.example.twice")
                    && reason.contains("already installed")),
            "the second folder is refused by id: {reasons:?}"
        );
        assert!(
            reasons.iter().any(|reason| reason.contains("missing")),
            "the first folder is still reported for its own missing library: {reasons:?}"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn roots_put_a_configured_development_root_before_the_user_directory() {
        // The user plugin directory is always last, so a hand-installed package
        // is never shadowed by a development root.
        let roots = roots();
        assert!(
            roots.last().is_some_and(|root| root.ends_with("plugins")),
            "the user plugin directory is the last root: {roots:?}"
        );
    }
}
