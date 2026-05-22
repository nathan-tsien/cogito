# H09-hook · Hook Pipeline

> **Status**: Sprint 5 (2026-05-22): real `HookHandler` trait + 2 example hooks
> + 5 lifecycle wirings + panic catch + `MetricsRecorder`. `HookDecision::Modify`
> deferred (see §"Open design questions").

## Role in Harness

Lets external policy code observe and gate Brain's actions at fixed
lifecycle points (before prompt build, before tool dispatch, after model
completion, on turn end). Hooks are **Brain-side**: they run inside the
Harness, on the same task as H01.

## Inviolable purity rule

Hooks **may not perform I/O**. Concretely, a `HookHandler` may:

- Read the in-flight turn's state (prompt, tool call args, model output)
- Return one of: `Allow`, `Reject(hook_name, reason)`
- Emit a `ConversationEvent` via the Step Recorder (allowed because the
  Step Recorder writes through the Session contract, not the hook itself)

A `HookHandler` **may not**:

- Make network calls
- Touch the filesystem
- Spawn processes
- Call third-party services synchronously inside the hook body

If a policy needs side effects (remote audit log, remote content
classifier, anything stateful outside Session), it must enqueue the work
through a `ToolProvider` or `JobManager`. The hook itself stays pure.

**Why**: a side-effecting hook is an undocumented Hand living inside
Brain. It reintroduces the Brain→world coupling that ADR-0004 forbids.
The compiler cannot catch this (hooks are trait objects); the rule has
to be enforced in code review and by the `HookHandler` trait signature
(no async I/O affordances).

## Interface

The canonical types live in `cogito-protocol::hook`. All four are
re-exported at the crate root.

### `HookLifecyclePoint`

One of the five points at which the pipeline fires. Serialised as
`snake_case` in `EventPayload::HookRejected`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HookLifecyclePoint {
    PrePrompt,
    PreDispatch,
    PostModel,
    PostTurn,
    OnError,
}
```

### `HookDecision`

The value returned by every `HookHandler` method that can gate the
pipeline. `#[non_exhaustive]` so `Modify` can be added additively in a
future sprint (see §"Open design questions").

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HookDecision {
    /// Continue normal pipeline flow.
    Allow,
    /// Abort the pipeline with the given reason.
    Reject {
        /// Name of the hook (from `HookHandler::name()`).
        hook_name: String,
        /// Human-readable rejection reason.
        reason: String,
    },
}
```

### `HookHandler`

Brain-side policy gate. All methods MUST be free of I/O. Default
implementations return `Allow` / no-op so implementors override only
the lifecycle points they care about.

```rust
pub trait HookHandler: Send + Sync {
    /// Stable identifier used in events and metrics.
    /// SHOULD be kebab-case and unique within a deployment.
    fn name(&self) -> &str;

    fn pre_prompt(&self, _input: &ModelInput) -> HookDecision {
        HookDecision::Allow
    }

    fn pre_dispatch(
        &self,
        _call_id: &str,
        _tool_name: &str,
        _args: &serde_json::Value,
    ) -> HookDecision {
        HookDecision::Allow
    }

    fn post_model(&self) {}
    fn post_turn(&self) {}
    fn on_error(&self, _reason: &str) {}
}
```

### `HookProvider`

Aggregation surface used by the runtime to build a
`CompositeHookPipeline` from one or more sources (Sprint 5: built-ins;
v0.2 Plugin: plugin-bundled hooks).

```rust
pub trait HookProvider: Send + Sync {
    fn list(&self) -> Vec<Arc<dyn HookHandler>>;
}
```

A `HookProvider` is the aggregation surface used by the Runtime when
multiple sources contribute hooks. Sprint 5 ships built-in providers
only (constructed directly in code). v0.2 Plugin (ADR-0021) will let
plugins declare hooks in `.cogito-plugin/plugin.toml`; the plugin
loader exposes each plugin as a `HookProvider`, and the Runtime
combines them via `CompositeHookPipeline::with_handlers(provider_a.list()
.chain(provider_b.list()).collect())`. Plugin-bundled hooks get a
namespaced name `<plugin_id>:<hook_name>` (per ADR-0021); the
`HookHandler::name()` method is responsible for returning the
namespaced form.

Within a single session, all providers' hooks share the same
`MetricsRecorder` (the one held by `SessionState.metrics`) — see the
unified-Arc invariant documented in `TurnDeps.metrics`.

## Dependencies

Calls (out):
- `cogito-protocol::ConversationEvent` (to emit gate-decision events via H02)

Called by:
- H01 Turn Driver, at lifecycle points (`pre_prompt`, `pre_dispatch`,
  `post_model`, `post_turn`, `on_error`).

Never called by H02–H11 directly; H09 is invoked by H01 only.

## Component relationships

### Lifecycle timeline

```
TurnStarted
   │
   ├─ ContextManaged stage (H10 / H11 / H04 / H05) ──┐
   │                                                  ▼
   │                                          [pre_prompt]   ← gate (Allow / Reject)
   │
   ├─ ModelCalling stage (H06 stream) ────────────────┐
   │                                                  ▼
   │                                          [post_model]   ← observation
   │
   ├─ ToolDispatching loop (per tool call)
   │   ├─ H07 ToolResolve
   │   ├─ [pre_dispatch]                              ← gate (Allow / Reject)
   │   └─ H08 Dispatch + H02 Record
   │
   ├─ Completed terminal       → [post_turn]          ← observation
   ├─ Paused terminal          → [post_turn] (TODO sprint-6, see Open design questions)
   └─ Failed terminal (5 sites) → [on_error]          ← observation
```

### Hook point mapping

| Point | FSM transition | Triggering file | Components already run | Observable state | Decision |
|---|---|---|---|---|---|
| `pre_prompt` | `ContextManaged → PromptBuilt` | `transitions/context_managed.rs` | H10 / H11 / H04 / H05 | `&ModelInput` (complete prompt + tool surface) | Allow / Reject |
| `pre_dispatch` | inside ToolDispatching loop | `transitions/tool_dispatching.rs` | H06 / H07 | `&call_id`, `&tool_name`, `&args` (per invocation) | Allow / Reject |
| `post_model` | after H06 stream completion | `transitions/model_calling.rs` | H06 | (no payload — observation only) | observation |
| `post_turn` | `* → Completed` | `transitions/model_completed.rs` | full turn | (no payload) | observation |
| `on_error` | `* → Failed` (5 sites — see below) | various | varies by path | `reason: &str` | observation |

### Wired `on_error` sites

`on_error` fires immediately before each `TurnState::Failed { ... }` construction. The 5 current sites:

- `transitions/context_managed.rs:104` — `pre_prompt` hook rejection
- `transitions/prompt_built.rs:54` — H06 gateway open failure
- `transitions/model_calling.rs:60` — H06 stream error
- `transitions/model_completed.rs:113` — max-consecutive-tool-errors abort
- `turn_driver/mod.rs:101` — resume re-validation failure in `enter_turn`

### `pre_prompt` lifecycle position (post-context-management)

`pre_prompt` fires at the **end** of the prompt-build phase, after the full
five-component sequence H10 → H11 → H04 → H05 has produced a complete
`ModelInput`. Hooks therefore observe the **post-context-management**
prompt — they see compaction replacements, system-prompt injections, and
tool-filter overrides from H11 as already-applied. A hook that rejects on
"prompt mentions sensitive data" reasons against the final, ready-to-ship
prompt, not the raw history.

A future `pre_context` / `post_context` hook lifecycle point (around H11
itself) is an open question for the Context Management initiative
(ADR-0008). It is **not** in this design as of the 2026-05-19 PR #6
amendment. See `docs/components/H01-turn-driver.md` §"Init →
ContextManaged → PromptBuilt sequence" for the canonical walkthrough.

## Why these five points (and not more)

The five-point surface is a deliberate minimum, not an end state. For
reference:

| Project | Hook count | Coverage |
|---|---|---|
| cogito v0.1 | 5 | prompt gate, tool gate, model observe, turn observe, error observe |
| Codex CLI | 10 | adds SessionStart/Stop, PreCompact/PostCompact, SubagentStart/Stop, PermissionRequest, PostToolUse |
| Claude Code | 31 | adds elicitation, file/cwd/worktree events, batch tool, permission-denied, plus per-stage failure variants |

### Design rationale

- Each lifecycle point ties a fixed cost: FSM wiring + a `wrap_*` panic-
  catch helper + a `CompositeHookPipeline` iteration method + metrics +
  one or more integration tests. Adding a point without a concrete
  consumer use case is a YAGNI violation.
- `HookLifecyclePoint` is `#[non_exhaustive]`, so new variants are
  additive — they do NOT require a `SCHEMA_VERSION` bump and do NOT
  break existing `HookHandler` impls (the trait method has a default).
- Two points (`pre_prompt`, `pre_dispatch`) are gates; three
  (`post_model`, `post_turn`, `on_error`) are observation-only. Gating
  is expensive (every consumer must reason about Allow/Reject in their
  policy); observation is cheap. Keeping the gate count low until we
  see real consumer needs is intentional.

### Semantic note on `pre_prompt`

cogito's `pre_prompt` fires AFTER prompt composition is complete — the
hook sees `&ModelInput` (the full assembled prompt + tool surface), not
the raw user input. Codex / Claude Code's `UserPromptSubmit` fires
earlier (raw user string, before composition). The two are different
tools:

- `pre_prompt` (cogito): "this prompt is about to be sent; should it?"
- `UserPromptSubmit` (Codex / CC): "the user typed this; rewrite or
  reject before composition begins?"

A `pre_compose` / earlier-stage variant could be added later if a
prompt-rewriting use case surfaces — see §"Future expansion" below.

## Future expansion

The roadmap reserves additional lifecycle points for upcoming sprints.
Adding them is additive (no SCHEMA_VERSION bump, no breaking
`HookHandler` change — defaults absorb the new methods).

### Planned

| Point(s) | Sprint | Trigger |
|---|---|---|
| `pre_compact` / `post_compact` | Sprint 6 (Context C2) | wraps the Compactor decision step in H11 |
| `post_turn` on `Paused` terminal | Sprint 6 (async jobs) | new code path; `TurnState::Paused` becomes reachable |
| `subagent_start` / `subagent_stop` | Sprint 11 (Subagent S2) | wraps `BrainSpawner::spawn(...)` invocations |

### Considered but not scoped

- `post_tool_use` — observation hook after each tool dispatch completes.
  Costs: noisier event log if every tool emits, plus careful sequencing
  with `H02 record_tool_result`. Adopt when a concrete audit / retry
  use case surfaces.
- `session_start` / `session_end` — needed for plugin-bundled hooks
  that hold session-scoped state. Defer until Sprint 12 (Plugin)
  reveals concrete needs.
- `permission_request` — gating UI-driven user approval. cogito v0.1
  has no permission layer; this point lands with the eventual
  permission system, not on a fixed sprint.

### Recipe: adding a new lifecycle point

When a sprint adds a new point, the additive change is:

1. Add variant to `HookLifecyclePoint` (`cogito-protocol::hook`).
2. Add a trait method to `HookHandler` with a default `Allow` / no-op
   body — existing impls compile unchanged.
3. Add a `wrap_<point>` helper in
   `cogito-core::harness::hooks::panic_catch`.
4. Add an iteration method on `CompositeHookPipeline` that times each
   call, records metrics, and short-circuits (for gate points) or
   continues (for observation points).
5. Wire the call site in the appropriate `transitions/*.rs` (or other
   transition module) BEFORE the state change it observes.
6. If the point gates, add a `HookRejected` event emission via
   `StepRecorder::record_hook_rejected` before the followup error /
   failure event (ADR-0007 causality invariant).
7. Cover with one unit test (composite) + one integration test
   (end-to-end).

This recipe is the operational interpretation of the
`#[non_exhaustive]` annotation: the surface grows by addition, never by
mutation.

## Critical invariants

1. Hook execution is synchronous and bounded (target P99 < 5ms per hook
   per gate point; measured in Experiment E06).
2. A hook returning `Reject` produces a `HookRejected` event immediately,
   then a `TurnFailed` event with the hook's reason string, and aborts
   the turn.
3. v0.1 ships `Allow` / `Reject` only. `Modify` is deferred to a future
   sprint — see §"Open design questions". The `HookDecision` enum is
   `#[non_exhaustive]` so adding `Modify` is additive.
4. A panicking hook is treated as `Reject("hook panic: …")` — Brain does
   not crash because a hook author wrote `unwrap()`.

## Open design questions

- **`post_turn` on `Paused` terminal**: `post_turn` is currently wired
  only for the `Completed` terminal (fired inside `model_completed.rs`
  before the FSM returns `TurnState::Completed`). The `Paused` terminal
  will be wired in Sprint 6 alongside the JobManager / async job path
  (Context Management, C2). Until that wiring lands, the `Paused` match
  arm in `turn_driver/mod.rs` carries a `TODO(sprint-6)` comment as a
  reminder. Cross-reference Sprint 6 (Context Management) and
  `docs/superpowers/plans/2026-05-22-sprint-5-hook-pipeline.md`.

- **`HookDecision::Modify`**: originally proposed to allow a hook to
  rewrite the in-flight value (prompt slice, tool call args) before the
  pipeline continues. The variant would produce a `HookModified` event
  recording the before/after, and the modified value would flow to the
  next state. Deferred because no concrete consumer use case has surfaced
  yet; the `#[non_exhaustive]` annotation keeps the door open without
  committing to an API shape prematurely.
- Hook ordering: declarative priority vs registration order? Lean
  toward declarative priority so config files can reason about it.
- Should hooks be allowed to insert *new* events (e.g. a "redaction"
  event)? Initial answer: no — only `HookModified`. Revisit if a
  concrete need appears.

## Testing strategy

- Unit: pure hook decisions against synthetic turn state.
- Integration: a turn with a `Reject` hook produces `TurnFailed` and a
  matching event in the log; a turn with a panicking hook is isolated and
  does not crash the session loop.
- Chaos: hook panics do not propagate to H01.

## References

- ARCHITECTURE.md §"Brain / Hands / Session boundaries"
- ADR-0004 §6 (Hooks are Brain-side policy only)
- AGENTS.md §"Inviolable design principles"
- `docs/superpowers/plans/2026-05-22-sprint-5-hook-pipeline.md`
