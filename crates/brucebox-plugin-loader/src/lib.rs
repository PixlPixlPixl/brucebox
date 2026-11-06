//! Loading a native plugin package.
//!
//! This is the only crate that opens a plugin library or resolves a symbol from
//! it. Everything it knows about a package came from
//! [`brucebox_plugin_manifest`], and everything it hands back is
//! `brucebox_plugin_api` data — but the code behind that data is the plugin's.
//!
//! # What loading promises
//!
//! - The manifest was validated before this point; a package that reached here
//!   declared the API and SDK versions this build implements.
//! - The plugin's registration is validated before it is used: [`register`] runs
//!   every check in `brucebox_plugin_api::Registration::validate`.
//! - The library is never unloaded while the process runs. Rust has no stable ABI,
//!   and a dropped plugin leaves function pointers and trait objects in caches the
//!   host does not own; `LoadedPlugin` therefore has no `unload` and is documented
//!   as process-lifetime. Restarting the *plugin* is a Cordis restart, not a
//!   library unload.
//!
//! See `docs/adr/0005-native-plugin-boundary.md` for why this boundary is shaped
//! this way, including the measured `TypeId` constraint that keeps plugin event
//! listeners from firing.

use std::fmt;
use std::path::{Path, PathBuf};

use brucebox_plugin_api::{
    HostContext, PluginEntry, PluginRegistration, RegistrationError, validate,
};
use brucebox_plugin_manifest::{Manifest, PluginPackage};
use libloading::Library;

/// A plugin library that is loaded and validated.
///
/// Dropping this drops the library handle. Handles are kept for the lifetime of
/// the process (see the module documentation), because code from it may still be
/// reachable elsewhere.
pub struct LoadedPlugin {
    package: PluginPackage,
    registration: &'static PluginRegistration,
    /// Kept alive for the process, and after `registration` so the registration
    /// outlives nothing that could call it.
    _library: Library,
}

impl LoadedPlugin {
    /// The package this plugin came from.
    pub fn package(&self) -> &PluginPackage {
        &self.package
    }

    /// The package's manifest.
    pub fn manifest(&self) -> &Manifest {
        self.package.manifest()
    }

    /// The plugin's stable id.
    ///
    /// This is the id the *library* declares, which loading has already checked
    /// against the manifest.
    pub fn plugin_id(&self) -> &'static str {
        self.registration.plugin_id
    }

    /// The validated registration: what this plugin contributes.
    pub fn registration(&self) -> &'static PluginRegistration {
        self.registration
    }

    /// The plugin's human-readable name.
    pub fn display_name(&self) -> &'static str {
        self.registration.display_name
    }

    /// The plugin's own version, as it declares it.
    pub fn version(&self) -> &'static str {
        self.registration.version
    }

    /// The panels this plugin contributes.
    pub fn panels(&self) -> &'static [brucebox_plugin_api::PanelContribution] {
        self.registration.panels
    }

    /// The plugin's own Cordis contribution, if it has one.
    pub fn runtime(&self) -> Option<brucebox_plugin_api::RuntimeContribution> {
        self.registration.runtime
    }
}

impl fmt::Debug for LoadedPlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadedPlugin")
            .field("plugin_id", &self.plugin_id())
            .field("root", &self.package.root())
            .field("registration", &self.registration)
            .finish()
    }
}

/// Load the library of `package` and validate its registration.
///
/// Fails if the library cannot be opened, if it does not export the entry
/// symbol, if the entry returns nothing, or if the registration it describes is
/// not one this host can render (see [`RegistrationError`]).
pub fn load(package: PluginPackage, context: &HostContext) -> Result<LoadedPlugin, LoadError> {
    let library_path = package
        .library_path()
        .map_err(|error| LoadError::Package(error.to_string()))?;
    let library = open(&library_path).map_err(|error| LoadError::Open {
        path: library_path.clone(),
        detail: error,
    })?;

    // SAFETY: `entry` is resolved from a library that stays loaded for the rest of
    // the process (it is moved into `LoadedPlugin` below, which never unloads), and
    // the symbol is only used with the signature the plugin API defines for it.
    let entry: PluginEntry = unsafe {
        let symbol = library
            .get::<PluginEntry>(brucebox_plugin_api::ENTRY_SYMBOL)
            .map_err(|error| LoadError::MissingEntry {
                path: library_path.clone(),
                detail: error.to_string(),
            })?;
        *symbol
    };

    // SAFETY: the plugin API documents the entry contract — it is called once,
    // with a context the host owns for the rest of the process, and returns either
    // null or a pointer to a registration that outlives the plugin — and the
    // pointer is only dereferenced once, into a shared reference that `validate`
    // inspects before the host uses any of it.
    let registration = unsafe {
        let pointer = entry(context);
        if pointer.is_null() {
            return Err(LoadError::NoRegistration { path: library_path });
        }
        &*pointer
    };

    validate(registration).map_err(|error| LoadError::Invalid {
        path: library_path.clone(),
        error,
    })?;

    // A package may not claim one id in its manifest and another from its code:
    // the workspace, the config and the saved layout all key on one of them.
    if registration.plugin_id != package.plugin_id() {
        return Err(LoadError::IdMismatch {
            path: library_path,
            manifest: package.plugin_id().to_owned(),
            registration: registration.plugin_id,
        });
    }

    Ok(LoadedPlugin {
        package,
        registration,
        _library: library,
    })
}

/// Open a native library without unloading it on error.
///
/// `libloading` places no portable requirements on the caller beyond the library
/// staying loaded while its symbols are used; that is the whole contract this
/// boundary relies on.
fn open(path: &Path) -> Result<Library, String> {
    // SAFETY: opening a library runs the platform loader and the library's
    // initialisers. A plugin is trusted in-process code (SPEC §5.6): it can do
    // anything the host can do, and this is where it starts doing it. Nothing here
    // unsafely assumes about the library's contents — every symbol is looked up by
    // name and checked before use.
    unsafe { Library::new(path) }.map_err(|error| error.to_string())
}

/// Load every package in `packages`, collecting successes and failures.
///
/// A package that fails to load is reported and skipped; it never prevents the
/// others from loading, and the host keeps running.
pub fn load_all(
    packages: &[PluginPackage],
    context: &HostContext,
) -> (Vec<LoadedPlugin>, Vec<LoadError>) {
    let mut loaded = Vec::new();
    let mut failures = Vec::new();
    for package in packages {
        match load(package.clone(), context) {
            Ok(plugin) => loaded.push(plugin),
            Err(error) => failures.push(error),
        }
    }
    (loaded, failures)
}

/// Why a plugin could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The package itself could not be read (a library is missing, for example).
    Package(String),
    /// The library could not be opened by the platform loader.
    Open {
        /// The attempted library.
        path: PathBuf,
        /// The platform loader's message.
        detail: String,
    },
    /// The library did not export the plugin entry symbol.
    MissingEntry {
        /// The attempted library.
        path: PathBuf,
        /// The lookup error.
        detail: String,
    },
    /// The entry symbol returned nothing.
    NoRegistration {
        /// The attempted library.
        path: PathBuf,
    },
    /// The library calls itself something the manifest does not.
    IdMismatch {
        /// The attempted library.
        path: PathBuf,
        /// The id the manifest declares.
        manifest: String,
        /// The id the library declares.
        registration: &'static str,
    },
    /// The registration is not one this host can use.
    Invalid {
        /// The attempted library.
        path: PathBuf,
        /// Which check failed.
        error: RegistrationError,
    },
}

impl LoadError {
    /// A one-line description for a status view.
    pub fn reason(&self) -> String {
        match self {
            Self::Package(detail) => detail.clone(),
            Self::Open { path, detail } => {
                format!("cannot open {}: {detail}", path.display())
            }
            Self::MissingEntry { path, detail } => format!(
                "{} does not export the plugin entry point ({detail})",
                path.display()
            ),
            Self::NoRegistration { path } => format!(
                "{} returned no registration from its entry point",
                path.display()
            ),
            Self::Invalid { error, .. } => error.to_string(),
            Self::IdMismatch {
                manifest,
                registration,
                ..
            } => format!(
                "the manifest declares plugin `{manifest}` but the library calls itself `{registration}`"
            ),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason())
    }
}

impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests {
    use super::*;
    use brucebox_plugin_manifest::Manifest;

    fn package_with(raw: &str, library: Option<&[u8]>) -> PluginPackage {
        let root = std::env::temp_dir().join(format!(
            "brucebox-loader-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        if let Some(contents) = library {
            std::fs::write(root.join("broken.so"), contents).unwrap();
        }
        let manifest = Manifest::parse(raw).expect("the manifest parses");
        PluginPackage::with_manifest(root, manifest)
    }

    /// A manifest naming `library` as this platform's library.
    fn manifest_raw(library: &str) -> String {
        format!(
            r#"
schema = 1

[plugin]
id = "brucebox.test"
name = "Test"
version = "0.1.0"
api_version = {api_version}
sdk_version = "0.1"

[library]
{platform} = "{library}"
"#,
            api_version = brucebox_plugin_api::API_VERSION,
            platform = brucebox_plugin_manifest::platform_key(),
        )
    }

    #[test]
    fn a_missing_library_is_a_package_error_before_anything_is_opened() {
        let manifest = Manifest::parse(&manifest_raw("brucebox_absent.so")).unwrap();
        let package = PluginPackage::with_manifest("/definitely/not/here", manifest);

        match load(package, &HostContext::inert()) {
            Err(LoadError::Package(detail)) => assert!(detail.contains("missing")),
            other => panic!("expected a package error, got {other:?}"),
        }
    }

    #[test]
    fn a_file_that_is_not_a_library_is_reported_as_an_open_failure() {
        let package = package_with(&manifest_raw("broken.so"), Some(b"not a library"));

        match load(package, &HostContext::inert()) {
            Err(LoadError::Open { path, .. }) => assert!(path.ends_with("broken.so")),
            other => panic!("expected an open failure, got {other:?}"),
        }
    }

    /// A real, loadable shared object that is *not* a plugin: the host's own
    /// libraries are the honest test for a binary that lacks the entry symbol.
    /// Skipped when the machine has none of the candidates, rather than pretending
    /// the case passed.
    fn foreign_library() -> Option<PathBuf> {
        [
            "/lib/x86_64-linux-gnu/libc.so.6",
            "/usr/lib/x86_64-linux-gnu/libc.so.6",
            "/usr/lib/libc.so.6",
            "/usr/lib/libSystem.B.dylib",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
    }

    #[test]
    fn a_library_without_the_entry_symbol_is_refused() {
        let Some(foreign) = foreign_library() else {
            eprintln!("no candidate shared library on this machine; skipping");
            return;
        };
        let name = foreign.file_name().unwrap().to_str().unwrap();
        let package = PluginPackage::with_manifest(
            foreign.parent().unwrap_or(Path::new("/")),
            Manifest::parse(&manifest_raw(name)).unwrap(),
        );

        match load(package, &HostContext::inert()) {
            Err(LoadError::MissingEntry { path, .. }) => assert!(path.ends_with(name)),
            other => panic!("expected a missing entry symbol, got {other:?}"),
        }
    }

    #[test]
    fn load_all_keeps_going_after_a_failure() {
        let bad = package_with(&manifest_raw("broken.so"), Some(b"not a library"));

        let (loaded, failures) = load_all(std::slice::from_ref(&bad), &HostContext::inert());

        assert!(loaded.is_empty());
        assert_eq!(failures.len(), 1);
        assert!(!failures[0].reason().is_empty());
    }
}
