# ADR 0001 — Foundation baseline and the panel catalog

- Status: accepted
- Date: 2026-11-05
- Scope: PLAN.md steps 0 and 1
- Related: SPEC.md §3, §4.1–4.3, §7; PLAN.md "Execution sequence"

## Context

The repository was a GPUI shell with three temporary panels constructed inline
in `workspace/dock.rs`. The plan's first two steps ask for (0) a verified
baseline with recorded decisions and (1) a panel catalog that decouples panel
identity, ownership, and creation from workspace and dock construction.

## Decisions

### 1. Plugins are runtime-loaded native libraries, never static members

The historical foundation roadmap (source plugins, static linking, native
libraries out of scope) is superseded. The host must not compile, link, or
register plugin crates at build time, and normal startup never runs Cargo.
First-party plugins will be standalone projects packaged into the same folder
format as third-party plugins.

Consequence for this step: the workspace may only learn about panels through
the catalog, never through `use crate::plugins::...`. That constraint is what
the later dynamic loader will rely on.

### 2. Pinned foundation

- Rust toolchain pinned by `rust-toolchain.toml` (1.99.1).
- `gpui-kit = 0.9.0` with `default-features = false`, which gives GPUI plus
  GPUI Base (`gpui_kit::base`) and no styled component layer.
- Docking uses `gpui_kit::base::dock` (`DockArea`, `DockLayout`, panel registry)
  with a custom, replaceable `DockAreaRenderer`.

`gpui-kit`'s `test-support` feature cannot be enabled in this environment: it
pulls `gpui-fast`, which requires `proptest ^1.12.0`, and that version is not in
the available registry snapshot. Catalog and layout logic is therefore tested as
pure data, not through `TestAppContext`. A GUI-level test harness can be added
when the feature resolves again.

### 3. Verification is a build plus an Xvfb launch

There is no wayland socket in this environment. Launch verification uses a
private Xvfb display and captures a screenshot:

```sh
Xvfb :120 -screen 0 1280x800x24 -nolisten tcp &
DISPLAY=:120 cargo run
DISPLAY=:120 xwd -root -silent > /tmp/shot.xwd
ffmpeg -i /tmp/shot.xwd /tmp/shot.png   # or any xwd reader
```

Mesa prints DRI3 warnings on that display; they are harmless for a software
render of a static layout.

### 4. Three identities, kept apart

- `PluginId` — namespaced `<vendor>.<feature>`, validated (lowercase slug
  segments, at least two segments).
- `PanelTypeId` — `<plugin-id>.<panel-segment>`. This is what layouts and the
  dock persist, and what the dock's restore registry is keyed by.
- `PanelInstanceId` — minted per live panel, independent of GPUI `EntityId`, so
  it can be persisted and restored later.

Every panel is constructed with a `PanelIdentity` carrying both ids, and can
report it. Two `Scratch Pad` panels therefore share a type id and differ by
instance id, which is what SPEC §4.2 requires.

`gpui-base::dock::Panel::panel_name` is `&'static str` while plugin ids only
exist at runtime, so ids are interned once (`ids::intern`) and leaked for the
process lifetime. The leak is bounded by the number of registered
contributions. Pointers are stable, so the dock registry key and the panel's own
answer cannot drift.

### 5. Two registries, one bridge

- `workspace::catalog::PanelCatalog` (a GPUI global) is the Brucebox-level
  registry: owner plugin, title, icon, multiplicity, factory, and the plugin
  records that make ownership checkable.
- `gpui_kit::base::dock::PanelRegistry` is the dock's own name → builder map used
  when a persisted layout is restored.

`catalog::register_contribution` registers in both, so a restored panel and a
freshly opened one go through the same factory. The catalog is not a wrapper
around the dock registry: it holds the metadata the dock has no concept of, and
the dock keeps the mechanics (tree, ids, drops) the catalog must not reimplement.

### 6. `HostPanel` is a handle, not a framework

The dock stores `Arc<dyn PanelView>`, which erases the contribution identity.
`panels::host::HostPanel` wraps a plugin's panel entity so the dock can be handed
a stable `&'static` panel name equal to the panel type id and the host can still
ask which contribution and instance a live panel belongs to. Lifecycle hooks,
visibility, focus and `dump` delegate to the plugin's panel; the host only
overrides the persisted `panel_name` so a plugin cannot persist a name the
catalog does not know.

A registry builder returns `Arc<dyn PanelView>` before the host entity exists
(the entity is the view), so the wrapper is constructed by the catalog factory,
immediately after the plugin's panel entity. The wrapper is not generic over the
panel type on purpose: tab titles and metadata come from the catalog by type id,
and a generic wrapper would force the renderer to know every panel type to
downcast to it.

### 7. Multiplicity policy

`PanelMultiplicity` defaults to `Single`; a contribution opts into `Multiple`.
The workspace planner counts placements per type and refuses the extra one, so
the rule holds for presets and, later, for user-opened panels. A preset that
violates it still builds: the duplicate becomes a placeholder with a diagnostic.

### 8. Failure behaviour

| Situation | Behaviour |
| --- | --- |
| Duplicate plugin id | `CatalogError::DuplicatePlugin`; the second registration is rejected, not merged. |
| Duplicate panel type id | `CatalogError::DuplicatePanel`; a plugin cannot shadow another's id. |
| Panel registered for an unregistered plugin | `CatalogError::UnknownPlugin`; a plugin that failed to load contributes nothing. |
| Panel type in a layout has no contribution | Planned as unavailable, rendered by `MissingPanel`, recorded as a diagnostic. The instance id is still reserved. |
| Contribution factory fails | `PanelCreateError::FactoryFailed` is contained: placeholder plus diagnostic, other panels unaffected. |
| Restored layout names an unknown panel | `DebugDockRenderer::build_placeholder` returns a `MissingPanel` that hands the original `PanelState` back from `dump`, so the next save keeps it. |
| Plugin unregistered | Its panel types are removed from the catalog; live layouts show placeholders on the next resolution. |

### 9. Planning is separate from execution

`layout::plan(&WorkspaceLayout, &PanelCatalog)` is pure: it resolves descriptors,
reserves instance ids, checks multiplicity, and reports diagnostics without
touching an `App`. `workspace::dock` executes a plan into a `DockLayout`. This is
why the catalog/layout rules are unit-testable without a window, and it is the
shape persistence in step 2 will reuse.

### 10. Presets are data

`preset::default_layout()` names panel type ids only. Nothing in `dock.rs` or the
renderer mentions a panel; adding or replacing a contribution is a registration
edit plus, at most, a preset edit.

## Deferred deliberately

- **Workspace persistence, saved layouts, floating groups** — steps 2–3.
- **Per-instance dynamic titles** — tab titles currently come from the catalog
  descriptor by type id. Panels that need a live title (open file, branch) get a
  host title service when the first real plugin needs it.
- **Crate split** per SPEC §7 (`crates/brucebox-workspace`, …) — deferred until
  the SDK, plugin API and loader crates exist (steps 5–6). Extracting a single
  crate now would move code without creating a boundary.
- **Cordis runtime** — step 4. The catalog takes no Cordis types so the runtime
  can drive registration later without a rewrite.
- **Temporary proving panels** (`brucebox.demo.*`) — keep the docking smoke test
  alive until `brucebox.agent-chat` and `brucebox.git` are loaded through the
  plugin path (steps 5 and 7). They are registered through the same catalog path
  and carry no business logic.
