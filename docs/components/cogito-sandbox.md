# cogito-sandbox — Hands (internal primitive)

The home of the **`CommandExecutor` seam**: the abstraction for "run a
subprocess in the policy-selected environment." Whether execution is
isolated is decided by the concrete implementation injected at runtime, not
by the tool that calls it. v0.1 ships one implementation, `DirectExecutor`,
which is **not a security boundary**. Not a Harness component (no
H-number); a Hands-internal primitive invisible to the Brain (ADR-0004).

Authoritative rationale: [ADR-0027](../adr/0027-command-executor-seam-and-builtin-scope.md).
Spec: `docs/superpowers/specs/2026-05-29-core-tools-bash-webfetch-design.md`.

## Position

```
Brain (cogito-core::harness)        sees ToolProvider only
  |  H08 dispatch
  v
Tool layer (ToolProvider impls)     e.g. cogito-jobs::BashTool
  |  tool-internal call
  v
CommandExecutor (cogito-protocol)   <- the seam (Layer 2)
  |  build_executor injects an impl
  v
cogito-sandbox::DirectExecutor      runs `sh -c` on the host
```

Two layers (ADR-0027 §"Two-layer model"):

- **Layer 1, Tool abstraction** — `ToolProvider`, the only thing H08 sees.
- **Layer 2, `CommandExecutor`** — beneath a tool, the subprocess
  primitive. **H08 does not know it exists.** A tool that needs to spawn a
  process (e.g. `bash`) holds an `Arc<dyn CommandExecutor>` injected at
  construction and decides internally how to use it.

The trait is defined in `cogito-protocol`, not here, so tools can depend on
the seam without depending on `cogito-sandbox`. It is a runtime-only trait:
never serialized, not part of the cross-language wire contract, so it does
not bump `SCHEMA_VERSION`.

## The seam: `CommandExecutor` (in `cogito-protocol`)

```rust
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn run(&self, spec: CommandSpec, ctx: ExecCtx)
        -> Result<CommandOutcome, CommandError>;
}
```

- `CommandSpec { command, cwd, timeout, max_output_bytes }` — `env` policy
  and the root directory are construction-time concerns (on
  `SandboxConfig`), deliberately absent from the per-call spec to keep the
  call surface minimal.
- `CommandOutcome { stdout, stderr, exit_code, timed_out, truncated }` — a
  **non-zero exit code is NOT an error**; it is a normal outcome with
  `exit_code = Some(n)`. A **timeout is NOT an error** either; it is an
  outcome with `timed_out = true` plus whatever output was captured before
  the kill.
- `CommandError { Spawn(String), Cancelled }` — only spawn failure and
  cooperative cancellation surface as errors. `#[non_exhaustive]` so callers
  must handle future variants gracefully.

## `DirectExecutor` (v0.1, the only implementation)

`cogito-sandbox::DirectExecutor`, constructed from `DirectConfig`:

- Runs `sh -c <command>` via `tokio::process::Command` (Linux target;
  Windows is a TODO).
- **cwd resolution**: `spec.cwd` if absolute; a relative `spec.cwd` is
  joined under the configured `root`; `None` uses `root`.
- **env policy**: inherits the parent process environment by default; if
  `inherit_env = false`, calls `env_clear()` first.
- **Output capture**: stdout/stderr are piped and drained concurrently on
  spawned tasks (the pattern validated by `cogito-jobs::run_tests`).
- **Termination race**: a `tokio::select!` races `child.wait()` against
  `ctx.cancel.cancelled()` and `tokio::time::sleep(spec.timeout)`. On cancel
  or timeout the child is killed; cancel returns `CommandError::Cancelled`,
  timeout returns a `CommandOutcome` with `timed_out = true` and
  `exit_code = None`.
- **Truncation**: each stream is head/tail-truncated to `max_output_bytes`
  (head and tail kept, middle elided); `truncated` is set when either
  stream was clipped.
- **Not a security boundary**: no namespaces, seccomp, or chroot. cwd jail
  and output/timeout limits only. Command admission (blocking `rm -rf /`)
  is an H09 hook concern, not this layer's.

## `SandboxConfig` + `build_executor` (the sole kind-dispatch)

```rust
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SandboxConfig {
    Direct(DirectConfig),   // v0.1: the only tag
}

pub struct DirectConfig {
    pub root: PathBuf,       // default "." (process cwd)
    pub inherit_env: bool,   // default true
}

pub fn build_executor(cfg: &SandboxConfig)
    -> Result<Arc<dyn CommandExecutor>, SandboxError>;
```

`build_executor` is the **only place in the workspace that matches on the
sandbox `kind`** — per CLAUDE.md's tagged-config-factory rule, the dispatch
lives in the owning crate. Surfaces (`cogito-cli`, `cogito-tui`) call it
once and receive a trait object, then inject it into `BashTool::new(...)`.
`DirectConfig::default()` is `root = "."`, `inherit_env = true`;
`SandboxConfig::default()` is `Direct(DirectConfig::default())`.

Configured under `[tools.sandbox]` in `cogito.toml`; see
[`docs/configuration/overview.md`](../configuration/overview.md) §`[tools]`.

## Spawn-point ownership

`CommandExecutor` is the intended single funnel for subprocesses cogito
itself spawns. v0.1 wires only `bash`; the rest are recorded as known
current state / future work (full per-row reasoning in spec §3.2–3.5 and
ADR-0027):

| Spawn point | Through `CommandExecutor`? | Notes |
|---|---|---|
| `bash` tool | Yes (this change) | First consumer of the seam. |
| `run_tests` tool | No (raw `tokio::process` today) | Working code; convergence is optional later dedup. |
| MCP stdio server connect | No (inside rmcp, one-shot at connect) | Not per-call; a known boundary, especially for SaaS. |
| skill scripts (today) | Yes (via `bash`) | ADR-0023 B-defer: scripts are data run with `bash`. |
| skill scripts (future) | Should be | A dedicated exec path should funnel through the seam. |

## Evolution (v0.4)

The seam is where production isolation slots in without touching the Brain
or any tool:

- **ADR-0012** — sandbox lifecycle, resource limits, real isolation
  (namespaces / cgroups / remote). New `SandboxConfig` tags (e.g.
  `LocalJail`, `Remote`) add `match` arms in `build_executor`; surfaces are
  untouched.
- **ADR-0013** — credential isolation (per-tenant env / secret scoping).
- The MCP stdio child-process gap (above) may be closed by a future ADR
  that routes rmcp's command spawn through `CommandExecutor` too.

## Testing strategy

- **Contract**: `cogito-protocol::test_support::contract_command_executor`
  is a shared suite every `CommandExecutor` impl must pass — success exit
  code, non-zero exit code, timeout sets `timed_out`, cancel kills the
  child, output truncation.
- **Unit**: `DirectExecutor` runs the contract against real `sh -c`.

## References

- [ADR-0027](../adr/0027-command-executor-seam-and-builtin-scope.md) (this
  seam + builtin scope)
- [ADR-0004](../adr/0004-brain-hands-session-boundaries.md) (layering)
- ADR-0012 / ADR-0013 (v0.4 isolation + credentials)
- [H08 Tool Dispatcher](H08-tool-dispatcher.md) (why H08 does not see this)
- `crates/cogito-protocol/src/command.rs`,
  `crates/cogito-sandbox/src/{config,executor}.rs`
