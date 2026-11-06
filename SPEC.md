# Brucebox — Native Composable Workspace

**Project specification and implementation direction**

> This document describes the intended architecture and implementation direction. Where the current code is only scaffolding, this specification is normative. Do not preserve an earlier implementation approach merely because it appears in the repository.

## 1. Product vision

Brucebox is a native desktop application and open, composable workspace framework.

It is not a conventional application with a fixed interface that plugins extend through predefined slots. It is a free-form workspace in which independently developed plugins contribute functionality and interfaces that users can arrange however they choose.

The application provides native rendering, plugin runtime, composition and workspace infrastructure. Plugins provide functionality and their own UI. Users decide how everything is composed.

The initial application should launch with a useful default workspace, but that is only a preset. Users must be able to dismantle it, rearrange it, replace its panels and build entirely different arrangements.

### Fundamental principles

1. **The workspace is user-owned.** No mandatory sidebars, content regions, toolbars or application-specific panel locations.
2. **Plugins own their interfaces.** Plugin developers can create completely custom GPUI components and visual systems.
3. **Plugins are independently built and runtime-loaded.** Adding or updating a plugin must not require rebuilding Brucebox.
4. **Plugins are composable.** They can provide services and events that other plugins consume.
5. **The host owns composition infrastructure.** Brucebox manages plugin discovery, compatibility, loading, lifecycle, communication and workspace integration.
6. **Layout is data.** The workspace can be rearranged, saved, restored, serialized and represented independently of its rendered GPUI views.
7. **Native first.** The application and its plugins are native Rust/GPUI code, not web applications embedded in desktop windows.
8. **Avoid reinventing existing infrastructure.** Use GPUI Kit for rendering and the baseline docking system, and Cordis for composition. Use Cargo to build each plugin independently, not to compile plugins into the host.

## 2. Technology foundation

| Layer | Technology | Responsibility |
|---|---|---|
| Language | Rust | Host and native plugin development |
| UI framework | GPUI | Native desktop rendering |
| UI infrastructure | GPUI Kit | Components and baseline docking |
| Composition runtime | Cordis-RS | Plugin lifecycle, services, dependencies and events |
| Plugin SDK | Brucebox SDK | Public plugin development interface |
| Plugin format | Versioned plugin folder | Manifest, independently built native library and assets |
| Dynamic loader | Native library loading (libloading or equivalent) | Discover and load plugin libraries at runtime |
| Build system | Cargo | Host and each plugin are built independently |
| Developer tooling | Optional xtask / CLI | Scaffold, build, validate and package individual plugins |
| Persistence | Serde + JSON initially | Workspace and plugin configuration |

### Dependency and compatibility baseline

Use the following as the initial pinned baseline unless implementation work verifies a necessary change:

- gpui-kit = "=0.9.0"
- cordis-rs = "=0.11.1"
- Rust 2024 Edition.
- Rust toolchain pinned to an exact version compatible with GPUI Kit, Cordis and the plugin SDK.

GPUI Kit 0.9.0 bundles a matching GPUI snapshot (gpui-pre 0.3.11). Use its facade instead of independently adding a different GPUI version.

Do not separately introduce another GPUI, gpui-component or gpui-base version.

Cordis v3 uses Rust 1.88 or newer and exposes its application runtime through the cordis Rust import.

The host and plugin SDK must publish explicit compatibility versions. A plugin must declare the plugin API/SDK compatibility it was built against, and the host must reject incompatible plugins with a useful diagnostic rather than attempting to load them.

**Important native-plugin constraint:** Rust does not promise a stable general-purpose ABI for arbitrary Rust types, trait objects or GPUI entities across independently compiled dynamic libraries. Do not pretend that a C entrypoint, libloading, or matching crate version automatically solves this. Before the plugin API expands, prove that a separately compiled plugin can safely register and render a real GPUI panel, participate in Cordis composition and shut down correctly. Keep all ABI-sensitive code behind one narrow loader/SDK boundary. Use an explicitly versioned and pinned compatibility contract; do not claim compatibility across arbitrary Rust toolchains or SDK versions. If the chosen boundary cannot safely support native GPUI contributions, stop and resolve that architecture before building more plugins.

## 3. High-level architecture

Brucebox has five distinct architectural layers.

### A. Application host

The main executable.

Responsible for:

- Initializing the GPUI application and main window.
- Starting the host-owned Cordis runtime.
- Initializing the workspace engine.
- Discovering plugin folders from configured plugin roots.
- Reading and validating plugin manifests.
- Loading enabled, compatible native plugin libraries.
- Passing host services and registration facilities to plugins.
- Coordinating application startup and shutdown.

The host should remain relatively small. It should not own individual plugin functionality or dictate the workspace's visual structure.

The host does **not** compile plugins, generate a plugin dependency bundle, or link each plugin's source crate into the application.

### B. Plugin composition — Cordis

Cordis owns application composition.

It manages:

- Plugin registration and lifecycle.
- Dependencies between plugins.
- Typed services.
- Typed events.
- Resource ownership and cleanup.
- Plugin activation and deactivation.

The host owns one application composition runtime. Plugins contribute to that runtime through the plugin SDK/adapter; do not accidentally create disconnected per-plugin Cordis runtimes that cannot share services or events.

Plugins should be able to publish functionality without Brucebox explicitly knowing about it.

Avoid building a second plugin runtime, dependency injection system or event bus.

### C. UI framework — GPUI Kit

GPUI Kit provides:

- GPUI rendering.
- Native application and window infrastructure.
- UI components and primitives.
- Docking and tab infrastructure.
- Layout behavior and persistence.

Use GPUI Kit Base's renderer-independent DockArea as the initial docked-workspace engine. Use its structural docking, panel, tab, split, drag/drop, and state APIs, with a Brucebox-owned renderer/presentation layer.

Do not require the styled GPUI Component layer. Plugins may use GPUI Kit Base, GPUI primitives, or their own GPUI component systems; no shared visual system is mandatory.

### D. Workspace engine — Brucebox

A dedicated workspace layer that connects plugin-contributed panels to GPUI Kit's docking infrastructure.

Responsibilities:

- Maintain the available panel catalog.
- Manage opened panel instances.
- Manage docked and floating surfaces.
- Handle workspace composition and movement between surfaces.
- Persist and restore user arrangements.
- Manage workspace presets.
- Coordinate drag-and-drop operations across workspace surfaces.

This must remain independent of individual plugin implementations.

Build on GPUI Kit's dock model instead of rewriting nested splits, tab groups and docking behavior.

### E. Plugin SDK

The common API used by first-party and third-party plugin developers.

It provides:

- GPUI Kit and GPUI re-exports.
- The supported plugin entry point and compatibility contract.
- Cordis integration.
- Plugin registration interfaces.
- Panel registration and workspace integration.
- Access to host-provided services.
- Common plugin metadata and configuration interfaces.

The SDK is an integration layer, not a visual design system that plugin developers must follow. It must be consumable by a plugin project built independently of the Brucebox application workspace.

---

## 4. Workspace architecture

This is Brucebox's defining feature and should receive particular architectural attention.

### 4.1 No predefined application regions

Brucebox must not require plugins to register in locations such as:

- Sidebar.
- Main content.
- Bottom toolbar.
- Right inspector.
- Navigation area.

There are no mandatory UI slots.

Instead, plugins register independent panel types. The user decides where instances of those panels appear.

### 4.2 Panel types and instances

Distinguish between a panel's definition and an opened instance.

For example, a Git plugin might register:

- Repository Explorer.
- Changed Files.
- Diff Viewer.
- Branch Viewer.
- Commit History.

Users might open several separate Diff Viewer panels.

Each opened instance must have its own identity and state, even though all instances originate from the same plugin and panel type.

Panel metadata should include:

- Stable, globally unique panel type ID.
- Owning plugin ID.
- Display name.
- Optional icon.
- Panel factory.
- State serialization and restoration interface.
- Support for multiple simultaneous instances.

A plugin can contribute multiple panel types or no UI panels at all.

### 4.3 GPUI Kit as the baseline docking engine

Use GPUI Kit Base's gpui_kit::base::dock implementation for the initial docked workspace. Do not make gpui-component or its styled Dock presentation a requirement.

It already provides:

- Nested horizontal and vertical splits.
- Draggable tab groups.
- Resizable split regions.
- Panel activation and movement.
- Drag-and-drop docking.
- Docking previews and drop targets.
- Left, right and bottom dock infrastructure.
- Layout serialization and restoration.

The initial workspace should use one central DockArea as the primary workspace surface.

Do not automatically reserve the left, right or bottom dock areas as permanent application regions. Additional docking regions can be introduced when the user creates them or when their use is justified.

Keep the renderer replaceable. Do not spend the first milestones perfecting a custom docking renderer before the actual workspace and plugin integration work.

Keep workspace integration behind a Brucebox abstraction so the underlying GPUI Kit implementation can be changed or customized later.

### 4.4 Floating surfaces

GPUI Kit's DockArea does not provide Brucebox's desired general-purpose floating-surface model.

Brucebox must provide a workspace-level floating layer around it.

A floating surface is a panel group that:

- Can be positioned anywhere inside the Brucebox workspace.
- Can be resized independently.
- Can overlap other floating surfaces.
- Has an independent stacking order.
- Can be moved back into the docked workspace.
- Can accept other panels as tabs or split regions.

Use GPUI and GPUI Kit components to implement this. This does not require creating another GPUI application or native WebView.

A useful approach is to let a floating surface contain its own DockArea. That allows panels inside a floating group to use the same nested splits and tabs as the main workspace.

The workspace controller should coordinate moving panel instances between the main docked surface and floating surfaces. Preserve panel identity and state when moving panels. Do not unnecessarily destroy and recreate an active plugin interface during a workspace rearrangement.

### 4.5 Dragging, snapping and resizing

The workspace should support:

**Docked panels**
- Drag tabs between tab groups.
- Drop panels beside other panels.
- Split horizontally or vertically.
- Merge panels into tab groups.
- Resize split regions.
- Reorder tabs.

**Floating panels**
- Move freely within the application workspace.
- Resize from edges and corners.
- Change stacking order when focused.
- Snap to nearby workspace edges.
- Drop onto existing surfaces to tabify or split.

**Cross-surface operations**
- Undock a docked panel.
- Move a floating panel into the docked workspace.
- Move panels between floating surfaces.
- Combine floating surfaces into tab groups where appropriate.

Implement a workspace-level drag coordinator for operations that cross between separate DockArea instances. Do not assume GPUI Kit's existing DockArea automatically handles cross-area transfers.

Start with basic floating, resizing and docking back into the central surface. More sophisticated snapping and multi-surface tabbing can follow once the integration is validated.

### 4.6 Workspace data model

Workspace layout must be represented as serializable data rather than a hardcoded arrangement of GPUI entities.

Use GPUI Kit's DockAreaState for the internal state of docked surfaces.

Brucebox should wrap this in its own versioned workspace document containing, conceptually:

- Workspace schema version.
- Workspace identifier.
- Main DockArea state.
- Collection of floating surfaces.
- Floating surface bounds and stacking order.
- DockArea state for each floating group.
- Panel instance metadata and plugin/panel identifiers.

Do not duplicate GPUI Kit's entire PaneTree implementation in Brucebox.

The Brucebox workspace document should compose the underlying DockArea state with Brucebox-specific surface information.

Panel state should be owned by the panel implementation and serialized through its restoration interface. Persist enough identity and opaque state to recover panels even when their plugin is unavailable.

### 4.7 Presets and persistence

A workspace preset is simply a saved WorkspaceDocument.

Brucebox should support:

- Default workspace.
- User-created presets.
- Saving the current arrangement.
- Restoring a previous arrangement.
- Resetting to a default preset.
- Switching between presets.

For the initial implementation, one default preset and automatic persistence of the current workspace are sufficient.

Never treat the default workspace as mandatory structure.

If a plugin is unavailable or incompatible during restoration, display a recoverable missing-panel indicator and preserve its saved state where possible. Installing or enabling the plugin later should make recovery possible without recreating the workspace.

---

## 5. Plugin architecture

### 5.1 Independent plugin packages and runtime loading

**Plugins are not compiled into the Brucebox executable.**

Each plugin is its own independently built native Rust project. Its build produces a platform-specific native library and a plugin folder containing that library, its manifest and any assets or supporting files it needs.

At runtime, Brucebox discovers plugin folders, validates their manifests and compatibility, loads the native library, and asks it to register its contributions with the host.

Adding, removing, updating or replacing a plugin folder must not require rebuilding Brucebox. A restart to load a changed native library is acceptable for the initial implementation.

The host must not:

- Add plugin crates as Cargo workspace members.
- Generate a Cargo dependency list containing installed plugins.
- Generate a Rust source registry that statically links plugins into the host.
- Invoke Cargo or compile plugins during normal application startup.
- Require editing Brucebox source code to add a plugin.

Cargo remains the way each plugin is built, but each plugin build is independent from the host build.

Do not implement live machine-code replacement or hot-unloading as a requirement. For the first version, discover and load enabled plugins during application startup and keep their native libraries loaded for the lifetime of the process. Disabling, replacing or updating a plugin takes effect after restart. Cordis/plugin deactivation and orderly resource cleanup must still be implemented during shutdown.

### 5.2 Plugin folder format

A plugin is an installable folder. The exact manifest schema and binary layout can be finalized during the loader milestone, but it should follow this general shape:

~~~text
brucebox-agent-chat/
├── plugin.toml
├── bin/
│   ├── linux-x86_64/
│   │   └── libbrucebox_agent_chat.so
│   ├── windows-x86_64/
│   │   └── brucebox_agent_chat.dll
│   └── macos-aarch64/
│       └── libbrucebox_agent_chat.dylib
└── assets/
    └── ...
~~~

A plugin folder must have:

- A unique, stable plugin ID.
- Human-readable name and description.
- Plugin version.
- Plugin API/SDK compatibility declaration.
- The native library path for the current platform and architecture.
- Optional asset/resource paths.
- Optional plugin dependency declarations.

The folder may also contain source or development files, but the installed runtime must not need the Rust source tree or a Cargo manifest to load the plugin. A packaged plugin should contain what is needed to run, not a source checkout or a Cargo build target directory.

The host should support configured plugin roots, including a development plugin directory and a per-user installed plugin directory. Bundled first-party plugins must use the same folder and loader format as third-party plugins; they are not statically linked exceptions.

Use a versioned manifest format. Validate paths and metadata before loading a library. Report malformed manifests, missing platform binaries, duplicate IDs, incompatible SDK/API versions, missing dependencies and loader failures in a way the user can understand.

An illustrative manifest (not a commitment to these exact field names) is:

~~~toml
[plugin]
id = "brucebox.agent-chat"
name = "Agent Chat"
version = "0.1.0"
api_version = 1
sdk_version = "0.1"

[library]
linux-x86_64 = "bin/linux-x86_64/libbrucebox_agent_chat.so"
windows-x86_64 = "bin/windows-x86_64/brucebox_agent_chat.dll"
macos-aarch64 = "bin/macos-aarch64/libbrucebox_agent_chat.dylib"
~~~

Keep the runtime package format simple. Do not turn this into a marketplace, package manager or elaborate distribution system in the initial foundation.

### 5.3 Plugin loader and lifecycle

The host plugin manager is responsible for:

1. Finding configured plugin roots.
2. Enumerating plugin folders without scanning arbitrary source trees.
3. Parsing and validating each manifest.
4. Resolving the binary for the current OS and architecture.
5. Checking plugin ID uniqueness, API compatibility and declared dependencies.
6. Loading enabled plugins through one isolated loader module.
7. Resolving the versioned plugin entry point.
8. Providing the supported host/SDK context.
9. Registering plugin contributions with the workspace and Cordis runtime.
10. Retaining the loaded library and all plugin-owned function pointers/objects for as long as they may be used.
11. Coordinating deactivation, cleanup and shutdown in a safe order.

The plugin manager is infrastructure, not a second composition runtime. Cordis remains responsible for plugin composition and lifecycle semantics; the loader handles native library discovery/loading and the boundary into the host.

Keep the loader implementation contained in a small module/crate. Do not spread raw library handles, unsafe symbol lookup or ABI-specific details across workspace code.

**Library lifetime is critical.** Never unload a library while a panel factory, panel instance, event callback, service vtable, task, destructor or other object originating from that library may still execute. The first implementation should retain loaded libraries until the host has shut down and all plugin-owned resources have been disposed.

A plugin load failure must not crash the entire workspace. Record the failure, continue loading other compatible plugins, and preserve missing-plugin placeholders for saved workspace panels.

### 5.4 Plugin ABI and GPUI integration

This is an explicit feasibility gate, not a detail to hand-wave away.

A separately compiled Rust dynamic library can be loaded by the host, but arbitrary Rust types do not have a stable cross-version ABI. GPUI panels are not plain C data: they involve GPUI types, trait implementations, entities, callbacks and lifetimes. Cordis contributions also need to join the host's composition runtime rather than create isolated state.

The loader milestone must validate the real requirements together:

- A plugin is compiled independently of the host.
- The host discovers and loads its plugin folder at runtime.
- It registers a real GPUI panel.
- The panel renders and handles interactions in the host's existing GPUI application.
- The plugin can register Cordis functionality against the host-owned composition runtime.
- The host safely retains and disposes plugin-owned objects and the library handle.
- An incompatible plugin is rejected before unsafe registration is attempted.

Use a narrow, versioned boundary and pin the compatible Rust toolchain, target, GPUI Kit/GPUI, Cordis and SDK versions. Establish explicit compatibility checks. Do not promise that arbitrary versions of separately compiled Rust plugins will work together.

Do not choose an interface that only supports metadata or serialized UI descriptions if that would prevent plugins from implementing arbitrary native GPUI components. Full native GPUI freedom is a requirement. If an initially attractive ABI approach cannot safely expose the required GPUI and Cordis functionality, investigate and resolve that before expanding the SDK.

Keep unsafe code limited to the loader/boundary and document its invariants. Do not claim the plugin system is a security sandbox: native plugins are trusted in-process code with the privileges of Brucebox.

**Measured result (step 5).** The gate was run, and it constrains the design in one specific way. Two builds with the same pinned toolchain and the same `gpui-kit`/`cordis-rs` versions but different dependency graphs produce **different `TypeId`s for the same type** in one process. GPUI delivers an event by downcasting it through `Any`, so a plugin's own `on_key_down`/`on_click` listeners never run, and a plugin cannot use a host-defined global or `EventEmitter` event type. Everything that is a vtable call or plain data — `Arc<dyn PanelView>`, plugin-owned entities, structs, enums, the rendering pipeline the plugin drives — crosses fine.

The resolution is explicit rather than silent: the host owns the input handlers (host compilation, where dispatch works) and forwards key and pointer events to a plugin panel as plain data through `PluginPanelInput`; plugin layout and painting stay unrestricted. The contract this depends on is narrow and checked — same toolchain, same pinned GPUI Kit and Cordis, same plugin API and SDK version — and a package declaring anything else is rejected before its code is called. See [`docs/adr/0005-native-plugin-boundary.md`](docs/adr/0005-native-plugin-boundary.md).

### 5.5 Building, installing and updating plugins

Plugin development is separate from host development.

A plugin developer should be able to:

1. Create or clone a standalone plugin project.
2. Add the Brucebox SDK as a dependency.
3. Implement the plugin entry point and contributions.
4. Build the plugin for a supported target.
5. Package the manifest, native library and assets into one plugin folder.
6. Copy that folder into a configured plugin root.
7. Restart Brucebox and see the plugin appear.

No Brucebox application source changes or host rebuild should be required.

An optional Brucebox CLI or xtask may help scaffold, build, validate, package or install an individual plugin. It must invoke that plugin's own build; it must not generate a host Cargo workspace containing every installed plugin. The normal desktop application's startup path must never invoke Cargo.

A plugin update replaces its installed folder/artifact. The host should not overwrite or remove a library while it is loaded. It is sufficient to stage the replacement and activate it on next launch.

### 5.6 Plugin registration and contributions

The SDK should provide a unified developer-facing registration model, with separate responsibilities internally.

A plugin can register:

**UI contributions**
- Panel types.
- Panel factories.
- Commands.
- Optional workspace presets.

**Runtime contributions**
- Cordis plugins/contributions.
- Services.
- Event listeners and publishers.
- Background tasks.
- Other application capabilities.

Do not require every plugin to contribute a panel.

Do not require every plugin to provide runtime services.

Plugins may be entirely visual, entirely functional, or a combination.

### 5.7 Plugin-owned UI

The SDK must expose the native GPUI API.

A plugin developer should be able to implement:

- Custom Render and RenderOnce views.
- Custom elements.
- Custom drawing and painting.
- Custom animations.
- Custom component libraries.
- Custom focus and interaction behavior.
- Custom internal panel layouts.

Using GPUI Kit's styled components should be optional.

A plugin must never be forced to use Brucebox-provided buttons, cards, menus, themes or design tokens.

Brucebox controls the placement and management of the panel, not its internal visual design.

### 5.8 Plugin communication

Cordis handles plugin-to-plugin communication through the host-owned composition runtime.

Plugins can:

- Publish typed events.
- Subscribe to typed events.
- Publish typed services.
- Consume services published by other plugins.
- Declare service dependencies.
- Respond to changes in service availability.

Plugin-specific contract crates should be independent of plugin implementations and versioned like other plugin dependencies.

For example, the Git plugin can publish repository state and change events, while Agent Chat can consume the Git plugin's repository-context service to attach selected repository information to a conversation. The host does not need to understand the semantics of those plugin-specific contracts.

Plugin developers should be able to publish their own contracts, and other plugins can depend on them. Do not create a central list of every capability that Brucebox developers must anticipate.

### 5.9 SDK responsibility

The SDK provides the common integration interface, not a predefined capability catalog.

It should expose host functionality such as:

- Plugin entry point and compatibility metadata.
- Workspace panel registration.
- Command registration.
- Access to shared application context.
- Cordis runtime integration.
- Basic notification and UI integration.
- Plugin metadata and configuration interfaces.

Everything else should be extensible through Cordis and plugin-owned crates.

The SDK should be versioned and released independently enough that a plugin project can depend on it without becoming a member of the Brucebox host Cargo workspace. Keep GPUI Kit, GPUI and Cordis compatibility explicit.

---

## 6. GPUI and Cordis runtime integration

GPUI's UI state and window context must remain on the GPUI application thread.

Cordis should manage application lifecycle and asynchronous plugin functionality without directly manipulating GPUI view state from arbitrary background tasks.

Use a clear bridge:

- GPUI user interactions can initiate application or Cordis operations.
- Cordis plugins can publish events and perform asynchronous tasks.
- Plugins can send typed messages to the GPUI host through the supported SDK interface.
- The GPUI host applies UI changes using its own application context.

Do not introduce IPC, JSON serialization or separate processes between plugins as a workaround for an in-process native plugin boundary. Plugin panels must be real GPUI contributions in the host window.

The host must maintain explicit ownership of Cordis lifecycle handles and plugin library handles. During shutdown:

1. Stop accepting new plugin work.
2. Deactivate plugins and stop their tasks/subscriptions according to Cordis's actual lifecycle semantics.
3. Dispose plugin-owned workspace panels, factories, callbacks and services.
4. Release remaining plugin-owned objects.
5. Release native library handles only after nothing can execute code from them.
6. Finish GPUI and host shutdown.

Follow Cordis's actual lifecycle and teardown semantics instead of assuming dropping a handle automatically stops a plugin.

**Implemented bridge (step 4).** The runtime runs on its own worker threads and the two sides exchange plain-data messages: a plugin *declares* its panel contributions (ids, titles, multiplicity) and the application thread joins that declaration with a panel factory it holds, because a factory builds GPUI entities and cannot move off the application thread. Typed events, service availability and lifecycle phases travel the same way, and the application thread applies them to the panel catalog and to a host status view. Services are lifecycle prerequisites: a plugin whose required service is missing stays `Pending` and never applies. The host keeps every lifecycle handle and disposes them newest-first at quit, attempt-all. See [`docs/adr/0004-cordis-composition-runtime.md`](docs/adr/0004-cordis-composition-runtime.md).

---

## 7. Initial project structure

Create Brucebox as a standalone Rust project.

Do not migrate Iceplane application code or reuse its previous application structure.

Suggested host repository layout:

~~~text
brucebox/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── README.md
│
├── apps/
│   └── brucebox-desktop/
│       ├── Cargo.toml
│       └── src/
│
├── crates/
│   ├── brucebox-sdk/
│   ├── brucebox-plugin-api/
│   ├── brucebox-plugin-loader/
│   ├── brucebox-runtime/
│   └── brucebox-workspace/
│
├── plugins/
│   ├── agent-chat/
│   └── git/
│
├── tools/
│   └── xtask/
│
├── assets/
│
└── scripts/
    └── dev.sh
~~~

The repository's plugins/ directory is for independently built first-party plugin projects during development. These plugin projects are **not** Cargo members/dependencies of the host and are not compiled by a host workspace build. Their output is packaged into plugin folders and discovered by the same runtime loader used for third-party plugins.

An independently maintained third-party plugin may live in any source repository or directory. The host does not assume all plugins are checked out below the Brucebox repository.

### Package responsibilities

**brucebox-desktop**
- Executable.
- GPUI initialization.
- Main window.
- Startup and shutdown.
- Plugin manager startup.
- Workspace initialization.

**brucebox-sdk**
- Public plugin developer interface.
- GPUI Kit re-exports.
- Supported plugin registration interfaces.
- Cordis integration interfaces.
- Panel and command contribution contracts.
- Plugin metadata and compatibility helpers.

**brucebox-plugin-api**
- Versioned host/plugin boundary.
- ABI-sensitive types and entry point definitions.
- Compatibility/version negotiation.
- Minimal shared interface required by independently built plugins.

Keep this narrow. Do not allow low-level ABI details to leak into unrelated host crates.

**brucebox-plugin-loader**
- Plugin root discovery.
- Manifest parsing and validation.
- Platform/architecture library resolution.
- Native library loading.
- Entry-point resolution.
- Loaded library lifetime ownership.
- Load diagnostics and compatibility rejection.

**brucebox-runtime**
- Host-owned Cordis initialization.
- Plugin lifecycle coordination.
- Shared services and event integration.
- Application runtime coordination.

**brucebox-workspace**
- Panel catalog.
- Panel instance management.
- GPUI Kit DockArea integration.
- Floating surface model.
- Cross-surface drag coordination.
- Workspace persistence.
- Preset loading and restoration.

**xtask**
- Optional plugin scaffolding, build, package, validation and local installation commands.
- Must operate on one standalone plugin project at a time.
- Must not generate a static registry or merge plugin crates into the host build.

Keep packages small. Avoid adding abstractions until an actual integration requirement justifies them.

---

## 8. First-party plugins: Agent Chat and Git

Use real first-party plugins as the proving ground for the plugin architecture. **Do not create disposable test-only plugins, mock plugins or demo plugins whose only purpose is to be replaced later.**

The first versions can be intentionally small and visually/basic-functionality focused. They must still be the beginning of the actual features Brucebox intends to provide.

Both plugins must be built independently, packaged as separate plugin folders, and loaded by the normal runtime plugin loader. They must not be statically linked into Brucebox.

### Agent Chat

Start with a minimal but genuine conversation experience:

- A native GPUI conversation panel.
- Message history and a composer/input.
- Basic send/receive flow through the chat/session integration available to the app.
- Plugin-owned UI and state.
- A clear path to adding agent/provider/ACP functionality without making that infrastructure a requirement for validating the workspace and plugin runtime.

Do not spend this foundation pass building a complete agent orchestrator or elaborate chat UX. The point is to establish a real Agent Chat feature through the independent plugin API.

### Git

Start with a useful, real Git workspace contribution:

- Repository selection/opening at a basic level.
- Current branch and repository status.
- Changed-files list.
- Basic diff inspection.
- Plugin-owned UI and state.

Full Git client functionality is outside this initial foundation pass.

### Real cross-plugin integration

Use the two real plugins to validate Cordis communication.

For example:

- Git publishes a repository/context service and typed repository-change events.
- Agent Chat can consume the Git service to attach selected repository or diff context to a conversation.
- The plugins communicate through their own versioned contract, not direct imports of each other's implementation.
- Moving either panel around the workspace must not affect the integration.

This is the proof that independently loaded plugins can contribute native UI, publish services/events and cooperate without host-specific coupling.

---

## 9. Initial implementation plan

Implement these stages in order. The current native GPUI/DockArea smoke test is already the bootstrap; verify it before continuing. Each stage should produce a usable result, be checked, and remain part of the product. Use real Agent Chat and Git features as plugin examples; do not create disposable test-only plugins.

### Stage 0 — Verify the existing native foundation

1. Re-check the pinned Rust toolchain, Cargo.lock and GPUI Kit 0.9.0 APIs.
2. Confirm the current native window and GPUI Kit Base DockArea still build and launch.
3. Keep the custom DockArea renderer replaceable; do not spend time polishing the temporary debug presentation.
4. Inspect the current GPUI Kit Base Dock source/examples before changing panel, restore or drag APIs.

**Exit condition:** The native GUI/DockArea bootstrap remains reliable and the dependency baseline is understood.

### Stage 1 — Panel catalog and workspace decoupling

1. Introduce stable namespaced plugin IDs, panel type IDs and unique panel instance IDs.
2. Define panel contribution metadata: owner, title/icon, multiplicity, factory and state create/restore hooks.
3. Implement the Brucebox panel catalog: register, lookup, enumerate and instantiate; define duplicate-ID and unavailable-plugin behavior.
4. Bridge the catalog to GPUI Kit Base's internal panel factory/restoration mechanism. Keep these registries conceptually distinct.
5. Refactor workspace construction to resolve panel IDs through the catalog. Workspace/DockArea code must not import concrete plugin implementations.
6. Replace temporary Welcome/Scratch/Activity panels as real first-party contributions become available.

**Exit condition:** A contribution can register a panel without editing workspace construction; IDs and instances are distinct; docking still works.

### Stage 2 — Workspace persistence and presets

1. Inspect the pinned DockArea state, change notification and restoration APIs.
2. Define a versioned WorkspaceDocument containing workspace identity/schema, DockArea state, panel type/instance/plugin IDs, and opaque panel state.
3. Save/restore using GPUI Kit Base's supported state APIs; do not duplicate the Dock tree.
4. Apply the default preset only on first launch or explicit reset. Restore the user's saved state on later launches.
5. Coalesce/debounce persistence; avoid writing on every pointer movement.
6. Preserve unresolved panel records/state when a plugin is missing, disabled or incompatible.

**Exit condition:** Layout and panel state survive restart; unavailable plugins do not erase user arrangements; reset-to-default is explicit.

### Stage 3 — Floating surfaces

1. Add a workspace-level floating group model with bounds, z-order/focus and an inner DockArea where useful.
2. Implement create, move, resize, detach, redock, basic drop previews and cross-surface transfer.
3. Preserve panel identity/state and avoid unnecessary destruction/recreation.
4. Persist floating geometry and group membership in WorkspaceDocument.
5. Start with in-window floating groups; defer advanced snapping, multi-monitor and animation polish.

**Exit condition:** Panels move between docked and floating groups, remain usable, and survive restart.

### Stage 4 — Cordis composition runtime

1. Add/pin cordis-rs 0.11.1 only after verifying its actual API and compatibility with the pinned toolchain.
2. Create a thin Brucebox runtime adapter around one host-owned Cordis runtime.
3. Define plugin lifecycle, dependency ordering/availability, typed services/events, async work and cleanup conventions.
4. Connect plugin registration/unregistration to the panel catalog without coupling workspace internals to Cordis types.
5. Bridge async plugin operations/events back to the GPUI application thread safely.
6. Use Cordis for composition/lifecycle; do not implement a second event bus, DI container or service locator.

**Exit condition:** The runtime starts/stops correctly; contributions can provide/consume typed services and events; dependencies and resource cleanup are explicit; Cordis remains UI-agnostic.

### Stage 5 — SDK and native ABI proof

1. Create a narrow versioned brucebox-plugin-api boundary and a developer-facing brucebox-sdk.
2. Expose the supported GPUI/GPUI Kit Base APIs, panel/command contributions, host services and Cordis integration. Do not require a shared visual component library.
3. Pin and document the supported Rust toolchain, target, GPUI Kit/GPUI, Cordis, SDK and plugin API compatibility set.
4. Isolate unsafe ABI/symbol work. Define entrypoint, host context, version negotiation, registration and teardown.
5. Build Agent Chat as a standalone plugin project, not a host Cargo workspace member.
6. Prove independent build + runtime load + real GPUI panel rendering/interaction + registration into the host-owned Cordis runtime + safe shutdown/library lifetime.
7. Resolve any ABI/GPUI constraint before expanding the API. Matching crate versions alone are not a stable Rust ABI guarantee.

**Exit condition:** An independently compiled Agent Chat plugin can safely contribute native GPUI UI and runtime functionality without rebuilding the host; incompatible builds are rejected clearly.

### Stage 6 — Plugin folder manager and developer workflow

1. Finalize the versioned manifest and self-contained package format: metadata, API/SDK compatibility, platform binary path, optional dependencies and assets.
2. Implement configured development and per-user plugin roots; scan package folders, not arbitrary Rust source trees.
3. Validate manifests, paths, unique IDs, platform/architecture, compatibility and dependencies before loading.
4. Load enabled plugins through the isolated native loader; retain library handles while any plugin-owned panel, factory, callback, service, task or destructor may execute.
5. Isolate load failures and report actionable diagnostics without crashing the host.
6. Implement optional CLI/xtask commands to build, validate, package and install one standalone plugin project at a time.
7. Keep plugin source projects outside the host Cargo workspace. Normal application startup must not invoke Cargo or generate a static plugin registry.
8. Plugin changes take effect after restart; live machine-code hot replacement/unloading is not required.

**Exit condition:** Copying a compatible packaged plugin folder into a configured root makes it available after restart, with no host source change or rebuild.

### Stage 7 — Real first-party plugin vertical slice

1. Build/package Agent Chat and Git independently using the same SDK, manifest and loader path available to third parties.
2. Implement the initial real Agent Chat conversation panel: message history, composer and a minimal session/send/receive seam.
3. Implement the initial real Git contribution: repository selection, branch/status, changed files and basic diff inspection.
4. Define a plugin-owned typed Git repository/context contract and change events.
5. Have Agent Chat consume Git context/events through the shared Cordis runtime, without importing Git implementation code.
6. Verify both plugins can register appropriate panel types and multiple instances where supported.
7. Verify UI remains functional and plugin communication remains independent of dock/floating placement.
8. Exercise load failure, missing-plugin restoration and shutdown behavior.

**Exit condition:** Two separately compiled real plugins render native GPUI panels and interoperate through Cordis.

### Stage 8 — Framework template and default workspace

1. Define a replaceable first-run preset using registered Agent Chat/Git panel IDs.
2. Add a minimal panel/plugin catalog or launcher for discovering/opening contributions; it is not a permanent dock slot.
3. Construct the default workspace from plugin registrations, never hardcoded panel implementations.
4. Verify arbitrary rearrangement, split/tab operations, floating groups, persistence, reset, restart and missing-plugin recovery.
5. Verify third-party plugins use the same packaged folder, compatibility, SDK, registration and runtime path as first-party plugins.

**Exit condition:** Brucebox is a connected native framework template ready for ongoing workspace and plugin development.

### Working rules

- Inspect actual pinned dependency source/examples before using APIs; do not guess from other releases.
- Make focused architectural decisions/ADRs only when they have long-term consequences.
- Prefer complete vertical slices over throwaway prototypes. Agent Chat and Git are real product code, not test fixtures.
- Unit/integration tests for registry, persistence, manifest validation, ABI compatibility and lifecycle are encouraged.
- Keep the repository compiling; run formatting, check/build, tests and Clippy as appropriate.
- Do not bypass an unresolved ABI, GPUI-thread, library-lifetime or restoration requirement just to progress.
- Avoid speculative abstractions and unrelated refactors. Update this specification/plan when a material architectural decision changes.

---

## 10. Initial acceptance criteria

The foundation is successful when:

### Application
- [ ] Brucebox builds and launches as a native Rust/GPUI application.
- [ ] GPUI Kit initializes successfully.
- [ ] Linux Wayland operation is verified.

### Workspace
- [ ] Panels are registered independently of placement.
- [ ] The user can open multiple instances of a panel.
- [ ] Docked panels can be split and tabified.
- [ ] Docked panels can be resized and rearranged.
- [ ] Floating panels can be moved and resized.
- [ ] Panels can move between floating and docked surfaces.
- [ ] Workspace layout persists across restarts.
- [ ] The default layout is replaceable.
- [ ] No mandatory application-specific layout regions exist.
- [ ] Missing plugin panels do not destroy saved workspace state.

### Independent plugin loading
- [ ] Each plugin is built independently from the host.
- [ ] Each installed plugin is a self-contained folder with a manifest and platform-appropriate native library.
- [ ] Plugin folders are discovered at runtime.
- [ ] The host loads plugins dynamically rather than statically linking them.
- [ ] Adding or updating a plugin does not require rebuilding Brucebox.
- [ ] Normal application startup does not invoke Cargo.
- [ ] Plugin API/SDK compatibility is checked before registration.
- [ ] Plugin load errors are isolated and diagnosable.
- [ ] Native library handles remain alive for as long as plugin-owned code can execute.
- [ ] Plugin shutdown disposes owned resources before releasing library handles.
- [ ] First-party plugins use the exact same folder format and loader as third-party plugins.

### Plugin capabilities
- [ ] Plugins can implement arbitrary native GPUI components.
- [ ] Plugins can contribute multiple panel types.
- [ ] Plugins can open multiple panel instances.
- [ ] Plugins can register commands and runtime functionality.
- [ ] Plugins can be independently built and packaged outside the host Cargo workspace.

### Composition
- [ ] Cordis manages plugin composition and lifecycle.
- [ ] One host-owned runtime connects independently loaded plugins.
- [ ] Plugins can publish typed events.
- [ ] Plugins can subscribe to independently defined event contracts.
- [ ] Plugins can provide and consume typed services.
- [ ] One plugin can affect another without knowing its implementation.
- [ ] Real Git and Agent Chat contributions prove the cross-plugin path.

### Reliability
- [ ] The ABI/GPUI integration strategy is explicitly validated for the pinned compatibility set.
- [ ] Incompatible plugins are rejected cleanly.
- [ ] Workspace restoration handles unavailable plugins gracefully.
- [ ] Layout changes are persisted without excessive disk writes.
- [ ] Formatting, linting and relevant tests pass.

---

## 11. Explicit non-goals

Do not build these things as part of the initial foundation:

- Complete agent orchestration or ACP integration.
- A complete Git client.
- A complete plugin marketplace or remote package manager.
- General-purpose plugin sandboxing or fine-grained permission infrastructure.
- JavaScript or WASM plugins.
- Multiple native application windows.
- Live machine-code hot swapping or hot-unloading.
- Arbitrary cross-version Rust ABI compatibility.
- A complete application design system.
- Custom rendering implementation for GPUI Kit's entire Dock engine.
- Complex workspace animations.
- Production-grade plugin signing, notarization or update distribution beyond what is needed to install local plugin folders.
- Full production installer and auto-update infrastructure.

Native plugins are trusted in-process code. They are not sandboxed. The initial goal is an independently compiled, dynamically loaded native plugin system with a clearly pinned compatibility contract—not a universal binary extension ABI.

The goal is to establish the architectural foundation and prove it with real first-party features, not to build the complete product.

---

## 12. Future extension direction

Keep the following possibilities open without implementing them now:

- Plugin manager UI for discovering, enabling and disabling installed folders.
- Better plugin installation, update and version management.
- Plugin-provided workspace presets.
- Plugin dependency discovery and conflict resolution.
- Plugin ecosystems with independently published contract crates.
- Multiple workspace windows.
- Workspace presets and sharing.
- A more general freeform canvas surface.
- Custom workspace renderers.
- Optional embedded WebViews for plugins that genuinely need web content.
- More robust binary compatibility strategies if the ecosystem later needs looser version coupling.

The core architecture must not depend on compiling plugin source into the host. Independent plugin folders and runtime loading are the intended model, not a temporary development shortcut.

---

## 13. Upstream references

**GPUI Kit**
- Repository: https://github.com/longbridge/gpui-kit
- Documentation: https://gpui-kit.com/docs
- Dock component: https://gpui-kit.com/component/dock/
- Renderer-independent Dock infrastructure: https://gpui-kit.com/base/dock
- Architecture: https://github.com/longbridge/gpui-kit/blob/main/docs/ARCHITECTURE.md

**Cordis**
- Repository: https://github.com/dshbox/cordis-rs
- Documentation: https://docs.rs/cordis-rs/0.11.1/cordis/

**GPUI**
- Repository: https://github.com/zed-industries/zed
- Getting started: https://github.com/zed-industries/zed/tree/main/crates/gpui

## Final architectural principle

Brucebox is a workspace composition framework.

GPUI Kit provides the native rendering and baseline docking machinery. Brucebox provides the open workspace model, floating-surface composition and native plugin loader. Cordis provides the application composition runtime. The SDK makes host capabilities available to independently built plugin developers.

Plugins are independently compiled native projects, packaged as folders and loaded by the running Brucebox host. They contribute functionality and arbitrary native interfaces; they do not own predetermined application slots.

Users own the arrangement. The initial layout is a preset, not a constraint.

**Do not reintroduce static plugin compilation, generated plugin registries or mandatory application regions as implementation shortcuts.**
