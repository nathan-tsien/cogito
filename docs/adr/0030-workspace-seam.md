# ADR-0030: `Workspace` seam — a rooted, sandboxable working tree

## Status

Accepted (2026-06-01). Phase 1 of the complete-skill-support design
(`docs/superpowers/specs/2026-06-01-complete-skill-support-design.md`).

This ADR locks the seam. The first increment ships the trait, its contract
suite, and the `LocalWorkspace` reference implementation. Consuming the seam
(injecting it via `ExecCtx`, migrating `read_file`, adding
`write_file`/`edit`/`glob`/`grep`/`list_dir`) follows additively.

## Context

Skills need a working tree: read a bundled `scripts/foo.py`, author an
intermediate file, run it, read its output. Today the only file tool is
`read_file`, which calls `tokio::fs` directly against the host filesystem
(`crates/cogito-tools/src/builtins/read_file.rs`). There is:

- no write/edit/list capability, so a skill cannot author files except via
  `bash` heredoc hacks;
- no redirection seam, so file I/O cannot be confined or relocated. Under
  the SaaS profile (Phase 3) a tenant's file operations must land in an
  isolated per-tenant tree, not the host FS. Direct `tokio::fs` calls make
  that impossible.

Decision needing resolution (spec §9 #2): **a dedicated `Workspace` seam vs.
reusing the planned v0.5 `StorageSystem`.**

`StorageSystem` (ADR-0009, v0.5) is a blob/URI store — durable, addressed by
URI, content-addressable. A skill's scratch tree is a different abstraction:
mutable, path-addressed, POSIX-ish, ephemeral-per-session. Conflating them
would force blob semantics onto a working tree (or vice versa) and couple
Phase 1 to a v0.5 deliverable.

**Decision: a dedicated `Workspace` seam.** A working tree and a blob store
are distinct concerns; each gets its own trait. They may share a backend
later (e.g. an object-store-backed workspace) without sharing an interface.

## Decision

### 1. New protocol trait `Workspace` (Hands seam)

In `cogito-protocol`, mirroring the existing `CommandExecutor` seam
(`#[async_trait]`, `Send + Sync`, `thiserror` errors). Brain sees only
`dyn Workspace`; concrete impls live in Hands crates and are wired by the
Runtime.

```rust
#[async_trait]
pub trait Workspace: Send + Sync {
    /// Absolute root directory this workspace is confined to.
    fn root(&self) -> &Path;
    /// Read the whole file at `path` as raw bytes.
    async fn read(&self, path: &str) -> Result<Vec<u8>, WorkspaceError>;
    /// Create or overwrite the file at `path`, creating parent dirs.
    async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), WorkspaceError>;
    /// Whether `path` exists.
    async fn exists(&self, path: &str) -> Result<bool, WorkspaceError>;
    /// Immediate entries of directory `path` (`""` = root).
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, WorkspaceError>;
    /// Remove the file at `path` (v0.1 does not remove directories).
    async fn remove(&self, path: &str) -> Result<(), WorkspaceError>;
}
```

`DirEntry { name: String, is_dir: bool }`. Bytes (not `String`) so binary
files round-trip; UTF-8 / size-cap policy stays at the tool layer (where
`read_file`'s 1 MiB cap already lives).

### 2. Path confinement is the contract

All `path` arguments are interpreted **relative to `root()`**. The
implementation MUST reject anything that escapes the root —
absolute paths and `..` components that climb above root — with
`WorkspaceError::PathEscapesRoot`. This is the property that makes the seam
sandboxable: a `SandboxWorkspace` (Phase 3) rooted in a tenant volume
confines all skill file I/O by construction, and `LocalWorkspace` confines
to the session cwd. Confinement is a trait-level invariant, asserted by the
contract suite, not left to each impl's discretion.

(Note: ADR-0029 surfaces a skill's *absolute* bundled-file root to the model.
Reconciling "model sees absolute skill paths" with "Workspace takes
root-relative paths" is a Phase-2/3 concern — when bundled files are
materialized into the workspace, the model will reference them by their
workspace-relative path. Phase 1 only locks the seam.)

### 3. `WorkspaceError`

`thiserror`, `#[non_exhaustive]`:

- `NotFound(String)` — no such path.
- `PathEscapesRoot(String)` — input resolves outside `root()`.
- `Io(String)` — any other I/O failure (stringified, like `CommandError`).

### 4. Contract suite

`cogito_protocol::test_support::contract_workspace` exposes
`pub async fn contract_*(ws: Arc<dyn Workspace>)` functions (write-then-read,
read-missing-is-not-found, path-escape-rejected, list, exists, remove), each
consumed by every `Workspace` impl's test — same pattern as
`contract_command_executor`. SQLite-vs-memory-style agreement (CLAUDE.md
"every contract has a contract test") applies once a second impl
(`SandboxWorkspace`) lands.

### 5. `LocalWorkspace` reference impl

Host-filesystem impl rooted at a configured directory (the session cwd for
the Local profile). Lives in `cogito-tools` (Hands) — the crate that already
hosts the file tools that will consume it and the `CompositeToolProvider`
utility; no new crate. `SandboxWorkspace` lands in the `cogito-sandbox` v0.4
redesign (Phase 3).

### 6. Injection (deferred to the next increment)

`Workspace` will be injected as `ExecCtx.workspace: Option<Arc<dyn Workspace>>`,
mirroring `brain_spawner` (ADR-0011): rebuilt per turn, swappable per session
(ADR-0028), `None` when unwired (file tools then return a structured
`ToolResult::Error`, as `delegate` does without a spawner). Tools read
`ctx.workspace` rather than calling `tokio::fs`, which is what lets the SaaS
profile redirect them. This ADR records the choice; the field + `read_file`
migration ship in the consuming increment to keep this PR a clean seam-only
change.

**Provisioning, scoping, lifetime, and the exec-cwd relationship are decided
in ADR-0031** (per-session ephemeral; `SessionSpec.workspace`; project cwd
locally / per-tenant sandbox root in SaaS; session root as the default exec
cwd).

## Consequences

**Easier**:
- File tools become redirectable: same tool, host tree locally, tenant
  sandbox tree in SaaS — no Brain change, just a different injected `dyn`.
- Path confinement is enforced in one place and contract-tested, instead of
  each tool re-deriving traversal guards.
- Phase 1 is decoupled from the v0.5 `StorageSystem`.

**Harder**:
- A second filesystem-ish trait alongside the future `StorageSystem`;
  consumers must know which to use (working tree vs. durable blob). Doc note
  in the spec covers it.
- `ExecCtx` gains a field in the next increment (additive; every
  construction site sets `None`, as with `brain_spawner`).

**Given up**:
- Nothing structural. Strict superset; no existing behavior changes until
  `read_file` is migrated (next increment), and that migration preserves its
  current contract (1 MiB cap, UTF-8 handling).

## Open questions

1. Absolute-path inputs: reject outright (chosen — `PathEscapesRoot`) vs.
   reinterpret as root-relative. Rejecting is safer and clearer; revisit if
   real skills demand absolute reads outside the tree (those are a different
   capability, not the working tree).
2. Directory removal / recursive ops: deferred. v0.1 `remove` is files-only;
   add `remove_dir` when a tool needs it.
3. `glob` / `grep`: built at the tool layer over `list` + `read`, or promoted
   to trait methods if a sandbox backend can do them more efficiently
   server-side. Start tool-layer.
4. Symlink policy: does `LocalWorkspace` follow symlinks that point outside
   root? Default deny (treat as `PathEscapesRoot` after canonicalization).

## References

- Complete-skill-support design §3, §5.2, §9 #2:
  `docs/superpowers/specs/2026-06-01-complete-skill-support-design.md`
- ADR-0027 — CommandExecutor seam (the pattern this mirrors)
- ADR-0028 — per-session provider injection (how `Workspace` swaps per tenant)
- ADR-0009 (planned, v0.5) — `StorageSystem` (the blob store this is
  deliberately *not* merged with)
- ADR-0029 — skill bundled-file path exposure (the absolute-vs-relative note)
- Code: `crates/cogito-protocol/src/command.rs` (error/trait style),
  `crates/cogito-protocol/src/exec_ctx.rs` (injection precedent),
  `crates/cogito-tools/src/builtins/read_file.rs` (the direct-`tokio::fs`
  call this seam will replace)
