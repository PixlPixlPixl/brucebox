//! The Brucebox plugin folder format, as data.
//!
//! A plugin is a folder (SPEC §5.2):
//!
//! ```text
//! brucebox-agent-chat/
//! ├── plugin.toml                              manifest: identity + compatibility
//! └── bin/linux-x86_64/libbrucebox_agent_chat.so   one library per platform
//! ```
//!
//! This crate reads and validates that folder without loading anything: the
//! host's loader trusts what is here enough to open the library, and the
//! developer tooling (packaging, validation, install, enable/disable) shares
//! exactly the same parser so a package cannot be built that the host will read
//! differently.
//!
//! # What is checked before anything is executed
//!
//! - the manifest schema this build understands,
//! - the plugin API version and SDK version the package was built against,
//! - a namespaced plugin id, and non-empty name and version,
//! - a library for *this* platform, at a relative path inside the folder,
//! - declared plugin dependencies, in a form that can be resolved.
//!
//! # Manifest
//!
//! ```toml
//! schema = 1
//!
//! [plugin]
//! id = "brucebox.agent-chat"
//! name = "Agent Chat"
//! version = "0.1.0"
//! description = "A native conversation panel."
//! api_version = 1
//! sdk_version = "0.1"
//!
//! [library]
//! linux-x86_64 = "bin/linux-x86_64/libbrucebox_agent_chat.so"
//!
//! [dependencies]
//! requires = ["brucebox.git", "brucebox.workspace@0.1"]
//! ```
//!
//! A `requires` entry is a plugin id, optionally followed by `@` and the version
//! *prefix* the dependency must have. A dependency is resolved against the
//! packages that are installed and enabled; a missing or disabled dependency
//! keeps the depending plugin out of the workspace instead of half-starting it.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The manifest file a plugin folder must contain.
pub const MANIFEST_NAME: &str = "plugin.toml";

/// The manifest schema this build understands.
pub const MANIFEST_SCHEMA: u32 = 1;

/// The plugin SDK version this host was built against.
pub const SDK_VERSION: &str = "0.1";

/// The plugin API version this host implements.
pub const API_VERSION: u32 = brucebox_plugin_api::API_VERSION;

/// The platform key a manifest uses to name this build's library.
///
/// One key per OS and architecture, so a package can ship several libraries and
/// the host picks the one it can load.
pub fn platform_key() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else {
        "unsupported"
    }
}

/// The conventional library file name for a plugin on this platform.
///
/// Packaging uses it to name the built library; loading does not require it,
/// because the manifest names the path.
pub fn library_file_name(plugin_id: &str) -> String {
    let stem = plugin_id.replace(['.', '-'], "_");
    if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// The library path a package should use on this platform.
pub fn library_relative_path(plugin_id: &str) -> String {
    format!("bin/{}/{}", platform_key(), library_file_name(plugin_id))
}

/// One plugin another package depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDependency {
    /// The plugin id this package needs.
    pub plugin_id: String,
    /// The version prefix it needs, if any (`0.1` matches `0.1.0`).
    pub version_prefix: Option<String>,
}

impl PluginDependency {
    /// Parse one `requires` entry: `id` or `id@version-prefix`.
    pub fn parse(entry: &str) -> Result<Self, ManifestError> {
        let entry = entry.trim();
        if entry.is_empty() {
            return Err(ManifestError::Empty("dependencies.requires entry"));
        }
        let (plugin_id, version_prefix) = match entry.split_once('@') {
            Some((id, version)) => {
                let version = version.trim();
                if version.is_empty() {
                    return Err(ManifestError::DependencyVersionMissing(entry.to_owned()));
                }
                (id.trim(), Some(version.to_owned()))
            }
            None => (entry, None),
        };
        if !plugin_id.contains('.') {
            return Err(ManifestError::IdNotNamespaced(plugin_id.to_owned()));
        }
        Ok(Self {
            plugin_id: plugin_id.to_owned(),
            version_prefix,
        })
    }

    /// Whether one installed plugin satisfies this dependency.
    pub fn is_satisfied_by(&self, plugin_id: &str, version: &str) -> bool {
        if self.plugin_id != plugin_id {
            return false;
        }
        match &self.version_prefix {
            None => true,
            Some(prefix) => version.starts_with(prefix.as_str()),
        }
    }
}

impl fmt::Display for PluginDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version_prefix {
            Some(prefix) => write!(formatter, "{}@{prefix}", self.plugin_id),
            None => formatter.write_str(&self.plugin_id),
        }
    }
}

/// A validated plugin manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    plugin_id: String,
    name: String,
    version: String,
    description: Option<String>,
    api_version: u32,
    sdk_version: String,
    libraries: BTreeMap<String, String>,
    dependencies: Vec<PluginDependency>,
}

impl Manifest {
    /// The plugin's stable id.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Plugin version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Optional description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The plugin API version the package declares.
    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    /// The SDK version the package declares.
    pub fn sdk_version(&self) -> &str {
        &self.sdk_version
    }

    /// The library path for the running platform, if the package has one.
    pub fn library_for_platform(&self) -> Option<&str> {
        self.libraries.get(platform_key()).map(String::as_str)
    }

    /// Every platform this package ships a library for, in manifest order.
    pub fn platforms(&self) -> impl Iterator<Item = &str> {
        self.libraries.keys().map(String::as_str)
    }

    /// The plugins this package requires.
    pub fn dependencies(&self) -> &[PluginDependency] {
        &self.dependencies
    }

    /// Parse and validate manifest text.
    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        let file: ManifestFile =
            toml::from_str(raw).map_err(|error| ManifestError::Malformed(error.to_string()))?;

        if file.schema != MANIFEST_SCHEMA {
            return Err(ManifestError::Schema {
                found: file.schema,
                supported: MANIFEST_SCHEMA,
            });
        }
        if file.plugin.api_version != API_VERSION {
            return Err(ManifestError::ApiVersion {
                declared: file.plugin.api_version,
                supported: API_VERSION,
            });
        }
        if file.plugin.sdk_version != SDK_VERSION {
            return Err(ManifestError::SdkVersion {
                declared: file.plugin.sdk_version,
                supported: SDK_VERSION.to_owned(),
            });
        }
        for (field, value) in [
            ("plugin.id", file.plugin.id.as_str()),
            ("plugin.name", file.plugin.name.as_str()),
            ("plugin.version", file.plugin.version.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(ManifestError::Empty(field));
            }
        }
        if !file.plugin.id.contains('.') {
            return Err(ManifestError::IdNotNamespaced(file.plugin.id));
        }
        if file.library.is_empty() {
            return Err(ManifestError::NoLibraries);
        }

        let mut dependencies = Vec::new();
        for entry in file.dependencies.requires {
            let dependency = PluginDependency::parse(&entry)?;
            if dependency.plugin_id == file.plugin.id {
                return Err(ManifestError::SelfDependency(file.plugin.id));
            }
            if dependencies.contains(&dependency) {
                return Err(ManifestError::DuplicateDependency(entry));
            }
            dependencies.push(dependency);
        }

        Ok(Self {
            plugin_id: file.plugin.id,
            name: file.plugin.name,
            version: file.plugin.version,
            description: file.plugin.description,
            api_version: file.plugin.api_version,
            sdk_version: file.plugin.sdk_version,
            libraries: file.library,
            dependencies,
        })
    }

    /// Resolve this platform's library path inside `root`, checking that it
    /// stays inside the plugin folder and exists.
    pub fn resolve_library(&self, root: &Path) -> Result<PathBuf, ManifestError> {
        let declared =
            self.library_for_platform()
                .ok_or_else(|| ManifestError::NoLibraryForPlatform {
                    platform: platform_key().to_owned(),
                    declared: self.libraries.keys().cloned().collect(),
                })?;

        let relative = Path::new(declared);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| component.as_os_str() == "..")
        {
            return Err(ManifestError::LibraryEscapesFolder(declared.to_owned()));
        }

        let path = root.join(relative);
        if !path.is_file() {
            return Err(ManifestError::LibraryMissing(path));
        }
        Ok(path)
    }
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    schema: u32,
    plugin: ManifestPlugin,
    library: BTreeMap<String, String>,
    #[serde(default)]
    dependencies: ManifestDependencies,
}

#[derive(Debug, Deserialize)]
struct ManifestPlugin {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    api_version: u32,
    sdk_version: String,
}

#[derive(Debug, Default, Deserialize)]
struct ManifestDependencies {
    #[serde(default)]
    requires: Vec<String>,
}

/// Why a manifest was refused.
#[derive(Debug)]
pub enum ManifestError {
    /// The text is not a manifest.
    Malformed(String),
    /// The manifest schema is not the one this host reads.
    Schema {
        /// Declared schema.
        found: u32,
        /// Supported schema.
        supported: u32,
    },
    /// The plugin was built against another plugin API version.
    ApiVersion {
        /// Declared version.
        declared: u32,
        /// Supported version.
        supported: u32,
    },
    /// The plugin was built against another SDK version.
    SdkVersion {
        /// Declared version.
        declared: String,
        /// Supported version.
        supported: String,
    },
    /// A required field is empty.
    Empty(&'static str),
    /// The plugin id is not namespaced.
    IdNotNamespaced(String),
    /// The manifest names no library for any platform.
    NoLibraries,
    /// The manifest names no library for this platform.
    NoLibraryForPlatform {
        /// This platform's key.
        platform: String,
        /// The platforms the package does provide.
        declared: Vec<String>,
    },
    /// A library path is absolute or climbs out of the plugin folder.
    LibraryEscapesFolder(String),
    /// The named library is not in the folder.
    LibraryMissing(PathBuf),
    /// A dependency entry names no version after `@`.
    DependencyVersionMissing(String),
    /// A package requires itself.
    SelfDependency(String),
    /// The same dependency is declared twice.
    DuplicateDependency(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(detail) => write!(formatter, "cannot read the manifest: {detail}"),
            Self::Schema { found, supported } => write!(
                formatter,
                "manifest schema {found} is not supported (this build reads schema {supported})"
            ),
            Self::ApiVersion {
                declared,
                supported,
            } => write!(
                formatter,
                "plugin API version {declared} is not compatible with this build (it implements version {supported})"
            ),
            Self::SdkVersion {
                declared,
                supported,
            } => write!(
                formatter,
                "plugin SDK version {declared} is not compatible with this build (it implements version {supported})"
            ),
            Self::Empty(field) => write!(formatter, "`{field}` must not be empty"),
            Self::IdNotNamespaced(id) => write!(
                formatter,
                "plugin id `{id}` must be namespaced, for example `com.example.plugin`"
            ),
            Self::NoLibraries => formatter.write_str("the manifest declares no library"),
            Self::NoLibraryForPlatform { platform, declared } => write!(
                formatter,
                "no library for `{platform}` (the package provides {})",
                if declared.is_empty() {
                    "nothing".to_owned()
                } else {
                    declared.join(", ")
                }
            ),
            Self::LibraryEscapesFolder(path) => write!(
                formatter,
                "library path `{path}` must be relative and inside the plugin folder"
            ),
            Self::LibraryMissing(path) => {
                write!(formatter, "library `{}` is missing", path.display())
            }
            Self::DependencyVersionMissing(entry) => {
                write!(formatter, "dependency `{entry}` names no version after `@`")
            }
            Self::SelfDependency(id) => write!(formatter, "plugin `{id}` cannot require itself"),
            Self::DuplicateDependency(entry) => {
                write!(formatter, "dependency `{entry}` is declared twice")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// One plugin folder, read but not loaded.
#[derive(Debug, Clone)]
pub struct PluginPackage {
    root: PathBuf,
    manifest: Manifest,
}

impl PluginPackage {
    /// Read the manifest of the plugin folder at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ManifestError> {
        let root = root.into();
        let manifest_path = root.join(MANIFEST_NAME);
        let raw = std::fs::read_to_string(&manifest_path).map_err(|error| {
            ManifestError::Malformed(format!("cannot read {}: {error}", manifest_path.display()))
        })?;
        Ok(Self {
            root,
            manifest: Manifest::parse(&raw)?,
        })
    }

    /// Wrap an already-parsed manifest for the folder at `root`.
    pub fn with_manifest(root: impl Into<PathBuf>, manifest: Manifest) -> Self {
        Self {
            root: root.into(),
            manifest,
        }
    }

    /// The folder this package was read from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The package's manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The plugin's stable id.
    pub fn plugin_id(&self) -> &str {
        self.manifest.plugin_id()
    }

    /// The package's version.
    pub fn version(&self) -> &str {
        self.manifest.version()
    }

    /// This platform's library path, checked to exist.
    pub fn library_path(&self) -> Result<PathBuf, ManifestError> {
        self.manifest.resolve_library(&self.root)
    }
}

/// What discovery found in a set of plugin roots.
#[derive(Debug, Default)]
pub struct Discovery {
    packages: Vec<PluginPackage>,
    problems: Vec<DiscoveryProblem>,
    refusals: Vec<(PathBuf, String)>,
}

impl Discovery {
    /// The plugin folders that can be loaded.
    pub fn packages(&self) -> &[PluginPackage] {
        &self.packages
    }

    /// The plugin folders that cannot be loaded, and why.
    pub fn problems(&self) -> &[DiscoveryProblem] {
        &self.problems
    }

    /// The folders that look like plugin packages but were refused, as
    /// `(folder, reason)`.
    ///
    /// Unlike [`Self::problems`], this never includes a root that could not be
    /// read: it is exactly the packages a caller would otherwise have had.
    pub fn refusals(&self) -> &[(PathBuf, String)] {
        &self.refusals
    }

    /// Scan every root for plugin folders, in the order given.
    ///
    /// A root that does not exist is not an error: a fresh installation has no
    /// plugin directory yet, and a plugin root may be configured before anything
    /// is installed into it. Folders without a manifest are skipped silently —
    /// they are not plugin packages, and a plugin root is not a source tree.
    ///
    /// A plugin id may appear only once across all roots; the first root wins and
    /// the later copy is reported, so a development root and an installed copy
    /// cannot both half-load.
    pub fn scan(roots: &[PathBuf]) -> Self {
        let mut discovery = Self::default();
        let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();

        for root in roots {
            let entries = match std::fs::read_dir(root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    discovery.problems.push(DiscoveryProblem {
                        path: root.clone(),
                        reason: format!("cannot read the plugin root: {error}"),
                    });
                    continue;
                }
            };

            let mut folders: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.path())
                .collect();
            folders.sort();

            for folder in folders {
                if !folder.join(MANIFEST_NAME).is_file() {
                    continue;
                }
                match PluginPackage::open(&folder) {
                    Ok(package) => {
                        let id = package.plugin_id().to_owned();
                        if let Some(previous) = seen.get(&id) {
                            let reason = format!(
                                "plugin `{id}` is already installed at `{}`",
                                previous.display()
                            );
                            discovery.refusals.push((folder.clone(), reason.clone()));
                            discovery.problems.push(DiscoveryProblem {
                                path: folder,
                                reason,
                            });
                            continue;
                        }
                        seen.insert(id, folder);
                        discovery.packages.push(package);
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        discovery.refusals.push((folder.clone(), reason.clone()));
                        discovery.problems.push(DiscoveryProblem {
                            path: folder,
                            reason,
                        });
                    }
                }
            }
        }

        discovery
    }
}

/// A plugin folder that was skipped, and why.
#[derive(Debug, Clone)]
pub struct DiscoveryProblem {
    path: PathBuf,
    reason: String,
}

impl DiscoveryProblem {
    /// The folder that was skipped.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Why it was skipped, phrased for a status view.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> String {
        format!(
            r#"
schema = 1

[plugin]
id = "brucebox.agent-chat"
name = "Agent Chat"
version = "0.1.0"
api_version = {api_version}
sdk_version = "0.1"

[library]
linux-x86_64 = "bin/linux-x86_64/libbrucebox_agent_chat.so"

[dependencies]
requires = ["brucebox.git", "brucebox.workspace@0.1"]
"#,
            api_version = API_VERSION,
        )
    }

    #[test]
    fn a_manifest_round_trips_into_a_package_description() {
        let manifest = Manifest::parse(&good()).expect("the manifest parses");

        assert_eq!(manifest.plugin_id(), "brucebox.agent-chat");
        assert_eq!(manifest.name(), "Agent Chat");
        assert_eq!(manifest.api_version(), API_VERSION);
        assert_eq!(manifest.sdk_version(), SDK_VERSION);
        assert_eq!(
            manifest.library_for_platform(),
            Some("bin/linux-x86_64/libbrucebox_agent_chat.so")
        );
    }

    #[test]
    fn dependencies_are_parsed_with_and_without_a_version() {
        let manifest = Manifest::parse(&good()).unwrap();
        let dependencies = manifest.dependencies();

        assert_eq!(dependencies.len(), 2);
        assert_eq!(dependencies[0].plugin_id, "brucebox.git");
        assert_eq!(dependencies[0].version_prefix, None);
        assert_eq!(dependencies[1].version_prefix.as_deref(), Some("0.1"));
        assert_eq!(dependencies[1].to_string(), "brucebox.workspace@0.1");

        assert!(dependencies[1].is_satisfied_by("brucebox.workspace", "0.1.4"));
        assert!(!dependencies[1].is_satisfied_by("brucebox.workspace", "0.2.0"));
        assert!(!dependencies[0].is_satisfied_by("brucebox.other", "0.1.0"));
    }

    #[test]
    fn a_dependency_entry_needs_a_namespaced_id_and_a_version_after_at() {
        for entry in ["no-namespace", "brucebox.git@", " "] {
            assert!(
                PluginDependency::parse(entry).is_err(),
                "`{entry}` must be refused"
            );
        }
    }

    #[test]
    fn another_api_version_is_refused_with_the_versions_named() {
        let manifest = Manifest::parse(&good().replace("api_version = 1", "api_version = 7"));

        match manifest {
            Err(ManifestError::ApiVersion {
                declared,
                supported,
            }) => {
                assert_eq!(declared, 7);
                assert_eq!(supported, API_VERSION);
            }
            other => panic!("expected an API version refusal, got {other:?}"),
        }
    }

    #[test]
    fn another_schema_version_is_refused() {
        assert!(matches!(
            Manifest::parse(&good().replace("schema = 1", "schema = 2")),
            Err(ManifestError::Schema { found: 2, .. })
        ));
    }

    #[test]
    fn a_package_cannot_require_itself_or_the_same_plugin_twice() {
        let itself = Manifest::parse(&good().replace(
            "requires = [\"brucebox.git\", \"brucebox.workspace@0.1\"]",
            "requires = [\"brucebox.agent-chat\"]",
        ));
        assert!(matches!(itself, Err(ManifestError::SelfDependency(_))));

        let twice = Manifest::parse(&good().replace(
            "requires = [\"brucebox.git\", \"brucebox.workspace@0.1\"]",
            "requires = [\"brucebox.git\", \"brucebox.git\"]",
        ));
        assert!(matches!(twice, Err(ManifestError::DuplicateDependency(_))));
    }

    #[test]
    fn a_missing_platform_library_is_reported_with_what_the_package_does_provide() {
        let manifest = Manifest::parse(&good()).unwrap();
        let other =
            Manifest::parse(&good().replace("linux-x86_64", "some-other-platform")).unwrap();

        if platform_key() != "some-other-platform" {
            match other.resolve_library(Path::new("/nowhere")) {
                Err(ManifestError::NoLibraryForPlatform { declared, .. }) => {
                    assert_eq!(declared, vec!["some-other-platform".to_owned()]);
                }
                other => panic!("expected a platform refusal, got {other:?}"),
            }
        }
        match manifest.resolve_library(Path::new("/nowhere")) {
            Err(ManifestError::LibraryMissing(path)) => {
                assert!(path.ends_with("libbrucebox_agent_chat.so"));
            }
            other => panic!("expected a missing library, got {other:?}"),
        }
    }

    #[test]
    fn a_library_path_may_not_climb_out_of_the_plugin_folder() {
        let manifest = Manifest::parse(&good().replace(
            "bin/linux-x86_64/libbrucebox_agent_chat.so",
            "../../libbrucebox_agent_chat.so",
        ))
        .unwrap();

        assert!(matches!(
            manifest.resolve_library(Path::new("/nowhere")),
            Err(ManifestError::LibraryEscapesFolder(_))
        ));
    }

    #[test]
    fn a_plugin_id_must_be_namespaced() {
        assert!(matches!(
            Manifest::parse(&good().replace("brucebox.agent-chat", "agent-chat")),
            Err(ManifestError::IdNotNamespaced(_))
        ));
    }

    #[test]
    fn the_conventional_library_path_matches_this_platform() {
        let path = library_relative_path("brucebox.agent-chat");

        assert!(path.starts_with(&format!("bin/{}/", platform_key())));
        assert!(path.ends_with(&library_file_name("brucebox.agent-chat")));
    }

    #[test]
    fn scanning_an_absent_root_finds_nothing_and_reports_nothing() {
        let discovery = Discovery::scan(&[PathBuf::from("/definitely/not/a/plugin/root")]);
        assert!(discovery.packages().is_empty());
        assert!(discovery.problems().is_empty());
    }
}
