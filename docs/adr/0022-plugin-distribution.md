# ADR-0022: Plugin distribution (git) — finalize

## Status

Proposed (draft, v0.3/v0.4). Finalizes the v0.2 placeholder, resolving the
four open questions it carried (submodules, Git-LFS, self-hosted/SSH origins,
lock-conflict resolution) and the two implementation choices it deferred
(git2 vs shell-out, content-hash crate). Awaiting human ratification — not yet
Accepted.

Captures the v0.3 tier of the **P4 tiered plugin distribution** plan from the
[2026-05-22 roadmap rebalance spec](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
(§2.4 + §3.3). v0.2 shipped **local path only** (ADR-0021, Accepted); v0.3 adds
**git fetch + lock file** (this ADR); marketplace / HTTP index / signing remain
v0.6+ (future ADR).

## Context

ADR-0021 shipped `cogito-plugin` with a stable manifest
(`.cogito-plugin/plugin.toml`) and a loader whose only distribution mechanism is
a local `path`. `PluginEntry` (in `crates/cogito-plugin/src/lib.rs`) today has a
**required** `path: String`, an `enabled` flag, and `artifact_overrides`. The
loader resolves entries into a `PluginContributions { skill_roots, mcp_servers }`
that the caller folds into the existing `SkillRegistry` (Plugin scope) and
`build_mcp_provider`. v0.3 attaches a fetch + pin + cache layer **underneath**
that loader: once a git plugin's source is materialized on disk, the existing
ADR-0021 loader runs over it unchanged.

Distribution requirements for v0.3:

- A plugin may live in a **Git repository**, pinned by **tag or full commit
  SHA**. No floating refs (`main`, `HEAD`, branch names) in production
  configuration — they break reproducibility.
- The set of resolved versions across all `[[plugins]]` entries is recorded in
  a **lock file** (`cogito.lock`) checked into the consumer's repo. The lock is
  the source of truth at runtime.
- Sync is **explicit**: `cogito plugin sync` fetches/updates; `cogito chat` /
  `cogito serve` make **no** network calls.
- Failed fetches are **non-fatal at startup** if a cached copy is present and
  matches the lock; **fatal** if no cache.

Note on the motivating consumer (praxis): praxis brings its own
`ConversationStore` and its own gateway routing, so cogito's distribution layer
is **not** on praxis's critical persistence path. Git plugin distribution
matters for praxis only insofar as a SaaS deployment wants reproducible,
pre-warmable plugin caches across replicas (see §8). The design therefore keeps
the cache **content-addressed and process-local**, with no shared-state
assumption.

Out of scope for v0.3:

- HTTP marketplace indexes and `cogito plugin search` (deferred to v0.6+).
- Signing / signature verification (no current threat model; the lock's
  `content_hash` gives tamper-evidence, not provenance).
- Vendoring (consumer copies plugin source into its repo and points `path` at
  it) — orthogonal and always allowed.
- Mirror / cache-server protocols.

## Decision

### 1. Plugin source syntax in `cogito.toml`

`PluginEntry` gains an optional `git` group; `path` becomes optional. Exactly
one of `path` (ADR-0021) or `git` must be present per entry — having neither or
both is a **fatal** config error surfaced at deserialization / load.

```toml
[[plugins]]
git = "https://github.com/org/cogito-plugin-review.git"
rev = "v0.1.0"            # tag OR full 40-hex commit SHA; floating refs rejected

# Optional, mutually-exclusive auth (mirrors the MCP env-var pattern, ADR-0018):
token_env_var   = "GH_PAT"             # HTTPS PAT -> injected as basic auth
ssh_key_env_var = "PLUGIN_DEPLOY_KEY"  # path to a private key for SSH origins

# enabled / artifact_overrides from ADR-0021 still apply unchanged
```

`rev` validation: accepted iff it is a full 40-character hex SHA **or** a value
that resolves to an annotated/lightweight tag at sync time. A bare branch name,
`HEAD`, `latest`, or a short SHA is rejected with a config error naming the
entry. (`cogito plugin update`, §4, is the supported way to move a pin forward.)

The serde shape stays additive: `path` moves from required to
`Option<String>`, `git`/`rev`/`token_env_var`/`ssh_key_env_var` are new
`Option` fields. No `SCHEMA_VERSION` impact (the `[[plugins]]` table already
omits `deny_unknown_fields`, per ADR-0021 §7). `PluginEntry` remains owned by
`cogito-plugin`; `cogito-config` continues to aggregate it.

### 2. git2 crate vs shell-out — decision: shell out to the host `git`

cogito **shells out to the host `git` binary** rather than linking the `git2`
(libgit2) crate. Rationale, weighted for this repo's constraints:

- **Transport coverage for free.** SSH origins, self-hosted GitLab/Gitea,
  corporate HTTPS proxies, custom `~/.ssh/config` host aliases, credential
  helpers, and Git-LFS (§6) all work exactly as the operator's existing `git`
  is already configured. libgit2's SSH support depends on optional `libssh2`
  linkage and does not honor `ssh_config`; matching the host environment would
  mean re-implementing a lot of git's edges.
- **Smaller, safer dependency surface.** `git2` pulls a C library
  (libgit2/libssh2/openssl) into a workspace whose lint floor is
  `unsafe_code = "forbid"` and `RUSTFLAGS=-Dwarnings`. Shelling out keeps the
  Rust side pure and the build portable.
- **Distribution is rare and explicit.** Fetches happen only at
  `cogito plugin sync`, never on the `cogito chat` hot path, so the
  process-spawn cost is irrelevant and the ergonomic loss of not having an
  in-process object database does not matter.

Concretely the sync path runs a fixed, non-interactive command sequence
(no porcelain parsing of free-text where a plumbing form exists):

```
git -c protocol.version=2 clone --filter=blob:none --no-checkout --depth … <url> <tmp>
git -C <tmp> fetch --depth 1 origin <rev>          # tag or SHA
git -C <tmp> -c submodule.recurse=false checkout --detach <resolved-sha>
git -C <tmp> rev-parse HEAD                          # -> resolved_commit
```

Each invocation runs with `GIT_TERMINAL_PROMPT=0` and a per-fetch timeout so a
hung auth prompt becomes a clean error, not a stuck process. `token_env_var` is
injected via an ephemeral `-c http.extraHeader=Authorization: …` or
`http.<url>.extraHeader`; `ssh_key_env_var` is wired through
`GIT_SSH_COMMAND="ssh -i <key> -o IdentitiesOnly=yes"`. Secrets are read from
the named env vars at sync time and never written to the lock or the cache.
`git` is a **runtime** prerequisite for git-distributed plugins only; a config
using `path`-only plugins still needs no `git` (a missing `git` binary is fatal
only when a `git` entry is actually resolved).

### 3. Submodules — decision: ignored (flat tree)

A git plugin is fetched as a **flat tree**: submodules are **not** recursed.
The checkout uses `submodule.recurse=false` and never runs
`git submodule update`. A plugin author who needs vendored code must commit it
into the plugin repo, not reference it via submodule.

Rationale: a submodule is a second, independently-pinned source whose SHA is not
captured by the plugin repo's own commit, which would silently widen the trust
boundary and the content the `content_hash` (§5) is meant to cover. If a
checked-out tree contains a populated `.gitmodules`, sync emits a **warning**
(the gitlinks remain empty directories); it is not an error, so a plugin repo
that merely *declares* submodules still loads.

### 4. CLI commands

- `cogito plugin sync` — for every `[[plugins]] git = …` entry: resolve `rev`
  to a commit, fetch if that commit's content is not already in the cache,
  verify/compute `content_hash`, then write/update `cogito.lock`. Idempotent:
  re-running with an unchanged manifest and a warm cache is a no-op that still
  rewrites the lock deterministically (stable entry ordering, §5).
- `cogito plugin sync --check` — resolve and compare against the committed
  lock **without** mutating it or the cache. Exit 0 if the lock is current
  (every entry present, `declared_rev` matches, and—when the cache is
  populated—`resolved_commit`/`content_hash` agree), non-zero with a diff
  otherwise. This is the one-line CI gate. `--check` performs network
  resolution only for entries whose pin is a moving tag; SHA-pinned entries
  with a cache hit verify offline.
- `cogito plugin list` — print declared plugins with their lock state:
  `plugin_id`, source (`path` or `git` url), `declared_rev`,
  `resolved_commit` (short), and cache status (cached / missing). No network.
- `cogito plugin update [<plugin_id> …]` — re-resolve the named entries' `rev`
  to the tag's *current* commit (the supported way to advance a pin after a
  maintainer moves a tag or to bump to a new tag edited in `cogito.toml`),
  fetch, and rewrite their lock rows. With no argument, updates all git
  entries. This is the **only** command permitted to change `resolved_commit`
  for an unchanged `declared_rev`.

### 5. Lock file: `cogito.lock` (TOML)

Auto-generated, committed alongside `cogito.toml`, never hand-edited.

```toml
# Auto-generated by `cogito plugin sync`. Do not edit by hand.
schema_version = 1

[[plugin]]
git             = "https://github.com/org/cogito-plugin-review.git"
declared_rev    = "v0.1.0"
resolved_commit = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"
resolved_at     = "2026-08-01T12:00:00Z"
content_hash    = "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
lfs             = false           # echoes the entry's LFS opt-in (§6)
```

- `schema_version = 1` from the outset; future shape changes bump it and ship a
  reader that errors clearly on unknown versions.
- `[[plugin]]` rows are written in a **stable, canonical order** (sorted by
  `git` url then `declared_rev`) so the file diffs cleanly regardless of
  `cogito.toml` ordering.
- `path` plugins (ADR-0021) are **not** recorded in the lock — they have no
  fetched, pinnable identity; only `git` entries appear.
- `content_hash` is the cache key (§7). It is **not** the git tree SHA: it is a
  deterministic digest of the *materialized plugin content* computed by cogito
  (§5a), so it survives re-fetches, re-clones, and shallow/full differences in
  the local object store.

#### 5a. Content hash — decision: `sha2` (SHA-256)

The content hash uses the **`sha2`** crate (pure Rust, already a common,
audited workspace-grade dependency; consistent with the `sha256:` prefix the
placeholder used and with the lock format above). The digest is computed over a
canonical serialization of the checked-out worktree **excluding the `.git`
directory**: walk files in sorted relative-path order, and for each file feed
its relative path, a separator, its byte length, and its bytes into one rolling
SHA-256. Symlinks are recorded by target string, not followed. The hex digest is
stored as `sha256:<hex>`. This makes the hash independent of clone depth, pack
layout, and filesystem mtimes, and identical across machines for identical
content.

Implementation note: a `git2`-free, pure-Rust hashing path keeps the digest
reproducible without depending on `git`'s object model, while §2's shell-out
handles only transport. Adding `sha2` and the `git` shell-out is an explicit,
pre-approved dependency change scoped to `cogito-plugin` (no new crate).

### 6. Git-LFS — decision: opt-in per entry, off by default

LFS-tracked blobs (e.g. a skill's large `assets/`) are **not** materialized by
default; the checkout leaves LFS pointer files in place. A plugin entry opts in
explicitly:

```toml
[[plugins]]
git = "https://…/heavy-plugin.git"
rev = "v2.0.0"
lfs = true        # run `git lfs fetch && git lfs checkout` during sync
```

When `lfs = true`, sync additionally runs `git -C <tmp> lfs fetch origin
<sha>` then `git lfs checkout`, and the `content_hash` (§5a) is computed over
the **smudged** (real) content. When `lfs = false` (default), the hash covers
the pointer files. A `git` lacking the `git-lfs` extension while an entry sets
`lfs = true` is a fatal sync error naming the entry. Off-by-default keeps the
common (pointer-free or pointer-tolerant) plugin fast and avoids surprising LFS
bandwidth on every sync.

### 7. Cache layout

```
~/.cache/cogito/plugins/
  <sha256-hex>/                # full content_hash, no prefix truncation
    .cogito-plugin/plugin.toml
    skills/…
    mcp.toml
    …
```

(Honors `XDG_CACHE_HOME` when set; `~/.cache` is the fallback.)

Keyed by the **full** `content_hash`, not by git URL or commit, so two entries
that resolve to byte-identical content share one cache directory. The directory
is the de-`.git`'d worktree only — no git metadata is cached, reinforcing that
the cache is read-only plugin content. Population is atomic: sync materializes
into a temp dir, computes the hash, then renames into place; a partial fetch
never leaves a half-populated `<hash>/`. The cache is process-local and
self-healing — deleting it and re-running `cogito plugin sync` rebuilds it from
the lock.

### 8. Runtime behavior (offline-capable)

`cogito chat` / `cogito serve` / any consumer process:

1. Read `cogito.toml` + `cogito.lock`.
2. For each `git` entry: look up its `resolved_commit` → `content_hash` in the
   lock, then load the cache dir `~/.cache/cogito/plugins/<content_hash>/`
   through the **existing ADR-0021 loader** (it sees a normal plugin
   directory and produces `PluginContributions` as today).
3. For each `path` entry: unchanged ADR-0021 behavior.
4. **No network calls at runtime**, ever.

Failure modes:

- Lock missing while any `git` entry is declared → **fatal** startup error
  instructing the operator to run `cogito plugin sync`.
- Lock present but a `git` entry's `content_hash` is absent from the cache, and
  runtime is forbidden to fetch → **fatal** startup error naming the plugin and
  pointing at `cogito plugin sync`.
- Lock present, cache hit, but the on-disk content's recomputed hash ≠ the
  lock's `content_hash` (tamper / corruption) → **fatal**; re-sync required.

This guarantees `cogito chat` is offline-capable as long as the cache is
populated, which is exactly what a multi-replica SaaS deployment wants: bake or
pre-warm `~/.cache/cogito/plugins/` from a committed lock, ship it in the
image / volume, and every replica loads identical plugin content with no
runtime egress. (The lock travels with the consumer's repo; the cache is the
materialization of that lock and can be rebuilt anywhere.)

### 9. Network-failure semantics during `sync`

- A single plugin's fetch failure is recorded in a startup-failure channel
  modeled on ADR-0018's `McpStartupFailure` (non-fatal, surfaced in a banner);
  sync continues with the remaining entries.
- If a failed entry already has a cache hit matching the lock, sync treats it as
  **satisfied** and does not fail on it — the goal is a usable cache, and a warm
  cache means the network round-trip was optional.
- If every git entry fails *and* none is satisfiable from cache → sync exits
  non-zero with per-entry reasons (network vs auth vs bad-rev), and the
  partial-failure exit code distinguishes "some unfetchable, others fine" from
  "total failure".
- `sync --check` never silently fetches-and-fixes; on mismatch it reports and
  exits non-zero so CI fails loudly rather than mutating the lock under CI.

### 10. Lock-conflict resolution — decision: always re-resolve, never merge

`cogito.lock` is a **generated artifact**, not a hand-maintained source of
truth, so cogito does **not** implement cargo-style "preserve oldest resolution,
update newest" merge logic. The contract:

- The committed lock is authoritative for **runtime** (§8) and for `--check`.
- For **writes** (`sync`, `update`), cogito re-resolves the affected entries
  from `cogito.toml` and rewrites their rows deterministically (§5 canonical
  ordering). It does not attempt to reconcile two divergent lock versions.
- A textual merge conflict in `cogito.lock` (two devs each ran `sync` on
  different `cogito.toml` edits) is resolved the same way any generated-file
  conflict is: take either side (or `cogito.toml`'s side), then run
  `cogito plugin sync` once to regenerate a clean, canonical lock; commit that.
  Because resolution is a pure function of `cogito.toml` + the remote refs, the
  regenerated lock is correct regardless of which conflict side was kept.

This avoids importing cargo's lock-merge machinery for a file whose every row is
recomputable, and matches the rebalance spec's "lock is the source of truth at
runtime, regenerable at author time" intent.

## Alternatives considered

- **Link `git2`/libgit2 instead of shelling out.** Rejected (§2): worse SSH /
  self-hosted / proxy / LFS coverage versus the host `git`, and it adds a C
  dependency under a `forbid(unsafe_code)` workspace, for a code path that runs
  only at explicit sync time.
- **Key the cache by git URL + commit SHA.** Rejected: would duplicate
  byte-identical content fetched under two URLs (fork, mirror) and ties the
  cache to git identity rather than the content the lock already hashes.
  Content addressing (§7) dedups and is verifiable.
- **Cargo-style lock merge (preserve-oldest / update-newest).** Rejected
  (§10): unjustified complexity for a fully regenerable artifact.
- **Recurse submodules / fetch LFS by default.** Rejected (§3, §6): both widen
  the trust boundary or the bandwidth footprint for the common plugin; both are
  available explicitly when actually needed (commit vendored code; `lfs =
  true`).
- **Fetch lazily at runtime on cache miss.** Rejected: violates the
  "`cogito chat` makes no network calls" / offline-capable invariant and the
  reproducibility goal. Sync is the only network boundary.

## Consequences

**Easier**:

- Plugins distribute over existing Git infrastructure — no new hosting,
  signing, or registry to operate in v0.3.
- Reproducible: the committed lock pins exact content; `--check` is a one-line
  CI gate.
- Offline / air-gapped and multi-replica SaaS: pre-warm
  `~/.cache/cogito/plugins/` from the lock and ship it; runtime never egresses.
- Self-hosted / SSH / proxied origins "just work" because the host `git` does
  the talking (§2).

**Harder**:

- `git` (and `git-lfs` for LFS plugins) becomes a runtime prerequisite for
  git-distributed plugins; this must be documented for consumers and CI images.
- Two new dependencies in `cogito-plugin` (`sha2`; a thin shell-out runner) and
  a new CLI command group; the resume/test matrix gains sync + cache cases.
- Consumers carry lock-file discipline (commit it; regenerate on conflict).

**Given up**:

- Floating tags (`main`, `latest`) and short SHAs — explicit, follows cargo's
  lockfile model; `cogito plugin update` is the sanctioned way to advance.
- Submodule and default-on LFS materialization (opt-in / vendor instead).
- In-process git object access (the shell-out trade-off).
- Marketplace discovery / `cogito plugin search` and signature verification —
  deferred to v0.6+.

## References

- ADR-0021 (plugin manifest + loader) — the manifest/loader this fetch layer
  sits beneath; `PluginEntry` / `PluginContributions` shapes
  (`crates/cogito-plugin/src/lib.rs`).
- ADR-0017 (config model) — `[[plugins]]` section, layered merge, additive
  serde evolution.
- ADR-0018 (MCP integration) — non-fatal-startup-failure channel + banner
  pattern reused in §9; env-var auth pattern reused in §1/§2.
- ADR-0028 (per-session provider injection) — how the loaded plugin set reaches
  a SaaS session per request.
- Rebalance spec:
  [`docs/superpowers/specs/2026-05-22-roadmap-rebalance-design.md`](../superpowers/specs/2026-05-22-roadmap-rebalance-design.md)
  §2.4 + §3.3.
- Cargo's git-source + `Cargo.lock` as the lockfile design reference.
