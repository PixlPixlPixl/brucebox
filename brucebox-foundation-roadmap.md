> **Superseded — do not use as active implementation instructions.** This roadmap predates the switch to independently compiled, runtime-loaded native plugin folders and contains obsolete guidance about compiling plugins into the host. The active execution brief is [PLAN.md](PLAN.md); [SPEC.md](SPEC.md) is the normative architecture contract. This file is retained only as historical source material.

# Brucebox Foundation Roadmap

## Purpose

Turn the existing Brucebox repository into a reusable, native Rust
plugin-host framework built around GPUI and GPUI Kit Base.

The immediate goal is **not** to build a polished agent application. It
is to establish the real infrastructure that lets independently authored
Rust plugins contribute native UI and functionality, lets users arrange
that UI freely, and lets the application ship with a useful default
workspace assembled from first-party plugins.

Work through the milestones in order. Implement each milestone to a
usable stopping point, verify it, document important decisions, and then
continue. Do not create disposable test-only plugins or prototype
implementations that are intended to be thrown away and replaced by real
features. Use small, real first-party plugins as the proving ground.
They can be deliberately minimal initially, but their code and
interfaces should remain part of the product.

------------------------------------------------------------------------

## Current repository state

The project has already passed its first native GUI smoke test:

-   Rust toolchain is configured.
-   The application builds and launches on the developer's Omarchy Linux
    desktop.
-   GPUI Kit is used with its unstyled Base layer; the styled GPUI
    Component layer is not intended to be mandatory.
-   A `DockArea` is running with a custom debug renderer.
-   Three temporary demonstration panels can be moved within the dock.

The current demo panels and debug presentation are scaffolding. Replace
the hardcoded panel construction with real plugin contributions as the
architecture develops. Do not spend time polishing the temporary demo
UI.

Before changing APIs, inspect the exact versions pinned in
`Cargo.toml`/`Cargo.lock` and consult the matching GPUI Kit source and
examples. Do not assume APIs from another release. Keep the project
compiling at each meaningful milestone.

# Milestones

## Milestone 1 --- Establish the panel and plugin contribution registry

### Goal

Remove the assumption that `workspace/dock.rs` knows which concrete
panels exist. Introduce a Brucebox-level registry/catalog through which
plugins contribute panels, and make the workspace instantiate panels by
registered identity.

### Work

1.  Inspect the current GPUI Kit Base Dock API, especially panel
    identity, panel factories, restoration registration, and DockArea
    state.
2.  Design stable, namespaced identifiers for plugins, panel types, and
    panel instances. For example:
    -   `brucebox.agent-chat`
    -   `brucebox.git`
    -   `brucebox.agent-chat.conversation`
    -   `brucebox.git.diff-tree`
3.  Define a panel contribution descriptor containing only the metadata
    actually needed by the host, such as:
    -   stable panel type ID;
    -   owning plugin ID;
    -   display title and optional icon;
    -   whether multiple instances are permitted;
    -   factory/creation callback;
    -   state restoration support or a clearly defined later extension
        point.
4.  Implement a registry that supports registration, lookup,
    enumeration, and panel-instance creation. Define duplicate-ID and
    missing-plugin behavior.
5.  Bridge the Brucebox registry to GPUI Base's own panel
    restoration/factory mechanism. Do not confuse the user-facing panel
    catalog with GPUI Base's internal restoration registry; they solve
    related but different problems.
6.  Refactor workspace construction so it requests panels by ID from the
    registry. `dock.rs` must not import concrete plugin panel
    implementations.
7.  Remove the temporary Welcome/Scratch Pad/Activity panels when a real
    first-party contribution is available. Do not preserve demo panels
    as a parallel permanent mechanism.

### Acceptance criteria

-   A plugin module can register a panel without modifying `dock.rs`.
-   The workspace can create and place a panel using its registered ID.
-   Panel type IDs and instance IDs have distinct, documented meanings.
-   Duplicate registrations fail clearly rather than silently
    overwriting.
-   GPUI Base can restore a panel by its stable type identity.
-   Existing docking interactions still work.
-   No mandatory styled component library has been introduced.

### Deliverable

A documented panel contribution API and registry, used by the workspace.

------------------------------------------------------------------------

## Milestone 2 --- Workspace state, persistence, and presets

### Goal

Make workspace arrangements user-owned and persistent. The code-defined
initial layout must be only a first-run preset.

### Work

1.  Inspect the pinned GPUI Kit version's DockArea state
    serialization/restoration and layout-change event APIs.
2.  Define a versioned Brucebox workspace document. It should identify
    the workspace, preserve DockArea state, and provide a place for
    panel-instance/plugin-specific state and future floating surfaces.
3.  Implement save and restore using GPUI Base's supported state APIs.
    Do not invent a second docking tree if GPUI Base already owns the
    live tree.
4.  Persist panel type identity and stable instance identity. Define
    behavior for missing, disabled, or unavailable plugins/panels;
    preserve recoverable placeholders or unresolved layout entries where
    supported rather than silently discarding user layouts.
5.  Add a default workspace preset that references registered panel
    types. The preset should be data/configuration, not permanent
    application structure.
6.  On first launch, create the default preset. On later launches,
    restore the user's saved workspace.
7.  Add an explicit reset-to-default action or command.
8.  Avoid writing to disk on every pointer movement. Coalesce/debounce
    layout persistence and save at appropriate lifecycle points.
9.  Keep the persistence format versioned and plan for migrations.

### Acceptance criteria

-   A user can rearrange panels, close and reopen Brucebox, and recover
    the arrangement.
-   A first-run user receives the default preset.
-   Resetting to the default is explicit and does not happen
    automatically on every startup.
-   Missing plugin contributions do not cause silent loss of the entire
    workspace.
-   Workspace persistence is not coupled to a particular visual theme.

### Deliverable

Versioned workspace persistence and a replaceable default preset.

------------------------------------------------------------------------

## Milestone 3 --- Floating surfaces and complete workspace behavior

### Goal

Extend the workspace beyond a single dock tree while keeping panel
contributions independent of placement.

### Work

1.  Confirm exactly which behaviors GPUI Base DockArea already supports
    in the pinned version. Do not reimplement existing functionality.
2.  Implement a Brucebox-level floating-surface/window model for
    behavior not provided by DockArea.
3.  Support creating floating groups, resizing and moving them, and
    moving panels between floating and docked surfaces.
4.  Support snapping/docking floating surfaces back into the main
    workspace where feasible.
5.  Persist floating-surface geometry and membership in the versioned
    workspace document.
6.  Keep visual presentation replaceable. The workspace engine should
    expose state and interaction hooks; it should not prescribe a fixed
    dock appearance.
7.  Defer multi-monitor edge cases and advanced window-management polish
    until basic behavior is reliable, but avoid an architecture that
    prevents them.

### Acceptance criteria

-   Users can use the main docked workspace and at least one independent
    floating surface.
-   Panels can move between docked and floating arrangements without
    being recreated unnecessarily or losing state.
-   Floating arrangements survive restart.
-   Plugin authors do not need to know whether their panel is docked or
    floating.

### Deliverable

A workspace engine that supports docked and floating panel groups.

------------------------------------------------------------------------

## Milestone 4 --- Integrate Cordis as the composition runtime

### Goal

Introduce Cordis for plugin composition, services, events, dependencies,
and lifecycle without making it responsible for rendering or layout.

### Work

1.  Inspect and pin the selected Cordis Rust version. Confirm its actual
    API and compatibility with the pinned Rust toolchain.
2.  Create a Brucebox runtime adapter around Cordis. Keep
    Cordis-specific types behind the runtime boundary where practical.
3.  Define plugin startup, registration, dependency declaration,
    initialization, error handling, and shutdown conventions.
4.  Establish how plugins publish and consume typed services and events.
5.  Establish ownership and cleanup rules for plugin-created resources,
    subscriptions, tasks, and registrations.
6.  Connect plugin startup to panel registration: a plugin registers its
    contributions during initialization; plugin shutdown removes or
    marks those contributions unavailable.
7.  Keep the panel registry and workspace engine independent of Cordis
    internals. Use an adapter to connect lifecycle events to
    registration changes.
8.  Do not build a bespoke event bus or service locator if Cordis
    already provides the needed behavior.
9.  Add a small real cross-plugin interaction as part of a real
    first-party feature. For example, a Git plugin can publish a commit
    event and another real plugin can react to it. Avoid a throwaway
    test plugin.

### Acceptance criteria

-   Cordis starts and stops with Brucebox.
-   A plugin can register a service, consume another plugin's service,
    and communicate through typed events.
-   Dependencies and missing dependencies have defined behavior.
-   Plugin-owned resources are cleaned up at shutdown.
-   A plugin can contribute UI without Cordis knowing GPUI rendering
    details.
-   A non-UI/background plugin remains possible.

### Deliverable

A functioning Cordis-backed plugin runtime and documented lifecycle
conventions.

------------------------------------------------------------------------

## Milestone 5 --- Create the public Brucebox SDK

### Goal

Make it practical for someone to author a native plugin without editing
Brucebox application source.

### Work

1.  Create a dedicated `brucebox-sdk` crate for public plugin-authoring
    interfaces.
2.  Create a separate internal runtime crate if it materially clarifies
    host/runtime boundaries; do not split crates merely for theoretical
    purity.
3.  Re-export or pin the supported GPUI/GPUI Kit APIs needed by plugin
    authors so plugins compile against the same versions as the host.
4.  Define the plugin entry/registration interface and plugin metadata
    format.
5.  Expose panel contribution and panel factory APIs.
6.  Expose Cordis service/event integration through supported SDK
    abstractions.
7.  Expose host services that plugins genuinely need, adding them
    incrementally rather than designing a speculative capability
    catalog.
8.  Give plugin authors direct access to GPUI primitives and permit
    custom GPUI elements/components. Do not require Brucebox's own
    component library.
9.  Document the supported Rust toolchain, dependency versions, plugin
    interface version, and compatibility policy.
10. Keep the initial model source-compiled. Do not claim that matching
    crate versions create a stable Rust ABI or guarantee compatibility
    for independently compiled dynamic libraries.

### Acceptance criteria

-   A plugin can be authored using the SDK and built as part of the host
    without editing application source.
-   A plugin can bring custom components and its own visual design.
-   The SDK does not force a fixed layout, panel slot, or component
    library.
-   Cordis and workspace internals are not unnecessarily exposed as
    implementation details.
-   Version constraints and compatibility expectations are documented.

### Deliverable

A usable, versioned native plugin SDK.

------------------------------------------------------------------------

## Milestone 6 --- Plugin discovery and build workflow

### Goal

Allow developers to add or remove source plugins by changing the plugin
directory and restarting/building Brucebox, without editing application
source.

### Work

1.  Establish a plugin folder convention, for example:

    ``` text
    plugins/
      builtin/
        agent-chat/
        git/
      third-party/
        my-plugin/
    ```

2.  Give each plugin a manifest containing its ID, name, version, entry
    point/contribution metadata, and dependencies as required.

3.  Implement discovery and validation of plugin manifests.

4.  Implement a build orchestrator (`xtask` or equivalent) that gathers
    enabled plugin crates into the host build. Avoid runtime `cargo`
    invocation during normal GUI startup.

5.  Generate or assemble the plugin registration bundle without
    requiring developers to edit `main.rs`, `app.rs`, or a central
    hand-maintained module list.

6.  Define how plugins declare dependencies on SDK and host versions.

7.  Define enabled/disabled plugin configuration and behavior when a
    plugin fails to compile or initialize.

8.  Keep Cargo lockfile and dependency resolution reproducible. Avoid
    copying large `target` or `node_modules`-style dependency
    directories into plugin folders.

9.  Document the developer workflow: add plugin folder, implement
    against SDK, build/restart host, plugin becomes available.

10. Treat plugins as trusted native code in this initial architecture.
    Do not introduce a sandbox or pretend these plugins are isolated.

### Acceptance criteria

-   Adding a conforming plugin folder makes it discoverable by the build
    workflow.
-   The developer does not edit Brucebox source to register the plugin.
-   Plugins are compiled with the same pinned toolchain and compatible
    dependency graph as the host.
-   Disabled plugins are excluded from the active registration bundle.
-   Build failures identify the responsible plugin clearly.
-   Restarting Brucebox is sufficient to load changed plugin code; no
    in-process hot unload is required.

### Deliverable

A reproducible source-plugin development and build workflow.

------------------------------------------------------------------------

## Milestone 7 --- Build real first-party plugins

### Goal

Replace the temporary demo panels with useful built-in plugins that use
exactly the same SDK and registration path available to third parties.

### Recommended starting plugins

Implement only enough functionality to prove the architecture and create
a useful foundation. Do not spend time polishing the complete product
design yet.

**Agent Chat** - A real conversation panel. - Basic message display and
input. - A minimal host/agent integration seam, even if the agent
backend is initially simple. - State that can later be persisted.

**Git** - A real repository status view. - A basic changed-files/diff
tree panel. - A basic diff display or selection flow. - A typed
event/service seam for future commit actions and other plugin
integrations.

These are real product features, not disposable test fixtures. Keep
their implementation and incrementally improve them.

Additional first-party plugins can follow, such as Terminal and Agent
Activity, once the core SDK and lifecycle are stable.

### Acceptance criteria

-   First-party plugins register through the same SDK path as
    third-party plugins.
-   Their panels can be opened, closed, rearranged, and restored.
-   At least one meaningful cross-plugin interaction uses Cordis
    services or events.
-   Plugin UI is free to use custom GPUI components and drawing.
-   No special first-party-only panel path is required.

### Deliverable

A small but real first-party plugin set that exercises the framework.

------------------------------------------------------------------------

## Milestone 8 --- Assemble the default Brucebox workspace

### Goal

Make the initial Brucebox experience a preset assembled from built-in
plugins, not a hardcoded application UI.

### Work

1.  Define the default workspace preset using registered first-party
    panel IDs.
2.  Choose a practical initial arrangement for Agent Chat, Git, and any
    other implemented plugins.
3.  Make the preset replaceable and resettable.
4.  Ensure users can remove every default panel and create a completely
    different workspace.
5.  Add a minimal panel/plugin catalog UI for discovering and opening
    registered contributions.
6.  Make sure the catalog itself is not a permanent workspace slot. It
    can be opened as a panel, overlay, or command-driven surface.
7.  Keep workspace visuals and plugin visuals independently
    customizable.

### Acceptance criteria

-   A new installation starts with a useful default workspace.
-   Users can rearrange it arbitrarily, remove built-in panels, and open
    other plugin panels.
-   The default preset is not reapplied over an existing saved
    workspace.
-   First-party and third-party contributions are presented
    consistently.

### Deliverable

A usable default Brucebox workspace assembled from plugins.

------------------------------------------------------------------------

# Implementation order and working rules

Work through Milestones 1--8 sequentially. The milestones are
architectural boundaries, not a reason to stop after every tiny task for
approval. Make sensible implementation decisions within the stated
constraints.

For each milestone:

1.  Inspect the current repository and pinned dependency source before
    designing against an API.
2.  Review the existing implementation; preserve working code where
    appropriate.
3.  Write down important architectural decisions briefly (ADRs or
    concise design notes when a decision has long-term consequences).
4.  Implement the smallest complete version that meets the milestone's
    acceptance criteria.
5.  Use real first-party product functionality as the integration
    example. Do not create disposable test-only plugins.
6.  Add focused unit/integration tests for registries, persistence,
    manifest validation, lifecycle, and other logic where useful. Tests
    are welcome; throwaway test products are not.
7.  Run formatting, compilation, tests, and Clippy as appropriate.
8.  Update documentation and mark completed work.
9.  Avoid broad speculative abstractions. Add interfaces when a real use
    case requires them.
10. Do not move to a later milestone by bypassing an unresolved
    architectural requirement from an earlier one.

## Technical constraints

-   Native Rust application.
-   GPUI and GPUI Kit Base; do not require GPUI Component.
-   Cordis for composition/lifecycle; do not reinvent its core features.
-   Rust source plugins compiled into the host for the initial
    implementation.
-   Restart/rebuild is an acceptable plugin update workflow; no hot
    swapping or unloading machine code is required.
-   No webview-based plugin UI.
-   No plugin sandbox or fine-grained security permission framework.
-   Plugin developers may implement their own GPUI components and visual
    systems.
-   The workspace must remain free-form; no mandatory fixed panel slots.
-   Keep plugin functionality/contracts as independent from the
    rendering layer as practical.
-   Pin and document the toolchain and key framework versions.
-   Do not add optional dependencies or architecture solely for
    hypothetical future requirements.

## Out of scope for this foundation pass

Unless a dependency is discovered that makes one necessary, defer:

-   Independently compiled dynamic-library plugins and Rust ABI design.
-   In-process hot reload/unload.
-   Sandboxing and hostile-code isolation.
-   A polished production theme or comprehensive component library.
-   A complete agent orchestration product.
-   A fully featured terminal emulator.
-   Advanced multi-monitor floating-window behavior.
-   Marketplace, remote plugin installation, or plugin signing.
-   Exhaustive plugin permissions and capability policy.

## Definition of success

Brucebox is ready for ongoing plugin and workspace development when:

-   A developer can add a Rust plugin under the plugin directory without
    editing host source.
-   The build workflow discovers and compiles it with the pinned
    host/SDK toolchain.
-   The plugin can contribute one or more fully custom native GPUI
    panels.
-   The plugin can communicate with other plugins through Cordis
    services/events.
-   The user can place those panels anywhere in a free-form workspace.
-   The workspace can be saved, restored, and reset to a default preset.
-   First-party plugins use the exact same contribution path as
    third-party plugins.
-   The default Brucebox experience is composed from plugins rather than
    hardcoded UI.
