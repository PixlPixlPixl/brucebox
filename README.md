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
    ├── dock.rs             live GPUI Base Dock area
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
