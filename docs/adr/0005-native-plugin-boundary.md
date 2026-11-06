# ADR 0005 — The native plugin boundary: what actually crosses a library edge

- Status: accepted
- Date: 2026-11-05
- Scope: PLAN.md step 5
- Related: SPEC.md §5.1–5.4, §5.6, §6; ADR 0004

## Context

Step 5 is the ABI feasibility gate. SPEC §5.4 says, in effect: prove it end to
end before multiplying plugins, and if the boundary cannot carry real native
GPUI, stop and resolve the design rather than quietly weakening it.

The host and a plugin are separately compiled `cdylib`s with different
dependency graphs. Rust promises nothing across that edge: layouts, trait
objects and `Any` all rest on shared compilation, not on the source language.

## What was measured

Two builds, same pinned toolchain (1.99.1), same `gpui-kit = 0.9.0` and
`cordis-rs = 0.11.1`, different dependency graphs (the host also links Cordis,
Tokio, serde; the plugin does not):

```text
HOST   TypeId<KeyDownEvent> = 0xab3cfe3ed52f417bf2653c107e57b776
PLUGIN TypeId<KeyDownEvent> = 0xd8280b36f26e819e1a962bc73f1affcc
HOST   TypeId<App>          = 0xc707af51caef1688c1d8e1b563a88c2d
PLUGIN TypeId<App>          = 0xa37aa5bec6df8b6a46e44d3245d9a16b
```

The same Rust type has two identities in the one process. Two consequences
follow, and they are not optional:

1. **GPUI event listeners inside a plugin never fire.** GPUI dispatches an event
   by downcasting `&dyn Any` to the concrete type (`KeyDownEvent`,
   `MouseDownEvent`, …). The downcast silently fails across the boundary, so a
   plugin's `on_key_down`, `on_click` or `cx.listener` is dead code that compiles.
2. **`Entity<T>` updates of *host-defined* types are unusable inside a plugin**,
   and so are globals and `EventEmitter` events whose type is defined by the
   host: they are keyed by the same `TypeId`. (`Entity<PluginType>` works, because
   that identity is created and consumed by the same library.)

Everything else does work, because it is either a vtable call or plain data:
`Arc<dyn PanelView>` and `dyn` trait objects, ordinary structs and enums, and
the rendering pipeline the plugin drives by returning its own element tree.

## Decisions

### 1. The boundary is data plus function pointers, and it is small

`brucebox-plugin-api` holds all of it: a version constant, an entry symbol, a
`#[repr(C)]` registration, panel declarations with a factory pointer, and a
runtime-contribution pointer that returns a `cordis::PreparedPlugin`. Plugins
and the host agree on one pinned set: the toolchain, `gpui-kit`, `cordis-rs`,
the API version and the SDK version. The host checks what it can (manifest,
API version, id agreement) and rejects mismatches before calling in.

The plan's alternative — serializing UI descriptions — was rejected in SPEC
§5.4 and remains rejected: the plugin builds real GPUI entities, elements and
paint code, and the host calls into that code on its own application thread.

### 2. Input crosses as data, because listeners cannot

A plugin panel that wants to be interactive implements `PluginPanelInput`; the
host owns the actual GPUI listeners. `HostPanel` wraps a plugin view in its own
element, tracks the wrapper's focus handle (host code, where dispatch works),
reads keys and pointer events, and forwards `PluginInput` — key name, produced
character, modifiers, pointer position — through a vtable call.

That is what makes Agent Chat's composer work at all. It is also the honest
statement of the constraint: a plugin's *layout and painting* are unrestricted,
its *event handling* goes through the SDK.

### 3. Contributions are declared; the loader pairs them up

A library's registration is ids, titles and multiplicity plus factory pointers;
the host turns that into a `PluginDeclaration` and pairs it with the factories in
`PanelFactories`, exactly as ADR 0004 already did for the built-in panels. The
plugin's own Cordis code is admitted separately (`PreparedPlugin` handed to the
host runtime, host owns the fiber).

### 4. The loader is the only unsafe surface

`brucebox-plugin-loader` reads plugin folders, validates manifests, resolves the
platform library, loads it, resolves `brucebox_plugin_entry_v1`, calls it, and
validates the registration — and keeps the `libloading::Library`. Nothing outside
that crate touches a handle or a symbol, and `LoadedPlugin` cannot be unloaded:
every function pointer, entity and destructor the plugin produced points into it.

### 5. A plugin project is a separate build, always

`plugins/agent-chat` has its own `Cargo.toml`, its own `Cargo.lock`, its own
`[workspace]`, and is excluded from the host workspace. Packaging is
`scripts/package-plugin.py`: build *that* project, copy the manifest and the
library into `target/plugin-packages/<plugin-id>/`. Installing is copying that
folder into a plugin root (`BRUCEBOX_PLUGIN_DIR` or `<data dir>/plugins`).
Startup never runs Cargo.

## Verification

Headless (Xvfb, `BRUCEBOX_DATA_DIR`, `BRUCEBOX_PLUGIN_DIR`, `scripts/x11-*.py`):

1. `scripts/package-plugin.py` builds and packages `brucebox.agent-chat`; the
   host is not rebuilt after that point.
2. Launch with the package root → the plugin's panel appears in a saved layout,
   rendering its own entity.
3. Click the panel, type `hello world`, press Return → the message and the
   agent's reply appear (host-forwarded input).
4. Close the window → the saved document carries the plugin's own payload
   (`{"messages": [...]}`) inside the host's envelope.
5. Remove the package folder and relaunch → the tab restores as *Panel
   unavailable* with the plugin named, and the saved state is kept.
6. Install a copy whose manifest declares `api_version = 99` → the Activity panel
   reports `plugin: brucebox.agent-chat  failed  plugin API version 99 is not
   compatible with this build (it implements version 1)`, and the host runs on.

Tests: 81 across the workspace (`cargo test --workspace`), including registration
validation, manifest validation (schema, API/SDK version, namespaced id, path
escaping, missing platform library), duplicate-id discovery, and the loader's
refusal cases. `cargo clippy --workspace --all-targets` and `cargo fmt --check`
are clean.

## Known limits

- **A plugin's own GPUI listeners do not run** (finding 1 above). The SDK's
  `PluginPanelInput` is the supported route; the host owns the handlers. A
  plugin that needs text editing gets key events, not `EntityInputHandler`, so
  there is no IME, cursor or selection support yet.
- **Host-defined globals and `EventEmitter` types are invisible to plugins** for
  the same reason. Contracts a plugin observes must be defined in a crate both
  sides compile identically, or cross as data — the SDK's contracts do the
  former.
- The compatibility promise is narrow and explicit: same toolchain, same pinned
  GPUI Kit and Cordis, same API/SDK version. A plugin built differently is
  rejected, not "best effort". None of this is a sandbox: a plugin is trusted
  in-process code.
- Pointer input is forwarded but Agent Chat does not use it yet: clicking focuses
  (host side), and sending is Return. A clickable control needs the panel to know
  its own painted geometry, which lands with the first plugin that needs it.
- Enable/disable configuration, multiple roots with precedence, and packaging
  ergonomics are step 6. Today every discovered, valid package is loaded.
