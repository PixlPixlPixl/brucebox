# ADR 0007 — Cross-plugin contracts cross as data

Status: accepted (PLAN step 7)

## Context

Steps 5 and 6 proved that an independently built plugin can register native GPUI
panels and join the host's Cordis runtime. Step 7 asks for the thing those stages
deliberately postponed: **two plugins cooperating**. SPEC §5.8 puts it in terms of
the runtime — one plugin publishes a typed service and events, another consumes
them, "through their own versioned contract, not direct imports of each other's
implementation".

The obvious implementation is a Cordis service: Git publishes a
`GitRepository` service, Agent Chat injects it. That is what the runtime is for,
and it was the first thing tried. It does not work across plugins, and the reason
is measurable rather than mysterious.

### Measurement 1 — a service value cannot cross a library boundary

Cordis keys services by `S::NAME` but *stores* them as `Arc<dyn Any + Send + Sync>`
and returns them through `downcast::<S>()`, with an explicit
`ContractMismatch` guard comparing `TypeId::of::<S>()` against the publication.
Two separately compiled libraries only agree on that `TypeId` while the whole
dependency metadata of the crates involved matches.

A three-crate experiment (`shared` defines the type; `a` and `b` are separate
`cdylib` workspaces depending on it; a probe loads both and compares
`TypeId` hashes) measured this:

| plugin graph | `TypeId` of the shared contract type |
| --- | --- |
| `a`: `shared` + `serde` | `0xe0e808b8beb8a14a` |
| `b`: `shared` + `serde` + `serde_json` | `0xe0e808b8beb8a14a` |
| `b` after adding `serde = { features = ["rc"] }` | `0xe86665e3057b98d5` |

Identical versions and identical source. **One extra feature on a transitive
dependency, enabling nothing the contract itself uses, changed the contract
crate's `TypeId`.** That is not a corner case to document away: it is the default
consequence of two Cargo workspaces resolving features differently, which for
third-party plugins is not controllable at all. A `dyn` value therefore cannot be
the contract.

This is the same wall as ADR 0005, one level up. There, GPUI event listeners could
not fire inside a plugin because `Any` downcasting compares `TypeId`s; here, the
same mechanism would make a published service unusable by exactly the plugins it
was published for.

## Decision

**1. The narrow ABI gains a contract bus; contracts cross as bytes.**

```rust
pub struct HostContext { pub api_version: u32, pub bus: ContractBus }
pub type PluginEntry = unsafe extern "C" fn(*const HostContext) -> *const PluginRegistration;
```

The entry symbol becomes `brucebox_plugin_entry_v2` and API version goes to `2`.
The bus is four `extern "C"` calls — `publish`, `read`, `subscribe`,
`unsubscribe` — over named topics, with `Bytes` ranges and an opaque `user`
pointer the host installed per plugin (which is how a publication is attributed
without the plugin naming itself). Topics are strings; payloads are bytes; the
limit is 1 MiB.

**2. The bus is a *value store*, not a message queue.**

- One value per topic, last write wins, and the store records the publisher.
- Publishing byte-identical content is not a change: it neither updates nor
  notifies. A plugin that polls can publish every tick and stay quiet.
- A publisher is not called back for its own publication.
- Subscribers run on the publishing thread, after the registry lock is released,
  and receive borrowed bytes valid only during the call.
- `read` is what a late consumer uses; there is no late delivery.

That last property is what makes "Git is not installed" a normal state rather
than an error: nothing was ever published, so `read` returns nothing.

**3. Typed contracts are a shared crate, versioned in the topic name.**

`plugins/git-contract` holds `RepositorySnapshot`, `Change`, `Selection` and the
typed helpers. Both plugins depend on it; **neither depends on the other**, and
the crate depends on neither. The version is part of the topic
(`brucebox.git.repository/0.1`), so a consumer built against `0.1` never reads a
`0.2` payload and misinterprets it — it simply finds nothing.

JSON is the encoding, chosen because it is boring, debuggable and already in the
dependency graph. A contract that needs binary efficiency would encode its own
bytes; the bus does not care.

**4. Agent Chat has no hard dependency on Git.**

It subscribes in `apply`, primes from `read`, and shows "Git: no plugin publishing
a repository" when nothing is there. `F2` attaches whatever is currently
published: the repository summary, the changed files, and the selected file's
diff when one is selected. A manifest-level `requires` would have made the chat
panel unavailable without Git, which is worse for the user and would have hidden
exactly the failure-isolation behaviour stage 7 is supposed to prove.

**5. Git reads repositories through the `git` command.**

`git status --porcelain=v1`, `git diff --no-color`, `git rev-parse`. The question
this slice answers is whether two independent plugins can cooperate; `libgit2`
would answer it more slowly. A Git-library implementation can replace
`plugins/git/src/repository.rs` without touching the panels, the store or the
contract.

## Consequences

- Cross-plugin data flow is part of the narrow, versioned ABI rather than the
  ergonomic SDK. A third-party plugin gets the same contract mechanism the
  first-party ones use, with no host change.
- Contract types are ordinary Rust types; only their *transport* is untyped. The
  `TypeId` mismatch cannot surface as a runtime failure, because no `dyn` value
  crosses.
- The bus lives for the process. Topics are shared by every loaded plugin, which
  is what lets a publisher start before a consumer and vice versa.
- API version 2, `brucebox_plugin_entry_v2`, and a rebuilt plugin library are
  required; a version 1 package is refused with both versions named.

## Verified

Two packaged plugins, installed in the user plugin root, hosted on Xvfb:

1. Both load as separate `cdylib`s; Activity shows `brucebox.agent-chat`,
   `brucebox.git` and the host's own workspace plugin active.
2. The Chat panel shows the live line `Git · main · 0b08348 · 30 changed files`
   published by the Git library, with no import of it.
3. Repository panel: branch, head, 30 changed files with status letters.
4. Selecting a file with `j`/`k` makes the Diff panel show that file's real diff,
   colourised; the selection is published on `brucebox.git.selection/0.1`.
5. `F2` in Chat attaches a "Git context" message *including* the selected diff —
   confirmed in the saved workspace document, which contains
   `Diff of crates/brucebox-plugin-api/src/lib.rs` and the added line
   `+use std::ffi::c_void;`.
6. Floating the Diff panel and attaching again still works: the contract does not
   depend on placement.
7. Disabling the Git plugin and restarting: Chat renders
   "Git: no plugin publishing a repository" and keeps its saved conversation,
   while both Git panels restore as "Panel unavailable — provided by plugin
   `brucebox.git`". Failure is isolated, and nothing else fails with it.

## Known limits

- **Arrow keys do not reach plugin panels.** The dock engine consumes them before
  a plugin panel's forwarded input; `a`, `Enter` and `F2` arrive normally. The
  Repository panel therefore walks its list with `k`/`j` as well as the arrows,
  and panel authors should not rely on navigation keys. Fixing this means
  intercepting arrows before the dock — a host change, deliberately not part of
  this stage.
- The bus is a last-value store: no request/response, no queue, no delivery for a
  late consumer. A contract that needs a round trip has to model it as two topics.
- No payload negotiation: a mismatch is silence, not a diagnostic. The version in
  the topic name is the whole compatibility story.
- The Git poller re-reads the repository every two seconds; there is no file
  watcher and no incremental status.
- Diffs are cut at 256 KiB and snapshots at 500 changes; both report truncation
  rather than silently dropping data.
- Contracts are trusted input: a plugin can publish malformed JSON on any topic.
  The consumer's decode failure is a no-op, but this is not a sandbox (SPEC §5.6).
