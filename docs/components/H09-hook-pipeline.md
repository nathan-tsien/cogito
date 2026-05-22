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

## Dependencies

Calls (out):
- `cogito-protocol::ConversationEvent` (to emit gate-decision events via H02)

Called by:
- H01 Turn Driver, at lifecycle points (`pre_prompt`, `pre_dispatch`,
  `post_model`, `post_turn`, `on_error`).

Never called by H02–H11 directly; H09 is invoked by H01 only.

### `pre_prompt` lifecycle position

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
