# ADR 0004 — Cordis as the one composition runtime, and the bridge to GPUI

- Status: accepted
- Date: 2026-11-05
- Scope: PLAN.md step 4
- Related: SPEC.md §3.B, §5.6, §5.8, §6; ADR 0001, ADR 0002, ADR 0003

## Context

SPEC §3.B puts composition on Cordis: plugin lifecycle, service dependencies,
typed events, and resource ownership. SPEC §6 also says GPUI entity and window
state stays on the application thread, and that a background task must never
mutate a view.

Those two requirements meet at one seam. A plugin's `apply` body runs wherever
the runtime polls it; a panel factory produces `Rc`-based GPUI entities and
cannot leave the application thread at all. So the host cannot hand a plugin a
`&mut App`, and it cannot move a factory across threads to meet the plugin
halfway.

The pin is `cordis-rs = "=0.11.1"` with `cordis-core 0.6.1`. Verified against
the published crate: `Context` is a cheap clone, `Context::spawn` takes a
`PreparedPlugin` and returns a `FiberHandle` that settles `Active` or `Pending`,
`InjectSpec` declares required *services* (lifecycle prerequisites, not
constructor injection), and disposal is explicit — dropping a `FiberHandle` is
ordinary Rust drop, not teardown.

## Decisions

### 1. One runtime, owned by the host, on its own threads

`HostRuntime` creates the tokio executor (two worker threads) and the root
`Context`. Every plugin is admitted to that one root context, which is what lets
a service published by one plugin resolve for another, and every lifecycle
handle is kept by the host.

The runtime is never polled from the application thread: work is submitted to
the executor and results come back as messages. Only two operations block, and
both sit outside the frame loop — startup, before the first window exists, and
shutdown, after the last one is gone.

### 2. The boundary is messages of plain data, not calls

`UiMessage` is the whole interface: `Declared(PluginDeclaration)`,
`Withdrawn(PluginId)`, `Phase { plugin, phase, detail }`, and `Note(String)`.
None of it names a Cordis type, so the workspace applies a message without
knowing a composition runtime exists, and no GPUI type reaches a plugin.

The application side drains the mailbox in a GPUI task, applies each message
under `&mut App`, and asks for a redraw. Startup drains it synchronously before
the first frame, so the initial layout never depends on a task being polled.

The obvious alternative — plugins calling `register_contribution(&mut App, …)`
directly — is what the crate looked like in step 3, and it cannot work once
`apply` runs off the application thread.

### 3. Contributions are declarations plus application-thread factories

A declaration crosses threads; a factory does not. `PluginDeclaration` carries
ids, titles and multiplicity. The code that builds the panel is registered
separately in `PanelFactories`, keyed by panel type id, and
`catalog::register_declaration` joins the two.

This keeps the ABI-sensitive question ("how does a plugin's code get called
here?") at the loader boundary that step 5 has to solve anyway, instead of
pretending a `Rc<dyn PanelFactory>` can be moved between independently compiled
libraries.

A declaration is checked whole before anything is registered: a plugin missing
one factory is refused entirely rather than landing half-contributed.

### 4. Registration is not a special path for built-in panels

The host submits its own plugins through the same `start_plugin` call a loader
would use. `WorkspaceAdapter` publishes the workspace service; the built-in
contributions plugin requires it, and declares its panels by publishing a
`PanelsDeclared` event.

That pair is the smallest honest test of the two things composition is for: a
dependency that must exist before `apply` runs, and a typed event carrying a
contribution across a plugin boundary.

The dependency order at startup is deliberate: the service is published first,
so the contributions plugin settles `Active` inside admission rather than
converging after the first window has already resolved a layout. The parked case
is not skipped — it is what the tests exercise, and what convergence reporting
exists for.

### 5. Lifecycle is reported when it changes

`start_plugin` waits for settlement and returns the phase, so the host knows what
it got. A plugin that settles `Pending` is watched with
`FiberHandle::wait_state(FiberState::Active, …)`; when the missing service
appears, the interface is told. A plugin whose `apply` fails is normalized by
Cordis into an opaque failure, reported as `Failed`, and the runtime keeps
running — one bad plugin never takes the host with it.

### 6. The host owns teardown, in spawn order

Cordis deliberately has no runtime-wide shutdown, so `HostRuntime::shutdown`
disposes every handle it kept, newest first, attempt-all. Reverse spawn order is
application policy: a plugin admitted later may depend on an earlier one.

The registry is a `Vec` in spawn order, not a map keyed by plugin name. The
first version was keyed by name and a test caught the consequence: two plugins
reporting the same name left one handle in the map, and the overwritten handle
was dropped rather than disposed — which, for Cordis, means the generation's
cleanup never ran.

Plugin-owned cleanup is registered by the plugin, not the host: the built-in
contributions plugin withdraws its declarations from a generation-owned effect,
so unloading a plugin removes its panels rather than leaving the catalog
claiming panels nothing can build.

## Verification

Headless (Xvfb, `BRUCEBOX_DATA_DIR`, `scripts/x11-*.py`):

1. Fresh launch → the window opens with the three proving panels, which arrived
   as a declaration from a Cordis plugin: the saved document names
   `brucebox.demo.welcome`, `.scratch` and `.activity`.
2. The **Activity** panel shows `brucebox.demo` and `brucebox.host.workspace`
   both `active` — runtime lifecycle reaching the interface across the bridge.
3. The status feed ticks (`brucebox.demo alive · tick N`, every 5 s), so the
   generation-owned task is running and its messages are being applied and
   painted.
4. Graceful window close → the process exits, which means the quit hook ran the
   blocking reverse-order dispose instead of deadlocking on the task join.
5. Relaunch → the saved layout is restored through the same declaration path,
   with no placeholder panels, and the Activity panel's saved tab selection is
   honoured.

Tests (61 total, `cargo test`): the runtime admits the host plugins and their
declaration reaches the mailbox; a plugin requiring the workspace service parks
`Pending`, names the missing service, never runs `apply`, and converges to
`Active` — delivering its declaration — once the service is published; shutdown
disposes newest-first and runs every generation's cleanup; a plugin whose
`apply` fails is reported and the runtime keeps admitting; mailbox ordering,
bounded notes and the closed-interface case.

## Known limits

- Startup admits plugins one at a time and blocks on settlement (10 s timeout
  each). Correct for two host plugins; the loader in step 6 will want a budget
  for the whole set.
- `WorkspaceService` is one-way. A plugin learns that its contribution was
  refused only from the status feed; nothing tells it to stop working. A
  registration acknowledgement belongs with the SDK, where a plugin can act on
  it.
- Shutdown blocks the application thread while generations drain. Fine at quit;
  do not reuse it for anything that must stay responsive.
- There is no dynamic load or unload yet: plugins are host-owned values admitted
  at startup. Step 5's ABI boundary and step 6's folder manager are what make
  `start_plugin` take something a loader produced.
