//! Finding plugin packages, deciding which ones may run, and saying why.
//!
//! Discovery is deliberately boring and deterministic: every configured root is
//! read in order, folders inside it are visited in name order, and a package's
//! fate is a small closed set of reasons. The result is a [`Scan`] — packages
//! that are ready to load, plus one report per folder the host decided *not* to
//! load, with the reason spelled out. Nothing here opens a library; that is
//! [`super::PluginSet`]'s job, and it only ever sees packages this module marked
//! ready.
//!
//! # Roots, and who wins
//!
//! Roots are searched in this order:
//!
//! 1. the roots named by `BRUCEBOX_PLUGIN_DIR` (`:`-separated), the *development*
//!    roots — what a developer points at while building a plugin;
//! 2. `<user data dir>/plugins`, the *user* root — where packages are installed.
//!
//! A plugin id may be installed once. If two folders declare the same id, the one
//! in the earlier root (and, within a root, the alphabetically earlier folder)
//! loads and the other is reported. That is what makes a development copy
//! shadowing an installed copy predictable instead of a race.
//!
//! # What keeps a package out
//!
//! - its manifest does not parse, or declares versions this build cannot use
//!   (`brucebox_plugin_manifest` has the checks and the messages);
//! - the user switched it off in [`PluginConfig`];
//! - another folder already claims its id;
//! - a plugin it requires is not installed, is switched off, or has an
//!   incompatible version.
//!
//! Every one of these leaves the host running and the rest of the workspace
//! intact: the saved layout restores with an "unavailable" placeholder where the
//! plugin's panels were.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use brucebox_plugin_manifest::{Discovery, Manifest, PluginPackage};

use crate::config::PluginConfig;

/// Environment variable naming development plugin roots, separated by `:`
/// (`;` on Windows).
pub const PLUGIN_ROOTS_ENV: &str = "BRUCEBOX_PLUGIN_DIR";

/// Which kind of root a package was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// A root a developer pointed the host at.
    Development,
    /// The root packages are installed into.
    User,
}

impl RootKind {
    /// A short label for a status view.
    pub fn label(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::User => "user",
        }
    }
}

/// One directory searched for plugin folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRoot {
    kind: RootKind,
    path: PathBuf,
}

impl PluginRoot {
    /// A root of `kind` at `path`.
    pub fn new(kind: RootKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
        }
    }

    /// Which kind of root this is.
    pub fn kind(&self) -> RootKind {
        self.kind
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Where the host looks for plugins, in the order it looks.
///
/// The user root is always last, so it is always the *fallback*: a package
/// installed by hand never shadows the development copy a developer is working
/// on. An installation with no `BRUCEBOX_PLUGIN_DIR` simply has one root.
pub fn roots() -> Vec<PluginRoot> {
    let mut roots = Vec::new();
    if let Some(configured) = std::env::var_os(PLUGIN_ROOTS_ENV) {
        roots.extend(
            std::env::split_paths(&configured)
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| PluginRoot::new(RootKind::Development, path)),
        );
    }
    roots.push(PluginRoot::new(RootKind::User, crate::user_plugin_dir()));
    roots
}

/// Why a package is not part of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageState {
    /// Valid, enabled, dependencies satisfied: load it.
    Ready,
    /// The manifest could not be read or declares an unusable version.
    Refused {
        /// What is wrong, phrased for a status view.
        reason: String,
    },
    /// The user switched it off.
    Disabled,
    /// A plugin it requires cannot satisfy the requirement.
    MissingDependency {
        /// The dependency as declared.
        dependency: String,
        /// What is wrong with it.
        problem: String,
    },
}

impl PackageState {
    /// Whether this package should be loaded.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// A one-line reason, or `None` when ready.
    pub fn reason(&self) -> Option<String> {
        match self {
            Self::Ready => None,
            Self::Refused { reason } => Some(reason.clone()),
            Self::Disabled => Some("disabled in the plugin config".to_owned()),
            Self::MissingDependency {
                dependency,
                problem,
            } => Some(format!("requires `{dependency}`, which {problem}")),
        }
    }
}

/// What the host knows about one folder in a plugin root.
#[derive(Debug, Clone)]
pub struct PackageReport {
    plugin_id: String,
    name: String,
    version: String,
    folder: PathBuf,
    root: PluginRoot,
    state: PackageState,
}

impl PackageReport {
    /// The plugin's id. For a folder whose manifest did not parse, this is the
    /// folder name, so a broken package still has a stable row.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Human-readable name, or the id when the manifest could not say.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The package's version, or an empty string when unknown.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The folder the package was read from.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The root it was found in.
    pub fn root(&self) -> &PluginRoot {
        &self.root
    }

    /// What the host decided about it.
    pub fn state(&self) -> &PackageState {
        &self.state
    }

    /// Record a later verdict about this package.
    ///
    /// Used by the host when loading fails after discovery cleared the package:
    /// the row stays the same and the reason replaces "ready", so the Plugins
    /// panel shows one row per folder either way.
    pub fn set_state(&mut self, state: PackageState) {
        self.state = state;
    }

    /// `id version (development root)` for a status line.
    pub fn summary(&self) -> String {
        let version = if self.version.is_empty() {
            String::new()
        } else {
            format!(" {}", self.version)
        };
        format!(
            "{}{version} ({} root)",
            self.plugin_id,
            self.root.kind().label()
        )
    }
}

/// One folder's manifest, or why it could not be used.
struct Candidate {
    folder: PathBuf,
    root: PluginRoot,
    parsed: Result<Manifest, String>,
}

impl Candidate {
    fn plugin_id(&self) -> String {
        match &self.parsed {
            Ok(manifest) => manifest.plugin_id().to_owned(),
            Err(_) => folder_name(&self.folder),
        }
    }

    fn report(&self, state: PackageState) -> PackageReport {
        match &self.parsed {
            Ok(manifest) => PackageReport {
                plugin_id: manifest.plugin_id().to_owned(),
                name: manifest.name().to_owned(),
                version: manifest.version().to_owned(),
                folder: self.folder.clone(),
                root: self.root.clone(),
                state,
            },
            Err(_) => PackageReport {
                plugin_id: folder_name(&self.folder),
                name: folder_name(&self.folder),
                version: String::new(),
                folder: self.folder.clone(),
                root: self.root.clone(),
                state,
            },
        }
    }
}

fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

/// The result of looking at every plugin root once.
pub struct Scan {
    ready: Vec<PluginPackage>,
    reports: Vec<PackageReport>,
    notes: Vec<String>,
    config: PluginConfig,
    config_path: PathBuf,
}

impl Scan {
    /// Look through `roots`, applying `config`.
    pub fn discover(roots: &[PluginRoot], config: PluginConfig, config_path: PathBuf) -> Self {
        let paths: Vec<PathBuf> = roots.iter().map(|root| root.path().to_path_buf()).collect();
        let discovery = Discovery::scan(&paths);

        let candidates = candidate_list(discovery, roots);

        // One verdict per candidate, in the same order. A folder whose manifest
        // could not be read is already decided; the rest start provisional and are
        // narrowed below.
        let mut states: Vec<PackageState> = candidates
            .iter()
            .map(|candidate| match &candidate.parsed {
                Ok(_) => PackageState::Ready,
                Err(reason) => PackageState::Refused {
                    reason: reason.clone(),
                },
            })
            .collect();

        // Discovery has already refused a second folder claiming an id (so a
        // development copy and an installed copy cannot both load); this records
        // the ids that are installed for the config check below.
        let versions: BTreeMap<String, String> = candidates
            .iter()
            .filter_map(|candidate| candidate.parsed.as_ref().ok())
            .map(|manifest| {
                (
                    manifest.plugin_id().to_owned(),
                    manifest.version().to_owned(),
                )
            })
            .collect();

        // What a dependency may resolve to: installed, readable, and switched on.
        let available: BTreeMap<&str, &str> = candidates
            .iter()
            .zip(states.iter())
            .filter(|(_, state)| **state == PackageState::Ready)
            .filter_map(|(candidate, _)| candidate.parsed.as_ref().ok())
            .map(|manifest| (manifest.plugin_id(), manifest.version()))
            .filter(|(id, _)| config.is_enabled(id))
            .collect();

        for (candidate, state) in candidates.iter().zip(states.iter_mut()) {
            if *state != PackageState::Ready {
                continue;
            }
            let Ok(manifest) = &candidate.parsed else {
                continue;
            };
            if !config.is_enabled(manifest.plugin_id()) {
                *state = PackageState::Disabled;
                continue;
            }
            if let Some((dependency, problem)) = unsatisfied_dependency(manifest, &available) {
                *state = PackageState::MissingDependency {
                    dependency,
                    problem,
                };
            }
        }

        let mut notes = Vec::new();
        for (candidate, state) in candidates.iter().zip(states.iter()) {
            if let Err(reason) = &candidate.parsed {
                notes.push(format!(
                    "{} was not loaded: {reason}",
                    candidate.folder.display()
                ));
            }
            if let Some(reason) = state.reason()
                && (*state != PackageState::Disabled || candidate.parsed.is_ok())
            {
                notes.push(format!("{}: {reason}", candidate.plugin_id()));
            }
        }
        for id in config.disabled() {
            if !versions.contains_key(id) {
                notes.push(format!(
                    "the plugin config switches off `{id}`, which is not installed"
                ));
            }
        }

        let mut ready = Vec::new();
        let mut reports = Vec::with_capacity(candidates.len());
        for (candidate, state) in candidates.into_iter().zip(states) {
            let report = candidate.report(state.clone());
            if state.is_ready()
                && let Ok(manifest) = candidate.parsed
            {
                ready.push(PluginPackage::with_manifest(
                    candidate.folder.clone(),
                    manifest,
                ));
            }
            reports.push(report);
        }

        Self {
            ready,
            reports,
            notes,
            config,
            config_path,
        }
    }

    /// Packages that are valid, enabled and satisfied: the ones to load.
    pub fn ready(&self) -> &[PluginPackage] {
        &self.ready
    }

    /// One row per folder the host looked at.
    pub fn reports(&self) -> &[PackageReport] {
        &self.reports
    }

    /// Diagnostics worth showing that are not about one package.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The config that was applied.
    pub fn config(&self) -> &PluginConfig {
        &self.config
    }

    /// The file that config came from.
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Take the scan apart: packages to load, the rows for every folder, the
    /// diagnostics, and the config that produced them.
    pub fn into_parts(
        self,
    ) -> (
        Vec<PluginPackage>,
        Vec<PackageReport>,
        Vec<String>,
        PluginConfig,
        PathBuf,
    ) {
        (
            self.ready,
            self.reports,
            self.notes,
            self.config,
            self.config_path,
        )
    }
}

/// Every folder with a manifest, in root order and name order.
fn candidate_list(discovery: Discovery, roots: &[PluginRoot]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (folder, reason) in discovery.refusals() {
        candidates.push(Candidate {
            folder: folder.clone(),
            root: root_for(folder, roots),
            parsed: Err(reason.clone()),
        });
    }
    for package in discovery.packages() {
        let root = root_for(package.root(), roots);
        candidates.push(Candidate {
            folder: package.root().to_path_buf(),
            root,
            parsed: Ok(package.manifest().clone()),
        });
    }
    candidates.sort_by(|left, right| left.folder.cmp(&right.folder));
    candidates
}

/// The root a folder was found in.
fn root_for(folder: &Path, roots: &[PluginRoot]) -> PluginRoot {
    roots
        .iter()
        .find(|root| folder.starts_with(root.path()))
        .cloned()
        .unwrap_or_else(|| PluginRoot::new(RootKind::User, folder))
}

/// The first declared dependency this package cannot use.
fn unsatisfied_dependency(
    manifest: &Manifest,
    available: &BTreeMap<&str, &str>,
) -> Option<(String, String)> {
    for dependency in manifest.dependencies() {
        match available.get(dependency.plugin_id.as_str()) {
            None => {
                return Some((
                    dependency.to_string(),
                    "is not installed (or is switched off)".to_owned(),
                ));
            }
            Some(version) if !dependency.is_satisfied_by(&dependency.plugin_id, version) => {
                return Some((
                    dependency.to_string(),
                    format!("is installed at version {version}"),
                ));
            }
            Some(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "brucebox-manager-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_package(root: &Path, folder: &str, id: &str, version: &str, requires: &[&str]) {
        let folder = root.join(folder);
        std::fs::create_dir_all(&folder).unwrap();
        let requires = if requires.is_empty() {
            String::new()
        } else {
            format!(
                "\n[dependencies]\nrequires = [{}]\n",
                requires
                    .iter()
                    .map(|entry| format!("\"{entry}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        std::fs::write(
            folder.join("plugin.toml"),
            format!(
                r#"
schema = 1

[plugin]
id = "{id}"
name = "{id}"
version = "{version}"
api_version = {api_version}
sdk_version = "0.1"

[library]
{platform} = "bin/{platform}/lib{stem}.so"
{requires}"#,
                api_version = brucebox_plugin_manifest::API_VERSION,
                platform = brucebox_plugin_manifest::platform_key(),
                stem = id.replace(['.', '-'], "_"),
            ),
        )
        .unwrap();
    }

    fn scan(roots: &[PluginRoot], config: PluginConfig) -> Scan {
        Scan::discover(roots, config, PathBuf::from("/nowhere/plugins.toml"))
    }

    #[test]
    fn a_valid_package_is_ready_and_reported_with_its_root() {
        let root = temp_root("ready");
        write_package(&root, "chat", "brucebox.agent-chat", "0.1.0", &[]);
        let roots = vec![PluginRoot::new(RootKind::User, &root)];

        let scan = scan(&roots, PluginConfig::default());

        assert_eq!(scan.ready().len(), 1);
        assert_eq!(scan.reports().len(), 1);
        let report = &scan.reports()[0];
        assert_eq!(report.plugin_id(), "brucebox.agent-chat");
        assert_eq!(report.state(), &PackageState::Ready);
        assert_eq!(report.root().kind(), RootKind::User);
        assert!(report.summary().contains("0.1.0"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_plugin_switched_off_in_the_config_is_not_loaded() {
        let root = temp_root("disabled");
        write_package(&root, "chat", "brucebox.agent-chat", "0.1.0", &[]);
        let roots = vec![PluginRoot::new(RootKind::User, &root)];
        let mut config = PluginConfig::default();
        config.set_enabled("brucebox.agent-chat", false);

        let scan = scan(&roots, config);

        assert!(scan.ready().is_empty());
        assert_eq!(scan.reports()[0].state(), &PackageState::Disabled);
        assert!(
            scan.notes()
                .iter()
                .any(|note| note.contains("disabled in the plugin config")),
            "{:?}",
            scan.notes()
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_development_root_wins_over_an_installed_copy_of_the_same_id() {
        let development = temp_root("dev-shadow");
        let user = temp_root("user-shadow");
        write_package(&development, "chat", "brucebox.agent-chat", "0.2.0", &[]);
        write_package(&user, "chat", "brucebox.agent-chat", "0.1.0", &[]);
        let roots = vec![
            PluginRoot::new(RootKind::Development, &development),
            PluginRoot::new(RootKind::User, &user),
        ];

        let scan = scan(&roots, PluginConfig::default());

        assert_eq!(scan.ready().len(), 1, "only one copy loads");
        assert_eq!(scan.ready()[0].version(), "0.2.0");
        let duplicated = scan
            .reports()
            .iter()
            .find(|report| report.state() != &PackageState::Ready)
            .expect("the shadowed copy is reported");
        assert_eq!(duplicated.root().kind(), RootKind::User);
        match duplicated.state() {
            PackageState::Refused { reason } => {
                assert!(reason.contains("already installed"), "{reason}");
                assert!(
                    reason.contains(&development.display().to_string()),
                    "{reason}"
                );
            }
            other => panic!("expected the shadowed copy to be refused, got {other:?}"),
        }
        std::fs::remove_dir_all(development).ok();
        std::fs::remove_dir_all(user).ok();
    }

    #[test]
    fn a_missing_dependency_keeps_the_package_out_and_names_the_dependency() {
        let root = temp_root("missing-dep");
        write_package(
            &root,
            "chat",
            "brucebox.agent-chat",
            "0.1.0",
            &["brucebox.git@0.1"],
        );
        let roots = vec![PluginRoot::new(RootKind::User, &root)];

        let scan = scan(&roots, PluginConfig::default());

        assert!(scan.ready().is_empty());
        match scan.reports()[0].state() {
            PackageState::MissingDependency {
                dependency,
                problem,
            } => {
                assert_eq!(dependency, "brucebox.git@0.1");
                assert!(problem.contains("not installed"), "{problem}");
            }
            other => panic!("expected a missing dependency, got {other:?}"),
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_dependency_at_an_incompatible_version_is_named_with_the_installed_one() {
        let root = temp_root("bad-dep-version");
        write_package(&root, "git", "brucebox.git", "0.2.0", &[]);
        write_package(
            &root,
            "chat",
            "brucebox.agent-chat",
            "0.1.0",
            &["brucebox.git@0.1"],
        );
        let roots = vec![PluginRoot::new(RootKind::User, &root)];

        let scan = scan(&roots, PluginConfig::default());

        assert_eq!(scan.ready().len(), 1, "only the dependency loads");
        let chat = scan
            .reports()
            .iter()
            .find(|report| report.plugin_id() == "brucebox.agent-chat")
            .unwrap();
        match chat.state() {
            PackageState::MissingDependency { problem, .. } => {
                assert!(problem.contains("0.2.0"), "{problem}");
            }
            other => panic!("expected an incompatible dependency, got {other:?}"),
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_dependency_that_is_switched_off_is_reported_as_unavailable() {
        let root = temp_root("dep-disabled");
        write_package(&root, "git", "brucebox.git", "0.1.0", &[]);
        write_package(
            &root,
            "chat",
            "brucebox.agent-chat",
            "0.1.0",
            &["brucebox.git"],
        );
        let roots = vec![PluginRoot::new(RootKind::User, &root)];
        let mut config = PluginConfig::default();
        config.set_enabled("brucebox.git", false);

        let scan = scan(&roots, config);

        let chat = scan
            .reports()
            .iter()
            .find(|report| report.plugin_id() == "brucebox.agent-chat")
            .unwrap();
        assert!(matches!(
            chat.state(),
            PackageState::MissingDependency { .. }
        ));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_config_that_switches_off_a_plugin_that_is_not_installed_says_so() {
        let root = temp_root("unknown-config");
        let roots = vec![PluginRoot::new(RootKind::User, &root)];
        let mut config = PluginConfig::default();
        config.set_enabled("brucebox.removed", false);

        let scan = scan(&roots, config);

        assert!(
            scan.notes()
                .iter()
                .any(|note| note.contains("brucebox.removed") && note.contains("not installed")),
            "{:?}",
            scan.notes()
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn roots_always_end_with_the_user_directory() {
        let roots = roots();
        assert_eq!(roots.last().map(PluginRoot::kind), Some(RootKind::User));
        assert!(
            roots
                .last()
                .is_some_and(|root| root.path().ends_with("plugins"))
        );
    }
}
