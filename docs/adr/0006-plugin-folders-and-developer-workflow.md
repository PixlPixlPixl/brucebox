# ADR 0006 — Plugin folders, installation state, and the developer workflow

Status: accepted (PLAN step 6)

## Context

Step 5 proved the native boundary: a separately built `cdylib` loads at runtime,
registers real GPUI panels, and joins the composition runtime. It left the
*operational* side thin:

- the only way to point the host at a package was `BRUCEBOX_PLUGIN_DIR`;
- packaging was a Python script with its own copy of the manifest rules;
- there was no way to switch a plugin off, no dependencies, no install step, and
  nothing in the interface that said why a plugin was missing;
- discovery, loading and reporting all lived in one host module, so the tooling
  could not share any of it.

That is the gap step 6 closes. The question was not "can a plugin load" but "what
does an installation look like, who decides what runs, and how does a developer
work on one without touching the host".

## Decision

**1. Split the format, the installation state, and loading into three crates.**

| crate | owns | may not do |
|---|---|---|
| `brucebox-plugin-manifest` | `plugin.toml`, versions, platform keys, dependencies, folder discovery | open a library |
| `brucebox-plugin-install` | data directory, plugin roots, `plugins.toml`, per-folder verdicts | open a library, know about GPUI |
| `brucebox-plugin-loader` | `dlopen`, symbol lookup, registration validation, handle lifetime | decide *what* to load |

`unsafe` stays in the loader. Everything above it — the host's startup path and
`cargo xtask` — makes its decisions from the same two data crates, so the tool
cannot disagree with the host about what a package is or whether it is enabled.
The immediate reason for this split was concrete: the step 5 packaging script had
a hand-copied platform-key table, which is exactly the kind of drift that makes a
package build on one machine and not load on the other.

**2. Roots are ordered, and the first folder to claim an id keeps it.**

Discovery searches `BRUCEBOX_PLUGIN_DIR` (development roots, `:`-separated) first
and `<data dir>/plugins` (the user root) last. Within a root, folders are visited
in name order. A second folder declaring an installed id is refused and reported;
it never half-loads. That makes the common developer situation predictable: a
package you are iterating on shadows the installed copy, and the shadowed copy is
visible in the panel instead of silently disappearing.

**3. Switching a plugin off is config plus restart, never an unload.**

`plugins.toml` in the data directory lists only ids that are off. A missing file,
an unreadable one, and a stale id are all non-fatal: the host starts, reports the
problem, and treats unlisted plugins as enabled.

The alternative — unloading on toggle — is not offered, because it cannot be done
correctly (ADR 0005): plugin code is reachable from GPUI entities, Cordis
factories and trait objects the host does not own, and Rust has no stable ABI to
prove otherwise. So `Disable` writes the config and the panel says
"switches off at the next start"; the next start honours the config *before* any
library is opened. The panel labels the disagreement between config and running
process rather than hiding it.

**4. Dependencies are declared, checked at discovery, and reported not repaired.**

```toml
[dependencies]
requires = ["brucebox.git", "brucebox.workspace@0.1"]
```

A dependency is a plugin id, optionally with a version *prefix*. It is resolved
against packages that are installed, readable and enabled. A missing, disabled or
incompatible dependency keeps the depending plugin out of the workspace with a
reason that names the dependency and, for a version mismatch, the version that is
actually installed. It does not attempt a fix, an ordering trick or a retry.

This is a *presence* check, not a service binding: whether a required service is
actually published is still Cordis's job at runtime (a plugin that requires a
service parks `Pending`, ADR 0004). The manifest check exists so the common case —
the dependency is not installed at all — is a startup row, not a plugin stuck
pending forever.

**5. `cargo xtask plugin ...` is the developer workflow, and only that.**

```text
list      what is installed, where, and may it run
validate  one folder or one project, before installing it
pack      build one plugin project and package it into target/plugin-packages/<id>
install   copy a package folder into the user plugin root
enable / disable  edit the same config the panel edits
```

`pack` runs Cargo **in that one project** (which has its own lockfile and is
excluded from the host workspace) and locates the single library it produced.
The host never runs Cargo: the only build-time coupling is the pinned
toolchain and library versions, which is what the ABI already requires. The step 5
Python packaging script is deleted rather than kept as a second implementation.

**6. The interface shows the host's verdicts, not its own opinion.**

The Plugins panel renders exactly the rows the host decided at startup — one per
folder, including duplicates, disabled plugins and refused manifests — with the
reason string the host used, the root it came from, and the config path. The only
action it offers is the switch, which writes the config. There is no "rescan"
button: a rescan would need to unload libraries, which is the one thing this
design will not do; the entry point for a rescan is a restart.

## Consequences

- A plugin installation is a plain folder copy. `cargo xtask plugin install` is a
  convenience, not a requirement, and an installation can be produced on one
  machine and copied to another with the same platform key.
- Every failure mode has one message, produced where the decision was made. The
  panel, the Activity notes and `cargo xtask plugin list` show the same words
  because they read the same `PackageState`.
- Config is per installation, not per workspace, and is not part of the saved
  workspace document.
- The host's startup cost is one directory walk per root plus one `dlopen` per
  ready package.

## Verified

On Xvfb, with a clean data directory:

1. `cargo xtask plugin pack plugins/agent-chat` produced
   `target/plugin-packages/brucebox.agent-chat/` with the manifest and the
   platform library.
2. `cargo xtask plugin install <that folder>` copied it into the user root, and
   the host — started with **no** `BRUCEBOX_PLUGIN_DIR` and no rebuild — loaded it:
   Activity showed `brucebox.agent-chat` active with "contributed 1 panel type".
3. The Plugins panel listed the package as `ready`, with its user-root path.
4. Clicking `Disable` wrote `plugins.toml` (`"brucebox.agent-chat" = false`) and
   the row immediately showed "switches off at the next start".
5. After a restart the plugin was not loaded: the panel showed `disabled` with
   the config reason, and Activity showed the plugin as `disabled`.
6. Clicking `Enable` removed the id from the config, showed "switches on at the
   next start", and after another restart the plugin was active again.
7. `cargo xtask plugin list` agreed with the panel at every step, including the
   disabled reason; `validate` accepted the packaged folder and the project, and
   refused a non-namespaced id with the same message the host would print.

## Known limits

- Discovery is sequential and synchronous; it runs before the first window and is
  cheap because it reads manifests and never libraries.
- Version requirements are prefix-only (`0.1` matches `0.1.4`); there are no
  ranges and no transitive constraint solving.
- `install` replaces a package with the same id instead of keeping versions
  side by side, and does not verify that the copied library matches the manifest
  beyond what loading checks.
- No hot reload: a rebuilt plugin is picked up on the next start (the library is
  never unloaded, so the process cannot swap it).
- A plugin disabled while running keeps its panels and services until restart.
