# ADR 0008 — The first-run workspace is a composition rule, not a layout

Status: accepted (PLAN step 8)

## Context

Up to step 7 the host's default workspace was a fixed preset naming four host
panels, written out by hand in `preset.rs`. That was correct while the panels were
built in, and wrong the moment they were not:

- a plugin the user installed did not appear until they found its tab;
- the arrangement could not represent "Agent Chat and Git are the first-party
  products" without hardcoding panel *implementations* in the host;
- there was no way to open a panel that the preset did not happen to place, so a
  plugin's second or third panel type was unreachable.

SPEC §9 stage 8 asks for the opposite: a replaceable first-run preset built from
registrations, and a discovery surface for opening contributions that is not a
permanent dock slot.

## Decision

**1. The preset is a rule over the live catalog.**

`preset::default_layout_from(&[CatalogPanel])` is pure and takes whatever is
registered. The rule:

1. panel type ids listed in `PREFERRED` come first, in that order — Agent Chat,
   then Git, then the host's own panels;
2. every other registered panel type follows in registration order, so a plugin
   nobody anticipated is still placed;
3. the first panel becomes the main column, the rest share a second column, and an
   empty column is dropped.

The ids in `PREFERRED` are *preferences expressed as data*: nothing references a
panel implementation, and a missing plugin is skipped rather than asserted. The
only place in the host that names a plugin panel type is this list and the
contribution table for the host's own panels; `grep -rn 'brucebox\.git' src/`
finds nothing else outside tests.

**2. The panel catalog is a host panel, opened by `Ctrl+Shift+P`.**

`brucebox.demo.panels` lists every registered panel type grouped by plugin, with
its multiplicity and how many instances are open. Clicking a row opens it.

It is written in the host rather than as a plugin because clicking means pointer
handling, and GPUI's own listeners cannot fire inside a plugin library (ADR 0005).
Making the catalog a plugin would have required the input bridge to carry
hit-testable geometry — a much larger change than the feature is worth.

It is a normal panel, not a permanent slot: it can be closed, floated or moved,
and the shortcut reopens it.

**3. Opening a panel has exactly one path: `workspace::open_panel`.**

The catalog asks the active workspace (a host-side `ActiveWorkspace` global, not a
plugin API) to create the panel through its plugin's factory and hand it to
`DockArea::add_panel_view`. That is the same factory path a restored document
uses, so a panel opened this way has normal identity, persistence and plugins.

The path enforces the declared multiplicity: a `Single` type with an instance open
is refused with a message in the catalog, and a `Multiple` type opens freely
(verified with two Scratch Pads and a refused second Chat).

**4. Instance counts are cached, not derived on demand.**

This was found by crashing, twice:

```text
thread 'main' panicked: cannot read LauncherPanel while it is already being updated
```

Counting instances by dumping the dock (`WorkspaceDocument`) reads every panel
entity — including the catalog panel that is currently running its own click
handler, which GPUI forbids. The same applies to the synchronous save the first
version performed.

So the workspace keeps `instances: BTreeMap<PanelTypeId, usize>`, seeded once at
build/restore, updated on open, and recounted from the dock's `LayoutChanged`
event — which runs outside any panel update. Opening a panel schedules the
debounced save instead of flushing.

## Consequences

- Installing a plugin changes the first-run arrangement with no host change. Two
  plugins installed and one disabled produce different layouts from the same code.
- A panel type with no preset entry is discoverable, because the catalog lists
  registrations rather than the preset.
- Multiplicity is enforced at the only place a new instance is created, so a
  `Single` panel cannot be opened twice from the interface (a hand-edited document
  can still do it; this is not a security boundary).
- The catalog is host UI, so it works even when every plugin is missing — which is
  what makes it usable as the recovery path in that case.

## Verified

Two plugins installed in the user plugin root, one data directory, Xvfb:

1. First launch with no saved document: the workspace is Chat in the main column
   and Repository, Diff, Welcome, Activity, Panels, Plugins, Scratch Pad as tabs —
   composed from registrations, with Git's panels present because Git is installed.
2. The Panels tab lists all four plugins' contributions with multiplicity and open
   counts (`Chat · single · 1 open`, `Scratch Pad · multiple · 2 open`).
3. Clicking `Scratch Pad` opened a second instance; the count became `2 open` and
   the saved document contains two `brucebox.demo.scratch` panels.
4. Clicking `Chat` was refused: *"Cannot open: `brucebox.agent-chat.chat` allows 1
   instance(s) and 1 is already open"*.
5. `Ctrl+Shift+R` rebuilt the preset from the current registrations: the extra
   Scratch Pad was gone and the order was Chat | Repository, Diff, Welcome,
   Activity, Panels, Plugins, Scratch Pad.
6. The restored document survived a restart with the extra instances intact, and
   the Chat panel's Git line still updates (the cross-plugin contract from step 7
   is untouched by any of this).

## Known limits

- The catalog is a list, not a launcher with search, drag-and-drop or keyboard
  navigation; arrow keys would not reach a plugin panel anyway (ADR 0007), and the
  catalog is host UI only so it could support them — it just does not yet.
- `PREFERRED` is a curated order. A third-party plugin is placed after the known
  ones, which is the intended behaviour but is not user-configurable.
- Instance counts are per window and not persisted: they are recounted on restore.
- Removing a panel is the dock's own affordance; the catalog only opens.
