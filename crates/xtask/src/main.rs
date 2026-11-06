//! The Brucebox developer workflow for plugin packages.
//!
//! ```sh
//! cargo xtask plugin list                 # what is installed, and may it run?
//! cargo xtask plugin validate .           # check one plugin project or folder
//! cargo xtask plugin pack .               # build one plugin project, package it
//! cargo xtask plugin install <folder>     # copy a package into the user root
//! cargo xtask plugin disable <id>         # switch a plugin off
//! cargo xtask plugin enable <id>          # switch it back on
//! ```
//!
//! This is a development tool and nothing else: it is not part of the host
//! binary, and normal application startup never runs Cargo. `pack` builds one
//! plugin project with Cargo *in that project* — never as part of the host's
//! workspace build — and copies the manifest and the built library into
//! `target/plugin-packages/<plugin-id>/`. `install` copies such a folder into the
//! user plugin root, which is what makes it appear on the next start.
//!
//! The manifest and config parsers are the host's own
//! (`brucebox-plugin-manifest`, `brucebox-plugin-install`), so the tool cannot
//! disagree with the host about what a package says or whether it is enabled.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::ExitCode;

use brucebox_plugin_install::{
    CONFIG_FILE_NAME, PluginConfig, PluginRoot, RootKind, Scan, config_path, data_dir, roots,
    user_plugin_dir,
};
use brucebox_plugin_manifest::{
    MANIFEST_NAME, Manifest, PluginPackage, library_relative_path, platform_key,
};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<(), String> {
    match arguments {
        [] => {
            print_usage();
            Ok(())
        }
        [command, rest @ ..] if command == "plugin" => match rest {
            [action, tail @ ..] => match action.as_str() {
                "list" => list(tail),
                "validate" => validate(tail),
                "pack" => pack(tail),
                "install" => install(tail),
                "enable" => set_enabled(tail, true),
                "disable" => set_enabled(tail, false),
                other => Err(format!("unknown plugin action `{other}`")),
            },
            [] => {
                print_usage();
                Ok(())
            }
        },
        [flag] if flag == "-h" || flag == "--help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("unknown command `{}`", other.join(" "))),
    }
}

fn print_usage() {
    println!(
        "usage: cargo xtask plugin <action>\n\n\
         actions:\n  \
         list                 list the plugin packages found in every root\n  \
         validate <path>      check one plugin folder or project\n  \
         pack <project>       build one plugin project and package it\n  \
         install <package>    copy a package folder into the user plugin root\n  \
         enable <id>          switch a plugin on\n  \
         disable <id>         switch a plugin off\n\n\
         environment:\n  \
         BRUCEBOX_PLUGIN_DIR  development plugin roots, separated by `:`\n  \
         BRUCEBOX_DATA_DIR    data directory holding plugins/ and {CONFIG_FILE_NAME}"
    );
}

fn list(arguments: &[String]) -> Result<(), String> {
    let roots = match option(arguments, "--root") {
        Some(root) => vec![PluginRoot::new(RootKind::Development, root)],
        None => roots(),
    };
    let (config, diagnostic) = PluginConfig::load(&config_path());
    let scan = Scan::discover(&roots, config, config_path());

    println!("plugin roots:");
    for root in &roots {
        let marker = if root.path().is_dir() {
            ""
        } else {
            " (missing)"
        };
        println!(
            "  {:<12} {}{marker}",
            root.kind().label(),
            root.path().display()
        );
    }
    if let Some(diagnostic) = diagnostic {
        println!("\n{diagnostic}");
    }

    if scan.reports().is_empty() {
        println!("\nno plugin packages found");
        return Ok(());
    }

    println!();
    let name_width = scan
        .reports()
        .iter()
        .map(|report| report.name().len())
        .max()
        .unwrap_or(0)
        .max(4);
    for report in scan.reports() {
        let state = match report.state().reason() {
            None => "ready".to_owned(),
            Some(reason) => reason,
        };
        println!(
            "{:<name_width$}  {:<7}  {:<10}  {state}",
            report.name(),
            report.version(),
            report.root().kind().label(),
        );
        println!(
            "{:name_width$}  {:<7}  {:<10}  {}",
            "",
            "",
            "",
            report.folder().display()
        );
    }
    for note in scan.notes() {
        println!("\nnote: {note}");
    }
    Ok(())
}

fn validate(arguments: &[String]) -> Result<(), String> {
    let path = positional(arguments).ok_or("validate needs a plugin folder or project")?;
    let path = PathBuf::from(path);

    if path.join("Cargo.toml").is_file() {
        let manifest = read_project_manifest(&path)?;
        println!(
            "{} {} is a valid plugin project",
            manifest.plugin_id(),
            manifest.version()
        );
        println!(
            "  library on this platform: {}",
            library_relative_path(manifest.plugin_id())
        );
        return Ok(());
    }

    let package = PluginPackage::open(&path).map_err(|error| error.to_string())?;
    package.library_path().map_err(|error| error.to_string())?;
    println!(
        "{} {} is a valid plugin package for {}",
        package.plugin_id(),
        package.version(),
        platform_key()
    );
    println!("  folder:  {}", package.root().display());
    println!(
        "  library: {}",
        package.library_path().map_err(|e| e.to_string())?.display()
    );
    Ok(())
}

fn pack(arguments: &[String]) -> Result<(), String> {
    let project = positional(arguments).unwrap_or(".").to_owned();
    let project = PathBuf::from(project);
    let release = arguments.iter().any(|argument| argument == "--release");
    let manifest = read_project_manifest(&project)?;

    let profile = if release { "release" } else { "debug" };
    println!("building {} ({profile}) …", manifest.plugin_id());
    let status = Command::new(cargo_program())
        .arg("build")
        .arg("--manifest-path")
        .arg(project.join("Cargo.toml"))
        .args(release.then_some("--release"))
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !status.success() {
        return Err(format!("the plugin project failed to build ({status})"));
    }

    let library = built_library(&project, profile)?;
    let destination = match option(arguments, "--out") {
        Some(out) => PathBuf::from(out),
        None => default_packages_dir(),
    }
    .join(manifest.plugin_id());

    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .map_err(|error| format!("cannot replace {}: {error}", destination.display()))?;
    }
    let relative = PathBuf::from(library_relative_path(manifest.plugin_id()));
    let library_target = destination.join(&relative);
    if let Some(parent) = library_target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::copy(&library, &library_target)
        .map_err(|error| format!("cannot copy {}: {error}", library.display()))?;
    std::fs::copy(project.join(MANIFEST_NAME), destination.join(MANIFEST_NAME))
        .map_err(|error| format!("cannot copy the manifest: {error}"))?;

    println!("packaged {}", manifest.plugin_id());
    println!("  folder:  {}", destination.display());
    println!("  library: {}", relative.display());
    println!();
    println!("run the host against it with:");
    println!(
        "  BRUCEBOX_PLUGIN_DIR={} cargo run",
        destination.parent().unwrap_or(&destination).display()
    );
    Ok(())
}

fn install(arguments: &[String]) -> Result<(), String> {
    let source = positional(arguments).ok_or("install needs a package folder")?;
    let source = PathBuf::from(source);
    let package = PluginPackage::open(&source).map_err(|error| error.to_string())?;
    package.library_path().map_err(|error| error.to_string())?;

    let root = match option(arguments, "--root") {
        Some(root) => PathBuf::from(root),
        None => user_plugin_dir(),
    };
    let destination = root.join(package.plugin_id());
    if destination.exists() {
        std::fs::remove_dir_all(&destination)
            .map_err(|error| format!("cannot replace {}: {error}", destination.display()))?;
    }
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("cannot create {}: {error}", root.display()))?;
    copy_tree(&source, &destination)?;

    println!(
        "installed {} into {}",
        package.plugin_id(),
        destination.display()
    );
    println!("restart Brucebox to load it");
    Ok(())
}

fn set_enabled(arguments: &[String], enabled: bool) -> Result<(), String> {
    let plugin_id = positional(arguments).ok_or("this action needs a plugin id")?;
    let path = config_path();
    let (mut config, diagnostic) = PluginConfig::load(&path);
    if let Some(diagnostic) = diagnostic {
        return Err(diagnostic);
    }

    if !config.set_enabled(plugin_id, enabled) {
        println!(
            "{} is already {}",
            plugin_id,
            if enabled { "enabled" } else { "disabled" }
        );
        return Ok(());
    }
    config
        .save(&path)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

    println!(
        "{} is {} in {}",
        plugin_id,
        if enabled { "enabled" } else { "disabled" },
        path.display()
    );
    println!("restart Brucebox to apply it");
    Ok(())
}

/// Read a plugin project's manifest.
fn read_project_manifest(project: &Path) -> Result<Manifest, String> {
    let manifest_path = project.join(MANIFEST_NAME);
    if !manifest_path.is_file() {
        return Err(format!(
            "{} is not a plugin project (no {MANIFEST_NAME})",
            project.display()
        ));
    }
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    Manifest::parse(&raw).map_err(|error| error.to_string())
}

/// The one library a plugin project's build produced.
fn built_library(project: &Path, profile: &str) -> Result<PathBuf, String> {
    let target = project.join("target").join(profile);
    let mut candidates: Vec<PathBuf> = Vec::new();
    for extension in ["so", "dylib", "dll"] {
        let Ok(entries) = std::fs::read_dir(&target) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                candidates.push(path);
            }
        }
    }
    candidates.sort();

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(format!("no plugin library found in {}", target.display())),
        _ => Err(format!(
            "expected one plugin library in {}, found: {}",
            target.display(),
            candidates
                .iter()
                .map(|path| path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Copy a package folder, keeping the layout the loader reads.
fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    let entries = std::fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)
                .map_err(|error| format!("cannot copy {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

/// Where packaged plugins go: `<repo>/target/plugin-packages`.
fn default_packages_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(|repo| repo.join("target").join("plugin-packages"))
        .unwrap_or_else(|| data_dir().join("packages"))
}

fn cargo_program() -> &'static str {
    if cfg!(windows) { "cargo.exe" } else { "cargo" }
}

fn positional(arguments: &[String]) -> Option<&str> {
    arguments
        .iter()
        .find(|argument| !argument.starts_with("--"))
        .map(String::as_str)
}

fn option(arguments: &[String], name: &str) -> Option<String> {
    let index = arguments.iter().position(|argument| argument == name)?;
    arguments.get(index + 1).cloned()
}
