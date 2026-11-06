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

## Layout

```text
src/
├── main.rs                 binary: GPUI bootstrap, then the host window
├── lib.rs                  host library
├── app.rs                  application shell
└── workspace/
    ├── ids.rs              plugin, panel type, panel instance identity
    ├── catalog.rs          panel descriptors, plugin records, dock bridge
    ├── layout.rs           layout data and planning (pure)
    ├── preset.rs           first-run layout data
    ├── document.rs         the versioned, persisted workspace document
    ├── store.rs            where that document lives and how it is written
    ├── surface.rs          floating surface identity and geometry (pure)
    ├── floating.rs         the live floating surface and its drags
    ├── dock.rs             live dock areas, panel transfers, save lifecycle
    ├── contributions.rs    contributions this build ships
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
