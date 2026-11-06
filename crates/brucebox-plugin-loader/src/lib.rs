//! Native plugin discovery and loading.
//!
//! This crate is the whole ABI-adjacent surface of the host: it reads plugin
//! folders, validates manifests, resolves the platform library, loads it, calls
//! the one exported entry point, and *keeps the library handle alive*. Nothing
//! outside this crate touches a `libloading` handle or resolves a symbol.
//!
//! # Lifetime rule
//!
//! A loaded library must outlive every object it created: panel factories,
//! panel entities, Cordis plugins, callbacks, service vtables and destructors.
//! [`LoadedPlugin`] therefore owns the handle and never unloads it; the host
//! keeps loaded plugins for the lifetime of the process and disposes their
//! contributions before it exits. Dropping the handle while plugin code is
//! reachable would be undefined behaviour, so there is deliberately no `unload`.
//!
//! # What is trusted, and what is checked
//!
//! A native plugin is trusted in-process code: no sandbox, no process boundary.
//! The loader checks what it *can* check before calling into the library — the
//! manifest schema, the API version the manifest declares, that the library file
//! exists where the manifest says, and that the plugin id the library reports
//! matches the manifest. The API version is checked again inside the library
//! registration (see `brucebox-plugin-api`), before anything is registered.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use brucebox_plugin_api::{
    API_VERSION, ENTRY_SYMBOL, ENTRY_SYMBOL_NAME, PanelContribution, PluginEntry,
    PluginRegistration, RegistrationError, RuntimeContribution,
};
use libloading::{Library, Symbol};
use serde::Deserialize;

/// The manifest file a plugin folder must contain.
pub const MANIFEST_NAME: &str = "plugin.toml";

/// The manifest schema this loader understands.
pub const MANIFEST_SCHEMA: u32 = 1;

/// The plugin SDK version this host was built against.
pub const SDK_VERSION: &str = "0.1";

/// The platform key a manifest uses to name this build's library.
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

/// The conventional file name for a plugin library on this platform.
///
/// Packaging uses it; loading does not require it, because the manifest names
/// the path.
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
            ("id", file.plugin.id.as_str()),
            ("name", file.plugin.name.as_str()),
            ("version", file.plugin.version.as_str()),
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

        Ok(Self {
            plugin_id: file.plugin.id,
            name: file.plugin.name,
            version: file.plugin.version,
            description: file.plugin.description,
            api_version: file.plugin.api_version,
            sdk_version: file.plugin.sdk_version,
            libraries: file.library,
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
            Self::IdNotNamespaced(id) => {
                write!(
                    formatter,
                    "plugin id `{id}` must be namespaced, for example `com.example.plugin`"
                )
            }
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
        }
    }
}

impl std::error::Error for ManifestError {}

/// One plugin folder, discovered but not yet loaded.
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

    /// The folder this package was read from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The package's manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Load this package's native library and validate what it registers.
    pub fn load(self) -> Result<LoadedPlugin, LoadError> {
        let library_path = self
            .manifest
            .resolve_library(&self.root)
            .map_err(LoadError::Manifest)?;

        // SAFETY: loading a native library runs the platform's initialisers and,
        // once the entry point is called, arbitrary plugin code. A plugin is
        // trusted in-process code (SPEC §5.4); the manifest and the API version
        // handshake are the checks that can be made before that happens.
        let library = unsafe { Library::new(&library_path) }.map_err(|error| LoadError::Open {
            path: library_path.clone(),
            detail: error.to_string(),
        })?;

        // SAFETY: the symbol type is the entry point this API version defines.
        // A library that exports a different shape under this name was built
        // against another API version, which the manifest and the registration
        // check both reject before its output is used.
        let entry: Symbol<PluginEntry> =
            unsafe { library.get(ENTRY_SYMBOL) }.map_err(|error| LoadError::MissingEntry {
                path: library_path.clone(),
                detail: error.to_string(),
            })?;

        // Calling the entry point is safe: it is a plain function pointer whose
        // contract is to return a registration valid for the library's whole
        // loaded lifetime, which `LoadedPlugin` guarantees by never unloading.
        let registration = entry();
        let Some(registration) = std::ptr::NonNull::new(registration.cast_mut()) else {
            return Err(LoadError::NullRegistration);
        };
        // SAFETY: as above — the pointed-to value lives in the library's static
        // data and the library stays loaded for the process lifetime.
        let registration: &'static PluginRegistration = unsafe { registration.as_ref() };

        brucebox_plugin_api::validate(registration).map_err(LoadError::InvalidRegistration)?;

        if registration.plugin_id != self.manifest.plugin_id() {
            return Err(LoadError::IdMismatch {
                manifest: self.manifest.plugin_id().to_owned(),
                library: registration.plugin_id,
            });
        }

        Ok(LoadedPlugin {
            library,
            registration,
            package: self,
        })
    }
}

/// What discovery found in a set of plugin roots.
#[derive(Debug, Default)]
pub struct Discovery {
    packages: Vec<PluginPackage>,
    problems: Vec<DiscoveryProblem>,
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

    /// Scan every root for plugin folders.
    ///
    /// A root that does not exist is not an error: a fresh installation has no
    /// plugin directory yet, and a plugin root may be configured before anything
    /// is installed into it. Folders without a manifest are skipped silently —
    /// they are not plugin packages, and a plugin root is not a source tree.
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
                        let id = package.manifest().plugin_id().to_owned();
                        if let Some(previous) = seen.get(&id) {
                            discovery.problems.push(DiscoveryProblem {
                                path: folder,
                                reason: format!(
                                    "plugin `{id}` is already installed at `{}`",
                                    previous.display()
                                ),
                            });
                            continue;
                        }
                        seen.insert(id, folder);
                        discovery.packages.push(package);
                    }
                    Err(error) => discovery.problems.push(DiscoveryProblem {
                        path: folder,
                        reason: error.to_string(),
                    }),
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

/// A plugin whose library is loaded and whose registration is validated.
///
/// Owns the library handle: every function pointer and object the plugin handed
/// over points into that library, so this value must live as long as anything
/// plugin-owned is reachable. There is intentionally no way to unload it.
pub struct LoadedPlugin {
    /// Never read; it exists so the library stays mapped.
    #[allow(dead_code)]
    library: Library,
    registration: &'static PluginRegistration,
    package: PluginPackage,
}

impl LoadedPlugin {
    /// The plugin's stable id.
    pub fn plugin_id(&self) -> &'static str {
        self.registration.plugin_id
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        self.registration.display_name
    }

    /// Plugin version.
    pub fn version(&self) -> &'static str {
        self.registration.version
    }

    /// The panels this plugin contributes.
    pub fn panels(&self) -> &'static [PanelContribution] {
        self.registration.panels
    }

    /// The plugin's own Cordis contribution, if it has one.
    pub fn runtime(&self) -> Option<RuntimeContribution> {
        self.registration.runtime
    }

    /// The manifest the folder was loaded from.
    pub fn manifest(&self) -> &Manifest {
        self.package.manifest()
    }

    /// The folder the plugin was loaded from.
    pub fn root(&self) -> &Path {
        self.package.root()
    }

    /// The library file that was loaded, for diagnostics.
    pub fn library_path(&self) -> PathBuf {
        self.package
            .manifest()
            .resolve_library(self.package.root())
            .unwrap_or_else(|_| self.package.root().join("<unresolved>"))
    }
}

impl fmt::Debug for LoadedPlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadedPlugin")
            .field("plugin_id", &self.plugin_id())
            .field("version", &self.version())
            .field("panels", &self.panels().len())
            .field("runtime", &self.runtime().is_some())
            .finish()
    }
}

/// Why a plugin package could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The folder's manifest is unusable.
    Manifest(ManifestError),
    /// The library could not be opened.
    Open {
        /// The library that failed.
        path: PathBuf,
        /// What the platform reported.
        detail: String,
    },
    /// The library does not export the entry point.
    MissingEntry {
        /// The library that was checked.
        path: PathBuf,
        /// What the platform reported.
        detail: String,
    },
    /// The entry point returned nothing.
    NullRegistration,
    /// The library's registration is malformed or from another API version.
    InvalidRegistration(RegistrationError),
    /// The manifest and the library disagree about the plugin id.
    IdMismatch {
        /// The id in the manifest.
        manifest: String,
        /// The id inside the library.
        library: &'static str,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => write!(formatter, "{error}"),
            Self::Open { path, detail } => write!(
                formatter,
                "cannot load `{}`: {detail}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Self::MissingEntry { path, detail } => write!(
                formatter,
                "`{}` does not export `{ENTRY_SYMBOL_NAME}`: {detail}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Self::NullRegistration => {
                formatter.write_str("the plugin entry point returned no registration")
            }
            Self::InvalidRegistration(error) => {
                write!(formatter, "plugin registration refused: {error}")
            }
            Self::IdMismatch { manifest, library } => write!(
                formatter,
                "the manifest says plugin id `{manifest}` but the library says `{library}`"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
schema = 1

[plugin]
id = "brucebox.agent-chat"
name = "Agent Chat"
version = "0.1.0"
api_version = 1
sdk_version = "0.1"

[library]
linux-x86_64 = "bin/linux-x86_64/libbrucebox_agent_chat.so"
"#;

    #[test]
    fn a_manifest_round_trips_into_a_package_description() {
        let manifest = Manifest::parse(GOOD).expect("the manifest parses");

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
    fn another_api_version_is_refused_with_the_versions_named() {
        let manifest = Manifest::parse(&GOOD.replace("api_version = 1", "api_version = 7"));

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
            Manifest::parse(&GOOD.replace("schema = 1", "schema = 2")),
            Err(ManifestError::Schema { found: 2, .. })
        ));
    }

    #[test]
    fn a_missing_platform_library_is_reported_with_what_the_package_does_provide() {
        let manifest = Manifest::parse(GOOD).unwrap();
        let api = manifest.clone();
        let other = Manifest::parse(&GOOD.replace("linux-x86_64", "some-other-platform")).unwrap();

        let _ = api;
        if platform_key() != "some-other-platform" {
            match other.resolve_library(Path::new("/nowhere")) {
                Err(ManifestError::NoLibraryForPlatform { declared, .. }) => {
                    assert_eq!(declared, vec!["some-other-platform".to_owned()]);
                }
                other => panic!("expected a platform refusal, got {other:?}"),
            }
        }
        // And the manifest that does name this platform resolves nothing when
        // the file is not there.
        match manifest.resolve_library(Path::new("/nowhere")) {
            Err(ManifestError::LibraryMissing(path)) => {
                assert!(path.ends_with("libbrucebox_agent_chat.so"));
            }
            other => panic!("expected a missing library, got {other:?}"),
        }
    }

    #[test]
    fn a_library_path_may_not_climb_out_of_the_plugin_folder() {
        let manifest = Manifest::parse(&GOOD.replace(
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
            Manifest::parse(&GOOD.replace("brucebox.agent-chat", "agent-chat")),
            Err(ManifestError::IdNotNamespaced(_))
        ));
    }

    #[test]
    fn scanning_an_absent_root_finds_nothing_and_reports_nothing() {
        let discovery = Discovery::scan(&[PathBuf::from("/definitely/not/a/plugin/root")]);
        assert!(discovery.packages().is_empty());
        assert!(discovery.problems().is_empty());
    }
}
