# ADR 0002 — Workspace persistence and restore

- Status: accepted
- Date: 2026-11-05
- Scope: PLAN.md step 2
- Related: SPEC.md §4.2, §4.6, §4.7; ADR 0001

## Context

Step 1 made the workspace build itself from catalog contribution ids, but every
launch started from the same preset. Step 2 requires the arrangement to be the
user's: restored across restarts, saved without duplicating the dock tree, and
recoverable when a plugin is missing.

## Decisions

### 1. One document, dock tree inside, no parallel panel list

`WorkspaceDocument { schema, workspace, dock: DockAreaState }`.

GPUI Base Dock's tree is the only description of layout shape. A second array of
panels or surfaces would be a second thing to keep in step, and would need its
own migration path. Panel identity and plugin state therefore travel inside each
leaf's `PanelState` instead (see 2). `DockAreaState` already carries the dock's
own `version`, which the dock checks independently.

Floating surfaces (step 3) will add a surface list to the document, with a schema
bump; that is a shape the dock tree alone genuinely cannot express, unlike the
panel list this decision avoids.

### 2. The host envelope carries identity

`HostPanel::dump` writes:

```json
{
  "panel_name": "brucebox.git.diff",
  "children": [],
  "info": {
    "panel": {
      "schema": 1,
      "instance": 4,
      "owner": "brucebox.git",
      "panel": { "...the plugin's own info..." },
      "children": []
    }
  }
}
```

Why an envelope rather than a side table:

- The dock tree stays authoritative; restore needs no pairing between tree order
  and a panel list.
- Unknown panels already round-trip through GPUI Base's placeholder contract,
  which preserves the whole leaf state. The envelope rides along untouched, so a
  missing plugin loses nothing.
- Instance identity survives a restart, which `EntityId` cannot.

`restore_state` strips the envelope before the plugin factory is called, and
`PanelInstanceId::adopt` moves the mint counter past the restored id so a later
`next()` cannot reissue an id that is already on screen.

A leaf whose recorded `owner` disagrees with the registered descriptor's plugin
is **not** restored through that factory: another plugin's payload interpreted
with this plugin's state schema is a correctness bug waiting to happen. It becomes
a placeholder with an explanatory reason, and keeps its state.

A leaf with no envelope at all (older build, or a caller that only used the dock)
still restores: it gets a fresh instance id and its state is handed through
unchanged.

### 3. Files, not a database

`<user data dir>/workspaces/<workspace-id>.json`, pretty-printed.

- `BRUCEBOX_DATA_DIR` overrides the root (tests, parallel dev builds).
- Otherwise: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS,
  `$XDG_DATA_HOME`/`~/.local/share` elsewhere.
- Writes go to a sibling `.json.tmp` and are renamed into place, so a crash
  cannot leave a half-written layout.

A file that exists but cannot be parsed or is from another schema is an error,
not "no document". The workspace quarantines it to `.json.invalid`, records a
`RestoreFailed` diagnostic, and builds the default preset. Quarantining rather
than overwriting means the next save cannot destroy a layout written by a newer
build.

### 4. First launch, restore, and explicit reset

- No document: build the default preset.
- Document present and valid: restore it; the preset is not built at all.
- Document unusable: quarantine, build the preset, report why.
- Explicit reset: `ctrl-shift-r` replaces the layout with the default preset,
  removes the stored document, and writes the default back.

The reset shortcut is a real action (`brucebox::ResetWorkspace`) bound in
`workspace::init`, handled by the workspace root through `on_action`. The root
div takes focus at startup so the action dispatches before any panel is clicked;
focus moving into a panel still bubbles the action up to the root.

### 5. Debounced, coalesced saves

`DockEvent::LayoutChanged` fires per edit, so writes are debounced by 400 ms and
coalesced by generation counter: only the newest scheduled save may write. The
dump happens on the GPUI thread; serialization is foreground and the file write
runs on the background executor.

Quitting is handled separately from the workspace entity's lifetime. Closing the
window drops the root view, and with it the workspace, *before* app-quit
observers run — so an `on_app_quit` flush alone silently loses the last edit. The
window-closed hook fires while the entities are still alive and flushes
synchronously from the captured `Entity<DockArea>` and store. The app-quit flush
stays as a second path for a quit that happens with the window still open.

## Verification

Headless, with a private Xvfb display and `BRUCEBOX_DATA_DIR` pointing at a temp
directory. `scripts/x11-*.py` drive the otherwise manual steps, because xdotool
is not available here:

1. Fresh directory, launch, close the window → document written with schema 1,
   the dock tree, and one envelope per panel (`instance` 1–3, owner
   `brucebox.demo`).
2. Hand-edit the document to move `activity` into the left tab group, relaunch →
   the edited layout is restored, not the preset.
3. Insert a `brucebox.agent-chat.chat` leaf with a saved payload, relaunch →
   placeholder with the owning plugin named; close → the document still contains
   that panel, its instance id, and its payload.
4. Corrupt the document, relaunch → default preset, `.json.invalid` retained, new
   valid document written on close.
5. Launch with a modified document, click a tab, wait → the file is rewritten
   with the new active index before the app exits (debounce path).
6. Launch, press `ctrl-shift-r` → default preset restored and persisted.

## Known limits

- Save errors surface as a `WorkspaceDiagnostic` and on stderr at shutdown; there
  is no status UI yet.
- Dock state is trusted as the dock wrote it; per-panel `dump` output is opaque
  to the host, so a plugin with a broken `dump`/restore pair shows up as its own
  bug, not as a host failure.
- Only one workspace is persisted by the running app. Multiple named workspaces
  become meaningful with presets and the launcher (step 8).
