# Brucebox — Native Composable Workspace Foundation

**Project specification and initial implementation plan**

## 1. Product vision

Brucebox is a native desktop application framework and composable workspace.

It is not a conventional application with a fixed interface that plugins extend through predefined slots. It is an open workspace in which independently developed plugins contribute functionality and user interfaces that users can arrange however they choose.

The application provides the rendering, composition and workspace infrastructure. Plugins provide functionality and UI. Users decide how everything is composed.

The initial application should launch with a useful default workspace, but this is only a preset. Users must be able to dismantle it, rearrange it, replace its panels and build entirely different arrangements.

### Fundamental principles

1. **The workspace is user-owned.** No mandatory sidebars, content regions, toolbars or application-specific panel locations.
2. **Plugins own their interfaces.** Plugin developers can create completely custom GPUI components and visual systems.
3. **Plugins are composable.** They can provide services and events that other plugins consume.
4. **The host owns composition infrastructure.** Brucebox manages plugin discovery, lifecycle, communication and workspace integration.
5. **Layout is data.** The workspace can be rearranged, saved, restored, serialized and represented independently of its rendered GPUI views.
6. **Native first.** The application and its plugins are Rust/GPUI applications, not web applications embedded in desktop windows.
7. **Avoid reinventing existing infrastructure.** Use GPUI Kit for rendering and the baseline docking system, Cordis for composition, and Cargo for compilation.

## 2. Technology foundation

| Layer | Technology | Responsibility |
|---|---|---|
| Language | Rust | Application and plugin development |
| UI framework | GPUI | Native desktop rendering |
| UI infrastructure | GPUI Kit | Component system and workspace docking foundation |
| Composition runtime | Cordis-RS | Plugin lifecycle, services, dependencies and events |
| Plugin SDK | Brucebox SDK | Public development interface |
| Build system | Cargo | Plugin dependency resolution and compilation |
| Build tooling | xtask | Plugin discovery and registry generation |
| Persistence | Serde + JSON initially | Workspace and plugin configuration |

### Dependency versions

Use the following as the initial pinned baseline:

- `gpui-kit = "=0.9.0"`
- `cordis-rs = "=0.11.1"`
- Rust 2024 Edition.
- Rust toolchain pinned to an exact version verified against both dependencies.

GPUI Kit 0.9.0 bundles a matching, pinned GPUI snapshot (`gpui-pre` 0.3.11). Use its facade instead of independently adding a different GPUI version.

Do not separately introduce another GPUI, gpui-component or gpui-base version.

Cordis v3 uses Rust 1.88 or newer and exposes its application runtime through the `cordis` Rust import.

Pin all shared dependencies centrally and commit the resulting `Cargo.lock`.

## 3. High-level architecture

Brucebox should have five distinct architectural layers.

### A. Application host

The main executable.

Responsible for:
- Initializing the GPUI application.
- Creating the main window.
- Starting Cordis.
- Initializing the workspace engine.
- Discovering and registering enabled plugins.
- Coordinating application startup and shutdown.

The host should remain relatively small. It should not own individual plugin functionality or dictate the workspace's visual structure.

### B. Plugin composition — Cordis

Cordis owns application composition.

It manages:
- Plugin registration and lifecycle.
- Dependencies between plugins.
- Typed services.
- Typed events.
- Resource ownership and cleanup.
- Plugin activation and deactivation.

Plugins should be able to publish functionality without Brucebox explicitly knowing about it.

Avoid building a second plugin runtime, dependency injection system or event bus.

### C. UI framework — GPUI Kit

GPUI Kit provides:
- GPUI rendering.
- Native application and window infrastructure.
- UI components and primitives.
- Docking and tab infrastructure.
- Layout behavior and persistence.

For the initial implementation, use the styled Dock implementation exposed through GPUI Kit.

Plugins are not required to use its styled component library. They can use GPUI primitives directly and implement their own component systems.

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

Importantly, Brucebox should build on GPUI Kit's dock model instead of rewriting nested splits, tab groups and docking behavior.

### E. Plugin SDK

The common API used by first-party and third-party plugin developers.

It provides:
- GPUI Kit and GPUI re-exports.
- Cordis integration.
- Plugin registration interfaces.
- Panel registration and workspace integration.
- Access to host-provided services.
- Common plugin metadata and configuration interfaces.

The SDK is an integration layer, not a visual design system that plugin developers must follow.

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
- Diff Tree.
- File Diff.
- Branch Viewer.
- Commit History.

Users might open three separate File Diff panels.

Each opened instance must have its own identity and state, even though all three originate from the same plugin and panel type.

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

Use GPUI Kit's `gpui-component::dock` / `gpui_kit::component::dock` implementation for the initial docked workspace.

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

Use GPUI Kit's `DockSkin` as the initial presentation layer. Do not spend the first milestone designing a custom docking renderer.

Keep the workspace integration behind a Brucebox abstraction so the underlying GPUI Kit implementation can be changed or customized later.

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

A useful architectural approach is to let a floating surface contain its own DockArea. That allows panels inside a floating group to use the same nested splits and tabs as the main workspace.

The workspace controller should coordinate moving panel instances between:
- The main docked surface.
- Floating surfaces.
- Other floating surfaces.

The implementation should preserve panel identity and state when moving panels. Do not unnecessarily destroy and recreate an active plugin interface during a workspace rearrangement.

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
- Resize from their edges and corners.
- Change stacking order when focused.
- Snap to nearby workspace edges.
- Drop onto existing surfaces to tabify or split.

**Cross-surface operations**
- Undock a docked panel.
- Move a floating panel into the docked workspace.
- Move panels between floating surfaces.
- Combine floating surfaces into tab groups where appropriate.

Implement a workspace-level drag coordinator for operations that cross between separate DockArea instances.

Do not assume GPUI Kit's existing DockArea automatically handles cross-area transfers.

The first milestone should demonstrate basic floating, resizing and docking back into the central surface. More sophisticated snapping and multi-surface tabbing can follow once the integration is validated.

### 4.6 Workspace data model

Workspace layout must be represented as serializable data rather than a hardcoded arrangement of GPUI entities.

Use GPUI Kit's `DockAreaState` for the internal state of docked surfaces.

Brucebox should wrap this in its own versioned workspace document.

Conceptually:

- Workspace schema version.
- Workspace identifier.
- Main DockArea state.
- Collection of floating surfaces.
- Floating surface bounds and stacking order.
- DockArea state for each floating group.
- Panel instance metadata.

Do not duplicate GPUI Kit's entire `PaneTree` implementation in Brucebox.

The Brucebox workspace document should compose the underlying DockArea state with Brucebox-specific surface information.

Panel state should be owned by the panel implementation and serialized through its restoration interface.

### 4.7 Presets and persistence

A workspace preset is simply a saved `WorkspaceDocument`.

Brucebox should eventually support:
- Default workspace.
- User-created presets.
- Saving the current arrangement.
- Restoring a previous arrangement.
- Resetting to a default preset.
- Switching between presets.

For the initial implementation, one default preset and automatic persistence of the current workspace are sufficient.

Never treat the default workspace as mandatory structure.

If a plugin is unavailable during restoration, display a recoverable placeholder or missing-panel indicator and preserve its saved state where possible.

---

## 5. Plugin architecture

### 5.1 Initial compilation model

For the first implementation, plugins should be independent Rust crates that are compiled into the Brucebox executable.

This deliberately avoids a custom dynamic-library ABI while the architecture is being validated.

A plugin is a source project placed under the plugin directory.

The build utility discovers installed/enabled plugin crates, generates a dependency manifest and plugin registration registry, and invokes Cargo.

Adding a plugin must not require modifying the application source code.

This is the initial development and feasibility model. Independently compiled native shared libraries can be evaluated later without changing the overall conceptual plugin model.

### 5.2 Plugin directory

Use a structure similar to:

```text
plugins/
├── demo-git/
│   ├── plugin.toml
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
│
├── demo-strobe/
│   ├── plugin.toml
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
│
└── contracts/
    └── git-contracts/
        ├── Cargo.toml
        └── src/
            └── lib.rs
```

Every plugin has:
- Unique ID.
- Name and version.
- Package name.
- Enabled state.
- Rust crate.
- Plugin registration entry point.

The discovery utility validates manifests and generates the application plugin bundle.

### 5.3 Plugin registration

The SDK should provide a unified developer-facing registration model, with separate responsibilities internally.

A plugin can register:

**UI contributions**
- Panel types.
- Panel factories.
- Commands.
- Optional workspace presets.

**Runtime contributions**
- Cordis plugins.
- Services.
- Event listeners and publishers.
- Background tasks.
- Other application capabilities.

Do not require every plugin to contribute a panel.

Do not require every plugin to provide runtime services.

Plugins may be entirely visual, entirely functional, or a combination.

### 5.4 Plugin-owned UI

The SDK must expose the native GPUI API.

A plugin developer should be able to implement:
- Custom `Render` and `RenderOnce` views.
- Custom elements.
- Custom drawing and painting.
- Custom animations.
- Custom component libraries.
- Custom focus and interaction behavior.
- Custom internal panel layouts.

Using GPUI Kit's styled components should be optional.

A plugin must never be forced to use Brucebox-provided buttons, cards, menus, themes or design tokens.

Brucebox controls the placement and management of the panel, not its internal visual design.

### 5.5 Plugin communication

Cordis handles plugin-to-plugin communication.

Plugins can:
- Publish typed events.
- Subscribe to typed events.
- Publish typed services.
- Consume services published by other plugins.
- Declare service dependencies.
- Respond to changes in service availability.

Plugin-specific contract crates should be independent of plugin implementations.

For example:

- Demo Git publishes a `CommitCompleted` event.
- Demo Strobe consumes that event.
- Demo Strobe reacts by updating its own interface.

The Brucebox host does not need to understand the semantics of the Git event.

Plugin developers should be able to publish their own contracts, and other plugins can depend on them.

Do not create a central list of every capability that Brucebox developers must anticipate.

### 5.6 SDK responsibility

The SDK provides the common integration interface, not a predefined capability catalog.

It should expose host functionality such as:
- Workspace panel registration.
- Command registration.
- Access to shared application context.
- Cordis runtime integration.
- Basic notification and UI integration.
- Plugin metadata and configuration interfaces.

Everything else should be extensible through Cordis.

---

## 6. GPUI and Cordis runtime integration

GPUI's UI state and window context must remain on the GPUI application thread.

Cordis should manage application lifecycle and asynchronous plugin functionality without directly manipulating GPUI view state from arbitrary background tasks.

Use a clear bridge:

- GPUI user interactions can initiate application or Cordis operations.
- Cordis plugins can publish events and perform asynchronous tasks.
- Plugins can send typed messages to the GPUI host.
- The GPUI host applies UI changes using its own application context.

Avoid introducing IPC, JSON serialization or process boundaries between plugins. They are part of the same compiled application.

The host should maintain explicit ownership of Cordis plugin lifecycle handles and dispose them during application shutdown.

Follow Cordis's actual lifecycle and teardown semantics instead of assuming dropping a handle automatically stops a plugin.

---

## 7. Initial project structure

Create Brucebox as a new, standalone Rust project.

Do not migrate Iceplane application code or reuse its previous application structure.

Suggested initial layout:

```text
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
│   ├── brucebox-runtime/
│   └── brucebox-workspace/
│
├── plugins/
│   ├── demo-git/
│   ├── demo-strobe/
│   └── contracts/
│       └── git-contracts/
│
├── tools/
│   └── xtask/
│
├── generated/
│   └── plugin-registry/
│
├── assets/
│
└── scripts/
    └── dev.sh
```

### Package responsibilities

**brucebox-desktop**
- Executable.
- GPUI initialization.
- Main window.
- Startup and shutdown.
- Plugin registry invocation.
- Workspace initialization.

**brucebox-sdk**
- Public plugin development interface.
- GPUI Kit re-exports.
- Cordis integration interfaces.
- Panel and command contribution contracts.
- Plugin metadata.

**brucebox-runtime**
- Cordis initialization.
- Plugin lifecycle management.
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
- Plugin discovery.
- Manifest validation.
- Plugin dependency generation.
- Plugin registry generation.
- Development build and launch commands.

**generated/plugin-registry**
- Generated Cargo dependencies.
- Generated plugin registration functions.
- No manually maintained implementation logic.

Keep these packages small. Avoid adding abstractions until an actual integration requirement justifies them.

---

## 8. Initial demonstration plugins

Create two simple but genuinely independent demonstration plugins.

### Demo Git

Contributes:
- Git Diff Tree panel.
- Commit Composer panel or a simple Commit button.
- `CommitCompleted` typed event.

Use simulated Git data. Full Git implementation is outside this milestone.

### Demo Strobe

Contributes:
- A completely custom GPUI panel.
- A visual pulsing/strobe indicator.
- An event counter.
- A listener for `CommitCompleted`.

When the Demo Git plugin emits its event, the Demo Strobe plugin should receive it and update its own UI.

No real hardware control is needed.

### Why these plugins matter

They prove three separate things:

1. A plugin can contribute custom native UI.
2. A plugin can communicate with another plugin through Cordis.
3. A plugin's UI and functionality remain independent of its location in the workspace.

Both panels must be independently movable, resizable, split and tabifiable.

The Strobe panel must still receive events regardless of where it is located.

---

## 9. Initial implementation plan

Implement this in deliberate stages. Each stage should produce a working result that can be tested before proceeding.

### Stage 1 — Establish the Rust and GPUI Kit foundation

1. Create a clean Brucebox repository.
2. Install and configure the Rust toolchain.
3. Create the Cargo workspace.
4. Add GPUI Kit 0.9.0.
5. Add Cordis-RS 0.11.1.
6. Verify that the dependency graph resolves without duplicate GPUI or Cordis versions.
7. Create the smallest possible GPUI application.
8. Open a native window and render a basic view.
9. Verify Wayland operation on Linux.
10. Commit the dependency lockfile.

**Exit condition:** Brucebox launches as a minimal native GPUI application using GPUI Kit.

Do not proceed until this works reliably.

### Stage 2 — Establish the baseline workspace

1. Create `brucebox-workspace`.
2. Integrate GPUI Kit's DockArea using `DockSkin`.
3. Create two basic test panels.
4. Register the panels with the DockArea.
5. Create an initial nested split layout.
6. Verify panel dragging, tab creation, tab reordering and split resizing.
7. Test closing and reopening panels.
8. Save and restore `DockAreaState`.

Use the GPUI Kit docking example as the primary implementation reference.

Do not create a custom docking engine.

**Exit condition:** A functioning dockable workspace with native GPUI panels and persistent nested layouts.

### Stage 3 — Establish Brucebox's workspace abstraction

1. Introduce stable panel type identifiers.
2. Introduce unique panel instance identifiers.
3. Create a central panel catalog.
4. Allow plugins to register panel factories without choosing panel locations.
5. Implement a command palette or lightweight panel launcher.
6. Allow opening multiple instances of the same panel type.
7. Introduce a versioned `WorkspaceDocument`.
8. Wrap GPUI Kit's DockArea state in that document.
9. Add the default workspace as a replaceable preset.

The workspace must remain usable even if its initial preset is completely rearranged.

**Exit condition:** The workspace can be composed dynamically from registered panels without hardcoded UI slots.

### Stage 4 — Establish floating surfaces

1. Introduce the floating-surface model.
2. Render floating surfaces within the existing GPUI application window.
3. Support independent position and dimensions.
4. Implement drag-to-move.
5. Implement edge and corner resizing.
6. Support z-order and focus.
7. Allow a docked panel to be detached into a floating surface.
8. Allow a floating panel to be returned to the main DockArea.
9. Implement basic drop previews and docking targets.
10. Extend workspace persistence to include floating surfaces.

Use GPUI Kit's existing DockArea for nested tabs and splits inside floating groups where practical.

Do not create another GPUI application.

This is the one significant workspace capability Brucebox needs to implement on top of GPUI Kit.

**Exit condition:** A panel can be docked, detached, moved, resized and docked again without losing its identity or state.

### Stage 5 — Create the plugin SDK and source-plugin workflow

1. Establish `brucebox-sdk`.
2. Re-export the required GPUI Kit and Cordis APIs.
3. Define the plugin metadata and registration interface.
4. Define the UI panel contribution interface.
5. Define the runtime contribution interface.
6. Create the `plugin.toml` manifest format.
7. Implement plugin folder discovery.
8. Implement manifest validation.
9. Generate the plugin Cargo dependency bundle.
10. Generate the plugin registration registry.
11. Create a simple development launcher that runs generation, Cargo build and application startup.

The expected workflow should be:

- Create a plugin directory.
- Add a Rust crate.
- Add its manifest.
- Implement its registration entry point.
- Run Brucebox's development command.

No Brucebox application source changes should be required to add a plugin.

**Exit condition:** An independently authored plugin folder is incorporated into Brucebox through the build and registration workflow.

### Stage 6 — Connect Cordis

1. Initialize the Cordis runtime.
2. Implement startup registration for compiled plugins.
3. Retain plugin lifecycle handles.
4. Integrate plugin shutdown.
5. Create the Demo Git contract crate.
6. Have Demo Git publish `CommitCompleted`.
7. Have Demo Strobe consume that event.
8. Verify that the Strobe UI updates through the GPUI messaging bridge.
9. Implement a minimal service-provider and service-consumer example.

**Exit condition:** Independent plugins can publish and consume typed functionality without directly depending on one another's implementation.

### Stage 7 — Integrate and test the complete vertical slice

1. Register the two demo plugins.
2. Populate the default workspace with their panels.
3. Open additional panel instances.
4. Split and tabify panels.
5. Float and move panels.
6. Save the workspace.
7. Restart Brucebox.
8. Restore the workspace.
9. Confirm plugin communication still functions after restoration.
10. Disable one demo plugin and verify that missing-panel state is handled without destroying the saved workspace.

**Exit condition:** The complete foundation works as one composable native application.

---

## 10. Initial acceptance criteria

The initial foundation is successful when:

### Application
- [ ] Brucebox builds and launches as a native Rust/GPUI application.
- [ ] GPUI Kit and Cordis initialize successfully.
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

### Plugins
- [ ] Plugins are independently maintained Rust crates.
- [ ] Plugin source folders are discovered automatically.
- [ ] New plugins do not require application source modifications.
- [ ] Plugins can implement entirely custom GPUI components.
- [ ] Plugins can contribute multiple panel types.
- [ ] Plugins can open multiple panel instances.
- [ ] Plugins can register commands and runtime functionality.

### Composition
- [ ] Cordis manages plugin lifecycle.
- [ ] Plugins can publish typed events.
- [ ] Plugins can subscribe to independently defined event contracts.
- [ ] Plugins can provide and consume typed services.
- [ ] One plugin can affect another without knowing its implementation.

### Reliability
- [ ] Plugin shutdown disposes owned Cordis resources.
- [ ] Workspace restoration handles unavailable plugins gracefully.
- [ ] Layout changes are persisted without excessive disk writes.
- [ ] Formatting, linting and relevant tests pass.

---

## 11. Explicit non-goals

Do not build these things as part of the initial foundation:

- Agent orchestration.
- ACP support.
- Group chat.
- Agent UI.
- Git integration beyond the demonstration.
- A complete plugin marketplace.
- General-purpose plugin security infrastructure.
- JavaScript or WASM extensions.
- Dynamic shared-library loading.
- Rust ABI stabilization.
- Live plugin replacement.
- A complete application design system.
- Custom rendering implementation for GPUI Kit's entire Dock engine.
- Multiple native application windows.
- Complex workspace animations.
- Production installation and auto-update infrastructure.

The goal is to establish the architectural foundation and test its viability, not to build the complete product.

## 12. Future extension direction

Keep the following possibilities open without implementing them now:

- Independently compiled native Rust plugins.
- Runtime loading of native shared libraries.
- A more general freeform canvas surface.
- Multiple workspace windows.
- Workspace presets and sharing.
- Plugin-provided workspace presets.
- Plugin dependency discovery.
- Plugin-to-plugin ecosystems and published contract crates.
- Custom workspace renderers.
- Optional embedded WebViews for plugins that genuinely need web content.

The source-plugin build model is the initial way to establish a working native ecosystem. It should not become a permanent architectural dependency that prevents later experimentation with native shared-library plugins.

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

GPUI Kit provides the native rendering and baseline docking machinery. Brucebox provides the open workspace model and floating-surface composition. Cordis provides the plugin runtime. The SDK makes the host capabilities available to plugin developers.

Plugins contribute functionality and arbitrary native interfaces. They do not own predetermined application slots.

Users own the arrangement.

The initial layout is a preset, not a constraint.

**Do not compromise this principle by gradually introducing mandatory application regions as implementation shortcuts.**