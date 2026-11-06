# Brucebox — Agent Plan

## Authority / conflict resolution

- **SPEC.md is the normative product and architecture contract. This PLAN.md is the concise execution map.**
- The earlier brucebox-foundation-roadmap.md is historical input only. Its source-plugin/static-linking and dynamic-library-out-of-scope instructions are superseded by this plan and SPEC.md. Do not follow those obsolete instructions.
- Current objective: finish a reusable native plugin-host/workspace foundation that can be used to build Brucebox's real first-party plugins and later independent third-party plugins. Do not build the full agent product first.
- Use real, retained product features as proving examples: **Agent Chat** and **Git**. No disposable test-only/demo plugins. Tests for logic/contracts are expected; throwaway plugin products are not.
- Prefer small complete vertical slices; avoid speculative frameworks, abstractions, polishing, and micro-milestones. Keep the project building at each meaningful step.

## Target outcome

A working native GPUI host with:

1. A free-form workspace engine (docked + floating groups), panel catalog, saved layouts, and replaceable presets.
2. A host-owned Cordis composition runtime.
3. A versioned public Brucebox SDK/API that third-party plugin projects can consume independently.
4. A runtime plugin manager that discovers folders, validates manifests/compatibility, and loads independently compiled native plugin libraries.
5. Real Agent Chat and Git plugins, packaged and loaded through the same path as external plugins.
6. A default workspace assembled from plugin panel contributions, not hardcoded UI regions.

A developer should be able to build a plugin independently, place its packaged folder in a configured plugin root, restart Brucebox, and use its panels/services without editing or rebuilding the host.

## Current repository baseline (verify before changing)

Observed on main during plan creation:

- Rust 2024 project; rust-toolchain.toml pins Rust 1.99.1.
- Cargo.toml currently depends on gpui-kit = 0.9.0 with default-features disabled.
- The native GPUI application launches; Linux/Wayland smoke test has been completed.
- Workspace currently creates three temporary panels directly in workspace/dock.rs and uses a replaceable custom DebugDockRenderer.
- The current DockArea import is GPUI Kit Base (gpui_kit::base::dock).
- There is not yet a Cordis runtime, panel catalog, persistence layer, SDK, plugin API, or native plugin loader in the inspected baseline.

Treat this as a snapshot, not a reason to preserve temporary demo implementation. Re-check the actual tree, lockfile, dependency source and current build before each milestone; do not assume files or APIs are unchanged.

## Progress

- **Step 0 — bootstrap verified.** Build and launch re-checked on Rust 1.99.1 with gpui-kit 0.9.0 (GPUI Base dock, custom renderer). Launch was verified on a private Xvfb display because this environment has no Wayland socket. Baseline decisions, including why gpui-kit's `test-support` cannot be enabled here, are recorded in `docs/adr/0001-foundation-baseline-and-panel-catalog.md`.
- **Step 1 — panel catalog and workspace decoupling done.** Namespaced `PluginId`/`PanelTypeId`/`PanelInstanceId`; a catalog of descriptors (owner plugin, title, icon, multiplicity, factory, restore payload) that rejects duplicate ids and contributions from unregistered plugins; a bridge from the catalog into GPUI Base Dock's own panel registry; layout planning kept separate from execution; missing panels become state-preserving placeholders with diagnostics; workspace and dock construction resolve contribution ids only. The `brucebox.demo.*` proving panels stay registered through the same path until the real plugins load in steps 5 and 7. 22 unit tests, Clippy clean. Next: step 2, workspace persistence and default preset.
- **Step 2 — workspace persistence and presets done.** Versioned `WorkspaceDocument` (schema + workspace id + dock state) written to `<user data dir>/workspaces/<id>.json` with atomic writes and `BRUCEBOX_DATA_DIR` override; panel instance ids and owner plugin persist inside a host envelope in each leaf state, so the dock tree stays the only layout source; restore runs through the same catalog factory path; missing plugins and owner mismatches become state-preserving placeholders; saves are debounced and coalesced, with a window-closed flush so an edit made just before quit is not lost; missing/corrupt/foreign-schema documents are quarantined to `.json.invalid` and the default preset is used; `ctrl-shift-r` is an explicit reset. Verified headlessly (Xvfb + `scripts/x11-*.py`): fresh save, hand-edited restore, missing-panel round trip, corrupt-file quarantine, debounced tab-change save, and reset. 36 unit tests, Clippy clean. Decisions in `docs/adr/0002-workspace-persistence.md`. Next: step 3, floating surfaces.
- **Step 3 — floating surfaces done.** A surface is another GPUI Base Dock area drawn above the docked workspace with its own bounds and stacking position, so panels keep the same tabs and splits inside it. Panels move between areas as the same entity: the destination is given the view before the source lets it go, and the host handle withholds the dock's *removed* notice for the duration of the move, so no panel is torn down or rebuilt. The view is recovered for removal through `PanelHandle` (`PanelView::as_any`), the one place a transfer needs the concrete panel type. Moving, resizing (edges and corner) and raising are drags on the surface chrome; `ctrl-shift-f` / `ctrl-shift-d` and a per-surface **Dock** button cover cross-surface transfer until a workspace drag coordinator exists. Bounds, stacking and each surface's dock state persist in document schema 2, which still reads schema 1; restore is all-or-nothing so a half-restored workspace is never saved back. Verified headlessly: float, title-bar move, corner resize, window-closed flush inside the debounce window, restart restore, float-to-surface and dock-back, dock-all, missing panel inside a surface, reset with surfaces open, and two-surface stacking. 49 unit tests, Clippy clean. Decisions in `docs/adr/0003-floating-surfaces.md`.
- **Step 4 — Cordis composition runtime done.** `cordis-rs 0.11.1` is pinned and verified against its published API: one host-owned runtime on its own worker threads, one root context, and every lifecycle handle kept by the host. Plugin apply bodies run off the application thread, so the boundary is a mailbox of plain data — declarations, withdrawals, lifecycle phases and status lines — that the application thread applies to the catalog and to an Activity panel; no Cordis type reaches the workspace and no GPUI type reaches a plugin. A contribution crosses as a declaration (ids, titles, multiplicity) while the factory that builds the panel stays on the application thread, and `register_declaration` joins them, refusing a plugin whose build is missing one factory rather than landing half-contributed. Services are real dependencies: a plugin that requires the workspace service parks `Pending` and never applies until it is published, then converges and delivers its contribution on its own. The host's own panels are submitted as a plugin through the same `start_plugin` path a loader will use, with `WorkspaceAdapter` publishing the service and the built-in contributions publishing a `PanelsDeclared` event. Disposal is explicit and host-owned: reverse spawn order, attempt-all, with each plugin withdrawing its own contributions from a generation-owned cleanup, because dropping a Cordis handle is not teardown. Verified headlessly: declarations reach the window, both plugins show `active`, the generation-owned heartbeat ticks, graceful close exits through the blocking dispose, and a relaunch restores the saved layout with no placeholders. 61 unit tests, Clippy clean, `cargo fmt --check` clean. Decisions in `docs/adr/0004-cordis-composition-runtime.md`.
- **Step 5 — SDK and native ABI proof done.** The boundary is measured, not assumed: two builds with the same pinned toolchain and dependency versions but different graphs give a `TypeId` for the same type that differs across the edge, so GPUI's own event listeners (which downcast through `Any`) cannot run inside a plugin library, while rendering, `dyn` trait objects and plain data cross fine. That result shaped the design instead of being papered over. `crates/brucebox-plugin-api` holds the whole `#[repr(C)]` boundary (entry symbol, registration, panel declarations, panel factory, Cordis contribution pointer); `crates/brucebox-sdk` is what a plugin project depends on (`Plugin` builder, `declare_plugin!`, re-exported GPUI Kit and Cordis, `interactive_panel`); `crates/brucebox-plugin-loader` owns every `unsafe` byte of discovery, manifest validation, platform-library resolution, symbol lookup and registration validation, and keeps the library handle for the process lifetime. Plugin projects are separate builds with their own lockfile, excluded from the host workspace, packaged by `scripts/package-plugin.py` into `target/plugin-packages/<id>/`; startup never runs Cargo. Agent Chat is the proof: a real GPUI conversation panel with a composer and its own persisted state, an SDK-declared panel, and a session service published into the host's Cordis runtime. Because a plugin's own listeners cannot fire, the host keeps the handlers and forwards keys and pointer events as `PluginInput` through a vtable call; the plugin's layout and painting stay unrestricted. Verified headlessly: packaged folder loads without a host rebuild, panel renders inside a restored layout, typed input and Return reach the panel through the bridge, the conversation round-trips through the workspace document, removing the folder restores a *Panel unavailable* placeholder with the saved state intact, and a package declaring `api_version = 99` is refused with both versions named while the host keeps running. 81 tests, Clippy and `cargo fmt --check` clean. Decisions in `docs/adr/0005-native-plugin-boundary.md`. 
- **Step 6 — plugin folders and the developer workflow done.** The folder format is now a shared contract rather than three copies of the same rules: `crates/brucebox-plugin-manifest` owns `plugin.toml` (schema, plugin API and SDK versions, platform keys, declared plugin dependencies), `crates/brucebox-plugin-install` owns the data directory, the ordered roots, `plugins.toml` and the per-folder verdict, and `crates/brucebox-plugin-loader` still holds every `unsafe` byte and nothing else. Roots are searched development-first and then the user root, folders in name order, one folder per id — a development copy shadows an installed one and the shadowed copy is *reported* instead of vanishing. Dependencies (`requires = ["brucebox.git@0.1"]`) are checked at discovery against installed and enabled packages, and a missing, switched-off or incompatible dependency keeps the plugin out with a reason that names it and the version that is actually installed; service-level requirements stay Cordis's job. Enable/disable is `plugins.toml` plus restart and never an unload, because ADR 0005 rules out unloading code the workspace may still point into — the Plugins panel therefore labels the pending change instead of pretending. `cargo xtask plugin list|validate|pack|install|enable|disable` replaces the Python packaging script; `pack` builds one plugin project in its own workspace and the host still never runs Cargo. The Plugins panel shows the host's own verdicts, one row per folder, with the config path and the switch. Verified headlessly: a packaged folder copied into the *user* root loads with no environment variable and no rebuild; the panel reports `ready`; disabling from the panel writes the config and shows the pending state; after a restart the plugin is `disabled` in both the panel and Activity; enabling writes it back and the next start loads it again; `cargo xtask plugin list` agrees with the panel at every step and `validate` refuses a non-namespaced id with the host's own message. 102 tests across the workspace, Clippy and `cargo fmt --check` clean. Decisions in `docs/adr/0006-plugin-folders-and-developer-workflow.md`. Next: step 7, the Git vertical slice.

## Architecture / responsibility boundaries

### Host
- Owns one GPUI Application/window, startup/shutdown, plugin manager, workspace host services, and the one shared Cordis runtime.
- Does not contain Agent Chat/Git business logic.
- Does not compile or statically link installed plugins.
- Normal startup never runs Cargo.

### Plugin loader
- Small isolated subsystem: configured plugin roots → folder enumeration → manifest validation → platform binary resolution → API compatibility/dependency checks → native library load → entry point → registration.
- Owns library handles and loader diagnostics. Keep unsafe symbol resolution and ABI details here, not in workspace code.
- Load failure for one plugin must not crash the host or prevent other compatible plugins from loading.
- Missing/unavailable plugins must leave workspace state recoverable.

### Plugin API / SDK
- Keep a narrow versioned ABI boundary separate from the ergonomic public SDK.
- API covers plugin identity/version, entry point, host context, contribution registration, compatibility negotiation, and shutdown.
- SDK exposes the supported GPUI/GPUI Kit APIs, panel/command contribution APIs, access to host services, and Cordis integration.
- SDK is an integration contract, **not** a component library or a fixed capability catalog.
- Plugin projects must build outside the host Cargo workspace. First-party source projects may live in this repository, but are not host workspace members/dependencies.

### Cordis runtime
- Cordis is the composition system: plugin lifecycle, dependency ordering, service registration/injection, typed events, subscriptions, async tasks and teardown/resource ownership.
- Use one host-owned runtime so separately loaded plugins can share services/events.
- Add a thin Brucebox adapter to connect Cordis registration/lifecycle to workspace panel contributions.
- Do not build a second event bus, DI container, service locator, or plugin lifecycle runtime.
- Cordis does not render UI, own DockArea state, choose panel positions, or dictate component styling.

### Workspace engine
- Owns the panel catalog/instance identity, surface model, placement, persistence and presets.
- Does not import concrete Agent Chat/Git implementation modules.
- Knows a plugin through stable contribution IDs/factories, not source-level coupling.
- Distinguish stable panel type IDs from unique panel instance IDs.
- Panel factory metadata should cover owner plugin, title/icon, multiplicity, create/restore behavior and opaque panel state.
- Bridge the Brucebox catalog to GPUI Base's own Dock restoration/factory registry; these are related but distinct registries.
- Default layout is first-run data/preset only. Never force fixed sidebar/main/bottom/inspector slots.

## GPUI / GPUI Kit rules

- Keep the pinned GPUI Kit dependency as the GPUI facade; do not add a second incompatible GPUI/gpui-component/gpui-base version.
- Use **GPUI Kit Base** for the DockArea/docking engine. Current code already uses gpui_kit::base::dock.
- Do not require the styled GPUI Component layer or a shared component library. Use a custom DockAreaRenderer/presentation layer; replace/debug-renderer polish is secondary to correct workspace behavior.
- Let GPUI Kit Base handle the dock-tree mechanics it already supports: nested splits, tabs, drag/drop, tab activation/reordering, resizing and supported state serialization/restoration. Inspect the exact pinned version before using APIs.
- Brucebox adds what Base Dock does not provide: free-form surface composition, independent floating groups, cross-DockArea transfer coordination, workspace-level persistence/presets, and plugin-aware missing-panel recovery.
- Plugin UI is real GPUI in the host window. Plugins may use GPUI primitives, custom Render/RenderOnce views, custom elements/painting/animations/focus behavior, or their own component library. GPUI Kit styled widgets are optional.
- GPUI entity/window state stays on the GPUI application thread. Bridge async Cordis/plugin events back to GPUI through host-owned dispatch/message mechanisms; do not mutate GPUI state from arbitrary background tasks.
- Keep rendering/presentation independent from plugin services/contracts.

## Plugin model: independently built folders, runtime loaded

**This is the required model. Plugins are not compiled into the Brucebox executable.**

### Build and install
- Each plugin is a separate Rust project with its own Cargo build/dependency graph, built against an explicit compatible Brucebox SDK/API + pinned GPUI/GPUI Kit/Cordis/toolchain set.
- Build output is packaged as a self-contained plugin folder: manifest + platform-specific native library + optional assets/resources.
- Example shape:

~~~text
plugin-id/
├── plugin.toml
├── bin/<target-triple>/<native-library>
└── assets/...
~~~

- Manifest must declare stable plugin ID, plugin version, manifest/API/SDK compatibility, library path for current OS/architecture, optional dependencies, and optional asset metadata.
- Development plugin source and runtime-installable plugin packages are distinct. Suggested repo arrangement: plugin source projects under plugins/ (or another source root); package outputs under target/plugin-packages/<plugin-id>/; installed user plugins in the platform's Brucebox user-data plugin directory. Only scan configured **package** roots, not arbitrary source trees.
- The first-party source projects are also standalone projects and are not members of the host Cargo workspace. Their packaged output uses the exact same manifest, library and loader format as third-party plugins.
- A small CLI/xtask may build, validate, package and copy a selected plugin. It must run Cargo for that plugin only; it must never aggregate plugin crates into the host build.
- No Cargo invocation during normal GUI startup. No generated static Rust plugin registry/dependency bundle. No host-source edits for plugin addition.
- Restart to apply plugin additions/updates/removals is acceptable. No in-process machine-code hot replacement or hot-unload is required.

### Runtime
- Discover configured roots, validate manifest/path/ID/version/platform/dependency, load enabled compatible libraries, resolve a versioned entrypoint, and register contributions with host services/workspace/Cordis.
- Keep libraries loaded until every plugin-owned panel, factory, callback, service, task, vtable and destructor has been disposed. Initial safe default: retain handles until application shutdown, then tear down plugin resources before releasing handles.
- Native plugins are trusted in-process code with host privileges. No sandbox, WebView isolation, or fine-grained permission system is part of this foundation.
- API/SDK incompatibility or load errors should be clear and isolated; preserve workspace placeholder/state for absent plugins.

### ABI feasibility gate — do not hand-wave

- Rust does not promise a stable general ABI for arbitrary Rust types, trait objects, GPUI entities, or callbacks across independently compiled libraries. libloading/C entrypoints alone do not solve this.
- Before expanding the SDK, prove the real use case end-to-end: independently compile Agent Chat; discover/load its folder; register a real native GPUI panel; use GPUI interaction/rendering; register into the host-owned Cordis composition runtime; shut down safely while retaining library lifetime.
- Keep ABI-sensitive work in the narrow plugin-api/loader boundary. Explicitly pin/validate compiler, target, GPUI Kit/GPUI, Cordis and SDK compatibility. Add an entrypoint/API version handshake and reject incompatibility before registration.
- Matching crate versions are not a universal ABI guarantee. Do not promise arbitrary toolchain/version interoperability.
- If the boundary cannot safely carry the native GPUI functionality required by plugin authors, stop and resolve the design before implementing more features. Do not silently replace native GPUI with serialized UI descriptions/WebViews.

## Plugin capabilities

A plugin may be:
- UI-only, runtime/service-only, or both.
- A contributor of multiple panel types, commands, workspace presets, services, events and background tasks.
- A provider/consumer of typed services and events defined in plugin-owned contract crates.
- A provider of multiple independent panel instances with panel-specific serializable state.
- A consumer of host services exposed through the SDK.

No plugin is forced to use a specific panel slot or host visual style. The host controls registration and workspace placement; plugin code controls its internal interface and feature behavior.

## Cordis: why it is here

Use Cordis instead of hand-rolling:
- **Lifecycle**: startup, initialization, deactivation and cleanup.
- **Dependency graph**: plugin/service dependencies and ordering/availability.
- **Service composition**: typed service registration and injection/consumption.
- **Events**: typed publish/subscribe across independent plugin implementations.
- **Async/resource ownership**: tasks, subscriptions and disposables tied to plugin lifecycle.

Brucebox should adapt Cordis to the host/SDK boundary, not wrap every Cordis primitive without need. Plugins share the host runtime; they should not each create isolated runtimes. Cordis knows composition, not GPUI rendering or docking.

## First-party proving features (real product code)

### Agent Chat — brucebox.agent-chat
- Native GPUI conversation panel; message history + composer/input.
- Minimal useful send/receive or session seam; keep the path open for later agent/provider/ACP integration without making a full agent orchestrator part of the foundation.
- Owns its UI/state and future conversation persistence.
- Should consume Git repository/context service when available.

### Git — brucebox.git
- Basic repository open/selection, current branch/status, changed-files list and diff inspection.
- Owns its UI/state; full Git client is out of scope.
- Publishes typed repository/context service and repository-change events through Cordis.
- Agent Chat consumes Git's contract without importing Git's implementation.

Keep the first slice deliberately basic but retain and expand it. These two real plugins prove native UI, independent load, Cordis sharing and layout independence. Do not recreate them as throwaway test plugins.

## Execution sequence

Work sequentially, but continue through a milestone once its acceptance conditions are met; do not stop for approval after every small task. Resolve hard blockers instead of bypassing them.

### 0. Preserve/verify bootstrap
- Verify current native GUI build and launch using pinned toolchain/dependencies.
- Inspect GPUI Kit 0.9.0 Dock/Base sources/examples before changing APIs.
- Keep the existing custom-rendered DockArea smoke test working while replacing temporary Welcome/Scratch/Activity panels.
- Record important long-lived decisions briefly (ADR/design note); avoid paperwork for trivial changes.

### 1. Panel catalog and workspace decoupling
- Introduce namespaced plugin IDs, panel type IDs and unique panel instance IDs.
- Implement panel descriptors/catalog: register, lookup, enumerate, create, multiplicity, owner, metadata, restore/state hooks.
- Define duplicate-ID, plugin failure, and missing-panel behavior.
- Bridge catalog to GPUI Base Dock's own panel restore/factory mechanism.
- Refactor workspace/dock construction to resolve panel IDs; no concrete plugin panel imports or hardcoded permanent demo panel path.

**Done:** a contribution can register a panel without editing DockArea/workspace construction; panel type and instance IDs are distinct; existing docking still works.

### 2. Workspace persistence and default preset
- Inspect pinned DockArea state/change/restore APIs.
- Define versioned WorkspaceDocument: workspace ID/schema, dock state, surface list, panel type + instance IDs, plugin owner IDs and opaque panel state.
- Save/restore with GPUI Base state; do not duplicate the Dock tree.
- First launch applies default preset only; later launches restore user state. Reset-to-default is explicit.
- Debounce/coalesce layout writes; preserve unresolved entries/placeholders when plugin missing or disabled.

**Done:** rearrange/close/relaunch restores layout; absent plugin does not erase the user's layout/state.

### 3. Floating surfaces
- Add workspace-level floating group model with bounds, z-order/focus and inner DockArea where useful.
- Implement create, move, resize, detach, redock, basic drop previews and cross-surface transfer.
- Preserve panel identity/state; avoid unnecessary panel destruction/recreation.
- Persist floating geometry and membership in WorkspaceDocument.
- Start with one in-window floating group; defer multi-monitor/advanced polish.

**Done:** panels move between docked and floating groups, remain usable and survive restart.

### 4. Cordis composition runtime
- Add/pin cordis-rs 0.11.1 after verifying real APIs/toolchain; use one host-owned runtime.
- Implement a thin Brucebox runtime adapter for plugin lifecycle, dependency declaration/ordering, services, events, async work and cleanup.
- Connect plugin registration/unregistration to the panel catalog without coupling catalog/workspace internals to Cordis types.
- Bridge events/async operations to GPUI thread safely.
- Define plugin-owned resource teardown. No bespoke event bus/DI/lifecycle system.

**Done:** runtime starts/stops; plugin contributions can publish/consume typed services/events; dependencies and cleanup behavior are explicit; Cordis remains UI-agnostic.

### 5. SDK and native ABI proof
- Build narrow versioned brucebox-plugin-api and public brucebox-sdk interfaces; keep raw ABI/unsafe code isolated.
- Expose GPUI/GPUI Kit Base + host contributions + Cordis integration; no mandatory UI component system or fixed regions.
- Document/pin toolchain and compatibility set.
- Implement the smallest actual dynamic plugin entrypoint/registration boundary.
- Build Agent Chat as a standalone plugin project (not a host workspace member); prove independent build + runtime load + a real GPUI panel + host Cordis contribution + safe shutdown/library retention.
- Resolve ABI problems now, before multiplying plugin implementations.

**Done:** independently compiled Agent Chat folder can be loaded and contribute native GPUI UI safely without host rebuild; incompatibility is rejected clearly.

### 6. Plugin folder manager and developer workflow
- Finalize manifest/folder format and platform binary naming/selection.
- Implement configurable development and user plugin roots, deterministic discovery, duplicate detection, compatibility/dependency checks, enable/disable config and isolated load diagnostics.
- Implement packaging and optional CLI/xtask for one plugin project at a time.
- Keep source projects separate from packaged plugin folders; normal app startup does not run Cargo.
- Retain library handles for process lifetime; restart activates plugin updates.
- Test malformed/missing/incompatible binaries with focused loader tests, not synthetic plugin products.

**Done:** copying a compatible packaged folder into a configured root makes the plugin available after restart; no host source change or rebuild.

### 7. Git + real cross-plugin vertical slice
- Build Git as a second standalone plugin using the identical SDK/loader path.
- Implement basic real repo status, changed files and diff.
- Publish a typed Git repository/context contract and events.
- Have Agent Chat consume Git context/event through the shared Cordis runtime.
- Verify both UIs are movable/splittable/tabifiable/floatable and that plugin communication is independent of placement.
- Exercise missing-plugin restoration and failure isolation.

**Done:** two separately compiled native plugins render and interoperate via Cordis.

### 8. Framework template / default workspace
- Define a replaceable first-run preset using registered Agent Chat/Git panel IDs.
- Make a minimal plugin/panel catalog or launcher so users can discover/open contributions; it is not a permanent dock slot.
- Populate default workspace from plugin registrations, not hardcoded panels.
- Verify full dock/floating behavior, multiple instances where supported, persistence, reset, restart and missing-plugin recovery.
- Ensure third-party plugins use the exact same package, API, registration and runtime path.

**Done:** Brucebox is a connected template/framework ready for ongoing plugin and workspace development.

## Definition of foundation complete

- Host launches natively and uses GPUI Kit Base Dock with replaceable presentation.
- No fixed application regions; layout is user-owned, versioned, persisted and supports docked + floating groups.
- Workspace constructs panels only through the catalog/restore mechanism.
- Cordis is the one composition runtime for independently loaded plugins.
- A separately built plugin folder is discovered/validated/loaded at runtime; no host rebuild or Cargo on app startup.
- Native GPUI panels, services, events and background functionality are exposed through a versioned SDK boundary.
- Agent Chat and Git are real first-party plugins built and packaged independently; Git context reaches Agent Chat through Cordis.
- A useful first-run workspace is a replaceable preset assembled from those plugins.
- Missing/incompatible plugins fail gracefully and saved layout is retained.
- Formatting, check/build, tests and Clippy pass; update this plan and SPEC when architecture decisions materially change.

## Working rules

- Inspect exact pinned source/API; avoid guessing GPUI Kit or Cordis APIs.
- Never use sudo for Cargo/Rust project builds.
- Keep the host Cargo workspace free of installed plugin crates. First-party plugin projects build independently.
- Do not generate static plugin registries or plugin dependency bundles.
- No WebView plugin UI, no sandbox/security subsystem, no machine-code hot unload, no marketplace in this foundation.
- Unit/integration tests are good; disposable test plugins/products are not.
- Avoid broad refactors unrelated to a milestone. No over-engineering or speculative abstractions.
- At each milestone: implement → format/check/build → focused tests/Clippy → update progress and docs → continue.
