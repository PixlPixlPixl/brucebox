# Brucebox

A native GPUI workspace host for independently built, runtime-loaded plugins.

The host provides the application shell, the docked/floating workspace engine,
a panel catalog, and a versioned plugin boundary. Agent Chat and Git are real
first-party plugins, built and packaged as separate projects and loaded at
runtime — never compiled into this binary.

## Status

Foundation in progress. See [`PLAN.md`](PLAN.md) for the execution sequence and
[`SPEC.md`](SPEC.md) for the normative product and architecture contract.

Completed so far:

- Step 0 — bootstrap verified on the pinned toolchain, decisions recorded in
  [`docs/adr/0001-foundation-baseline-and-panel-catalog.md`](docs/adr/0001-foundation-baseline-and-panel-catalog.md).
- Step 1 — namespaced plugin/panel-type/panel-instance ids, a panel catalog with
  descriptors and factories, layout planning, missing-panel placeholders, and a
  workspace that builds its dock area from contribution ids.
- Step 2 — the layout is the user's: a versioned workspace document is restored on
  launch, saved with debounced writes, and reset explicitly with `ctrl-shift-r`.
  Panel instance identity and plugin state persist even when the owning plugin is
  missing. Decisions in
  [`docs/adr/0002-workspace-persistence.md`](docs/adr/0002-workspace-persistence.md).
- Step 3 — floating surfaces: a panel can be lifted out of the docked workspace
  into a movable, resizable group with its own dock area, its own stacking
  position, and its geometry saved with the workspace. Decisions in
  [`docs/adr/0003-floating-surfaces.md`](docs/adr/0003-floating-surfaces.md).
- Step 4 — the composition runtime: one host-owned Cordis runtime, and a bridge
  that carries plugin contributions, typed events and lifecycle state to the
  interface as messages. Decisions in
  [`docs/adr/0004-cordis-composition-runtime.md`](docs/adr/0004-cordis-composition-runtime.md).
- Step 6 — plugin folders and the developer workflow: a manifest schema with
  plugin dependencies, ordered development and user roots, an on/off config, a
  loader that turns each folder into a verdict with a reason, the Plugins panel
  that shows those verdicts, and `cargo xtask plugin ...` for build, package,
  install and switching. Decisions in
  [`docs/adr/0006-plugin-folders-and-developer-workflow.md`](docs/adr/0006-plugin-folders-and-developer-workflow.md).
- Step 5 — the native plugin boundary: a versioned plugin API, a public SDK,
  a loader that validates a folder and loads its library, and Agent Chat built
  and packaged as a separate project that the host discovers at runtime. The ABI
  gate was measured, not assumed; decisions and the measured constraint are in
  [`docs/adr/0005-native-plugin-boundary.md`](docs/adr/0005-native-plugin-boundary.md).

## Build and run

The toolchain is pinned by `rust-toolchain.toml` (Rust 1.99.1).

```sh
cargo build
cargo test
cargo clippy --all-targets
cargo run
```

### Running without a desktop session

Launch verification in a headless environment uses a private Xvfb display:

```sh
Xvfb :120 -screen 0 1280x800x24 -nolisten tcp &
DISPLAY=:120 cargo run
```

Mesa may warn about missing DRI3 support; software rendering is fine for the
workspace UI.

There is no window manager or xdotool on such a display. The `scripts/x11-*.py`
helpers drive it through XTEST and X11 client messages instead: close a window
gracefully (`x11-close-window.py`), press a shortcut (`x11-key.py`), click
(`x11-click.py`), move the pointer (`x11-move.py`), or drag (`x11-drag.py`).

## Workspace state

Layouts are saved as
`<user data dir>/workspaces/<workspace-id>.json`; set `BRUCEBOX_DATA_DIR` to use a
different root (tests and parallel dev builds do). The default root is
`%APPDATA%/brucebox` on Windows, `~/Library/Application Support/brucebox` on
macOS, and `$XDG_DATA_HOME/brucebox` (or `~/.local/share/brucebox`) elsewhere.

- First launch builds the default preset; later launches restore the saved layout.
- Saves are debounced and coalesced, and flushed when the window closes.
- A document that cannot be restored is moved aside to `.json.invalid` and the
  default preset is used, so a newer build's layout is never silently overwritten.
- `ctrl-shift-r` resets the workspace to the default preset.
- Floating surfaces and their bounds are part of the same document. A surface
  that cannot be restored takes the whole document with it: a half-restored
  workspace is never saved back over the real one.

## Floating surfaces

A floating surface is a group of panels above the docked workspace: it moves by
its title bar, resizes from its edges and corner, raises when clicked, and holds
its own tabs and splits. Panels move between it and the dock as the *same*
panel — no recreation, no state loss.

- `ctrl-shift-f` — float the active panel of the area under the pointer; press it
  again over a floating panel to dock that panel back.
- `ctrl-shift-d` — dock every floating panel back into the workspace.
- The **Dock** button on a surface does the same for that one surface.

Dragging a panel across surfaces is not wired yet; the transfer seam it needs is
in place (see ADR 0003).

## Plugins

A plugin is a folder: a `plugin.toml` manifest plus a platform-specific native
library. It is built in its own Cargo project — never as part of this one — and
loaded at runtime. Adding, updating or removing one needs no host rebuild.

```sh
cargo xtask plugin list                 # what is installed, and may it run?
cargo xtask plugin validate plugins/agent-chat
cargo xtask plugin pack plugins/agent-chat      # build + package, one project
cargo xtask plugin install target/plugin-packages/brucebox.agent-chat
cargo xtask plugin disable brucebox.agent-chat  # also a switch in the UI
```

`pack` builds that one project (never the host) into
`target/plugin-packages/<plugin-id>/`; `install` copies the folder into the user
plugin root. An installation is a plain folder copy, so it can be produced
anywhere and copied in. Startup never runs Cargo.

Roots are searched in order: `BRUCEBOX_PLUGIN_DIR` (`:`-separated development
roots) first, then `<user data dir>/plugins`. A plugin id may be installed once —
the first folder wins and the shadowed copy is reported, so a development copy
reliably shadows an installed one.

### Switching a plugin on and off

`plugins.toml` in the data directory lists the ids that are *off*; anything not
listed is on, which is what makes installing a folder enough. The **Plugins**
panel shows one row per folder the host looked at — ready, disabled, refused with
the reason, or missing a dependency — and its switch writes that file.

A switch takes effect on the next start, and the panel says so while it is
pending. Unloading a library that the open workspace may still be holding a
pointer into is not something this boundary promises (ADR 0005), so "off" means
"not admitted next time", not "unloaded now".

### Dependencies

A manifest may require other plugins by id, with an optional version prefix:

```toml
[dependencies]
requires = ["brucebox.git", "brucebox.workspace@0.1"]
```

Requirements are checked at discovery against the plugins that are installed and
enabled. A dependency that is missing, switched off, or at another version keeps
the depending plugin out of the workspace with a reason that names it. Whether a
required *service* is actually published remains Cordis's business at runtime.

A plugin project depends on `brucebox-sdk`, declares its panels and its Cordis
contribution, and exports one symbol:

```rust
declare_plugin!(Plugin::new("com.example.hello", "Hello", "0.1.0")
    .panel("main", "Hello", create));
```

`plugins/agent-chat` is the working example: a real GPUI conversation panel with
a composer, its own persisted state, and a session service published into the
host's composition runtime.

### What a plugin must match

The pinned toolchain, the pinned `gpui-kit` and `cordis-rs`, and the plugin API
version. The host refuses a package that declares anything else, with the
declared and supported versions named. A native plugin is trusted in-process
code with the privileges of Brucebox; this is a compatibility contract, not a
sandbox.

### What crosses the boundary

Rendering crosses freely: a plugin builds real GPUI entities and elements, and
the host renders them. **Input does not**, and not by choice — GPUI delivers an
event by downcasting it inside `Any`, and the same type has a different `TypeId`
in two separately compiled libraries, so a plugin's own `on_key_down` or
`on_click` would never run. The host keeps the handlers and forwards key and
pointer events to the plugin as plain data (`PluginPanelInput`). See ADR 0005
for the measurement and the consequences.

## Composition runtime

There is one Cordis runtime, owned by the host, on its own worker threads
(`cordis-rs 0.11.1`). It owns plugin lifecycle, service dependencies, typed
events, and generation-owned cleanup — the workspace does not have a second event
bus, DI container or lifecycle system, and it does not import Cordis.

Plugins run off the application thread, so the boundary between them and the UI
is a mailbox of plain data, not a shared `&mut App`:

- a plugin **declares** its panels (ids, titles, multiplicity); the factory that
  builds them stays on the application thread, because it produces GPUI entities;
- a plugin **requires** services it cannot work without, and stays `Pending`
  until they exist instead of failing;
- lifecycle phases, declarations and withdrawals arrive as messages that the
  application thread applies to the catalog and to the **Activity** panel.

The panels this build ships are submitted as a plugin through that same path, so
there is no special case for built-in contributions.

## Layout

```text
crates/
├── brucebox-plugin-api/     the versioned host/plugin boundary (repr(C), pinned)
├── brucebox-plugin-manifest/ the folder format: manifest, versions, dependencies
├── brucebox-plugin-install/  roots, plugins.toml, per-folder verdicts
├── brucebox-plugin-loader/   opening a library and validating its registration
├── brucebox-sdk/             what a plugin project depends on
└── xtask/                    the developer workflow: list, validate, pack, install

plugins/
└── agent-chat/             a first-party plugin, built as its own project

src/
├── main.rs                 binary: GPUI bootstrap, then the host window
├── lib.rs                  host library
├── app.rs                  application shell
├── plugins/                host side of plugin folders: load, declare, report
├── runtime/
│   ├── mod.rs              the host-owned Cordis runtime and its lifecycle
│   ├── contract.rs         the workspace service and typed events
│   ├── composition.rs      the host's own plugins (adapter, contributions)
│   ├── bridge.rs           runtime → UI mailbox, as plain data
│   ├── ui.rs               applying those messages with `&mut App`
│   └── error.rs            composition failures
└── workspace/
    ├── ids.rs              plugin, panel type, panel instance identity
    ├── catalog.rs          panel descriptors, factories, plugin records
    ├── declaration.rs      what a plugin contributes, as data
    ├── layout.rs           layout data and planning (pure)
    ├── preset.rs           first-run layout data
    ├── document.rs         the versioned, persisted workspace document
    ├── store.rs            where that document lives and how it is written
    ├── surface.rs          floating surface identity and geometry (pure)
    ├── floating.rs         the live floating surface and its drags
    ├── dock.rs             live dock areas, panel transfers, save lifecycle
    ├── status.rs           what the composition runtime is doing
    ├── contributions.rs    panel factories and the declaration this build ships
    ├── panels/             host panel handle, placeholder, proving panels
    └── presentation/       replaceable dock renderer
```

The docked UI uses **GPUI Kit Base** (`gpui_kit::base::dock`) with a custom,
replaceable renderer. No application regions are predefined: panels are placed
only through the catalog and a layout.

## Rules of thumb

- The workspace never imports a concrete plugin panel implementation.
- Plugin crates are not members of this Cargo project and are not built by it.
- Normal startup never runs Cargo.
- Missing or failed plugins show placeholders and keep persisted layout state.
- The workspace learns about panels and plugin lifecycle as data; only
  `runtime/` names a Cordis type.
- Plugin projects are separate builds. Only `crates/brucebox-plugin-loader`
  loads a library or resolves a symbol.
- A plugin panel renders whatever it likes; its input arrives from the host as
  plain data, because GPUI's own listeners cannot fire across the boundary.
- Tooling and host read one parser: a package cannot be built that the host reads
  differently, and a switch in the panel and `cargo xtask` write the same file.
