# H09-hook · Hook Pipeline

> **Status**: 🚧 Not implemented · See ROADMAP.md (Sprint 6)

## Role in Harness

Lets external policy code observe and gate Brain's actions at fixed
lifecycle points (before prompt build, before tool dispatch, after model
completion, on turn end). Hooks are **Brain-side**: they run inside the
Harness, on the same task as H01.

## Inviolable purity rule

Hooks **may not perform I/O**. Concretely, a `HookHandler` may:

- Read the in-flight turn's state (prompt, tool call args, model output)
- Return one of: `Allow`, `Modify(new_value)`, `Reject(reason)`
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

```rust
// TODO (Sprint 6): define HookHandler trait in cogito-protocol.
//
// The trait must NOT take a generic I/O capability. Inputs are owned
// values (prompt slice, tool call snapshot); outputs are pure
// HookDecision values. No &mut World, no impl AsyncWrite, no clients.
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
2. A hook returning `Reject` produces a `TurnFailed` event with the
   hook's reason string and aborts the turn.
3. A hook returning `Modify` produces a `HookModified` event recording
   the before/after; the modified value flows to the next state.
4. A panicking hook is treated as `Reject("hook panic: …")` — Brain does
   not crash because a hook author wrote `unwrap()`.

## Open design questions

- Hook ordering: declarative priority vs registration order? Lean
  toward declarative priority so config files can reason about it.
- Should hooks be allowed to insert *new* events (e.g. a "redaction"
  event)? Initial answer: no — only `HookModified`. Revisit if a
  concrete need appears.

## Testing strategy

- Unit: pure hook decisions against synthetic turn state.
- Integration: a turn with a `Reject` hook produces `TurnFailed` and a
  matching event in the log; a turn with a `Modify` hook produces
  `HookModified` and the downstream state uses the modified value.
- Chaos: hook panics do not propagate to H01.

## References

- ARCHITECTURE.md §"Brain / Hands / Session boundaries"
- ADR-0004 §6 (Hooks are Brain-side policy only)
- AGENTS.md §"Inviolable design principles"
