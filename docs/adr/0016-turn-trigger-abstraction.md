# ADR-0016: Turn-trigger abstraction (`TurnTrigger`)

## Status

Accepted (2026-05-20).

**Update (2026-05-21):** The convenience shim `SessionHandle::send_user`
described below was renamed to `SessionHandle::submit_user_text`. The
old name read as "send TO user" (verb+object) while the actual semantics
are "submit text FROM the user"; the noun-style name aligns with
`submit` and removes the directional ambiguity. Semantics unchanged —
still a 1-line shim over `submit(TurnTrigger::UserText(text.into()))`.
References to `send_user` in the prose below should be read with that
substitution where they describe the post-ADR API surface; references
in the Context and "Given up" sections describe pre-ADR / alternative
shapes and stand as-is.

## Context

Today the only way for a caller to start a turn is
`SessionHandle::send_user(text: impl Into<String>)`, which constructs
`NewMessage { text }` → `SessionCommand::Input(NewMessage)` → the
per-session loop writes
`EventPayload::TurnStarted { user_input: vec![ContentBlock::Text { text }] }`.
The trigger source is hard-coded as "user-typed plain text".

Three near-term needs break this assumption:

1. **Multimodal user input** (v0.2 storage / multimedia ADRs). A user
   message can carry an image, audio clip, or attached file in addition
   to (or instead of) text. `NewMessage { text: String }`'s field comment
   already flags this: "v0.2 may extend this to `Vec<ContentBlock>` for
   multimodal input."

2. **Skills as turn triggers** (post-v0.3 Subagent + Skills initiative).
   When a Skill is invoked — from the user via slash-command, or from
   another agent — the runtime needs to start a turn whose origin is
   "skill `foo` invoked with args `{...}`", not "user typed `foo`". The
   model's understanding of the turn differs (system-vs-user role; trace
   provenance for audit) and downstream consumers (billing, analytics)
   need to distinguish these cases.

3. **Hooks triggering a turn** (post-v0.6 Hooks beyond H09's policy-gate
   role). A scheduled or webhook-driven hook may need to inject a turn
   whose origin is the hook, not the human at the keyboard.

All three converge on the same shape: **the set of valid turn-triggers
is open and grows over time, but each one ultimately produces a
`TurnStarted` event and a `TurnDriver` task.** Without an abstraction in
place, every new trigger source forks `SessionHandle`'s API and
special-cases the per-session loop, which we know from Codex experience
leads to a sprawl of `send_*` methods that all look almost the same.

`AGENTS.md`'s open-closed guidance and ADR-0007's b-档 (additive variants
under `#[non_exhaustive]`) point at the same answer: introduce a
closed-by-default, forward-compatible enum now and ship variants as the
consumers land.

## Decision

### 1. Introduce `TurnTrigger` in `cogito-protocol`

```rust
// crates/cogito-protocol/src/turn_trigger.rs (new file)

use crate::content::ContentBlock;

/// What caused a new turn to start. Open-by-extension via
/// `#[non_exhaustive]` per ADR-0007 b-档: future variants are additive
/// and do NOT bump `schema_version`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TurnTrigger {
    /// User-typed plain text. The overwhelmingly common case for v0.1.
    UserText(String),

    // Reserved variants (DO NOT add to the enum until the matching
    // consumer lands — adding a variant before its handler exists
    // creates a dead-code path that drifts unverified):
    //
    // - UserContent(Vec<ContentBlock>)
    //     lands with: v0.2 multimedia ADR + ContentBlock::{Image, Audio}
    //     projection: actor writes TurnStarted.user_input = blocks
    //
    // - SkillInvocation { skill_id: String, args: serde_json::Value }
    //     lands with: post-v0.3 Skills initiative
    //     projection: actor writes TurnStarted.origin = Skill { skill_id }
    //                                       .user_input = derived from args
    //
    // - HookFired { hook_id: String, payload: serde_json::Value }
    //     lands with: post-v0.6 Hooks initiative beyond H09
    //     projection: actor writes TurnStarted.origin = Hook { hook_id }
    //                                       .user_input = derived from payload
}
```

The enum is the single source of truth for "what triggered this turn".
It lives in `cogito-protocol` (the only crate Brain may depend on) so
Brain can pattern-match without importing concrete trigger sources.

**v0.1 ships exactly one variant.** A single-variant `#[non_exhaustive]`
enum is intentional: it locks the *shape* of the abstraction so that
future variants are additive, even though the enum looks like overkill
today.

### 2. Public API surface on `SessionHandle`

```rust
impl SessionHandle {
    /// Submit a `TurnTrigger`. The session loop spawns a `TurnDriver`
    /// if no turn is in flight. **Canonical entry point** for any new
    /// trigger source.
    pub async fn submit(&self, trigger: TurnTrigger) -> Result<(), SessionError> { /* ... */ }

    /// Convenience: `submit(TurnTrigger::UserText(text.into()))`.
    /// Retained because user-typed text is the dominant path and
    /// callers should not have to spell out the enum for it.
    pub async fn submit_user_text(&self, text: impl Into<String>) -> Result<(), SessionError> {
        self.submit(TurnTrigger::UserText(text.into())).await
    }
}
```

`submit_user_text` is **not** deprecated. It is a thin shim with stable
semantics; deprecating it would create churn in every consumer (CLI,
integration tests, embed-in-product) for zero gain. New trigger kinds
use `submit`.

### 3. Internal command

```rust
// crates/cogito-core/src/runtime/types.rs

#[non_exhaustive]
pub enum SessionCommand {
    /// Caller-driven trigger (user text today; skills / hooks /
    /// multimodal in future versions). Spawns a `TurnDriver` if no
    /// turn is in flight.
    Trigger(TurnTrigger),

    JobCompleted { /* ... */ },
    InternalCancel { ack: tokio::sync::oneshot::Sender<()> },
    Shutdown { /* ... */ },
}
```

`NewMessage { text: String }` is **removed**.
`SessionCommand::Input(NewMessage)` → `SessionCommand::Trigger(TurnTrigger)`.
The struct exists only to carry the `text` field that
`TurnTrigger::UserText` already holds. The rename cost is bounded —
`NewMessage` was never exposed beyond `cogito-core::runtime` (`pub` only
at the module level).

### 4. Event-log projection

For v0.1, the event payload is **untouched**:

```rust
EventPayload::TurnStarted { user_input: Vec<ContentBlock> }
```

`TurnTrigger::UserText(text)` projects to
`vec![ContentBlock::Text { text }]`. This is exactly what the current
`record_turn_started` call already does; the projection step moves into
the per-session loop's `handle_command` arm.

**When the first non-user variant lands** (Skill, Hook, …), we extend
`TurnStarted` additively per ADR-0007:

```rust
EventPayload::TurnStarted {
    user_input: Vec<ContentBlock>,
    // Added when the first non-user trigger ships. Absent in the log
    // = "user" trigger (the v0.1 default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<TurnOrigin>,
}

#[non_exhaustive]
pub enum TurnOrigin {
    Skill { skill_id: String },
    Hook { hook_id: String },
    // ...
}
```

Why a separate `TurnOrigin` enum instead of widening
`user_input`? Two reasons. First, `Vec<ContentBlock>` is "what the
model sees as the user message" — a model-facing shape. Trigger
provenance is a runtime concept and does not belong in the prompt.
Second, billing / analytics consumers want to filter by trigger source
without parsing content blocks.

Existing logs stay valid (`origin: None` ≡ user-triggered) and external
readers (Go / Python / Node per ADR-0007) get a forward-compatible
upgrade path.

### 5. H03 resume semantics

H03's `replay()` treats `TurnStarted` as an opaque boundary event: it
reads `turn_id` from the envelope and only inspects payload fields when
reconstructing `ModelOutput` (`AssistantMessageAppended` /
`ToolUseRecorded`). Adding trigger-related fields to `TurnStarted` does
NOT change the resume algorithm.

The `RestartCurrentTurn` recovery path (currently downgraded to
`FreshTurn` per Sprint 3 closure) will read `TurnTrigger` from the
persisted `TurnStarted` event when it is implemented. The recovery is
symmetric across trigger kinds: if the original turn was triggered by
Skill `foo`, the restarted turn is also triggered by Skill `foo`.

### 6. Migration plan

| Sprint | Change |
|---|---|
| v0.1 (Sprint 4 or 5, opportunistic) | Add `TurnTrigger::UserText`, `SessionHandle::submit`, `SessionCommand::Trigger`. `send_user` becomes a 1-line shim. `NewMessage` deleted. Event payload unchanged. |
| v0.2 | Add `TurnTrigger::UserContent(Vec<ContentBlock>)` once multimodal storage / `ContentBlock::{Image, Audio}` ship. |
| v0.3+ | Add `TurnTrigger::SkillInvocation` together with the Skills consumer in `cogito-subagent` (or a successor crate); add `origin: Option<TurnOrigin>` to `TurnStarted`; CHANGELOG b-档 entry. |
| v0.6 | Add `TurnTrigger::HookFired` when the v0.6 Hooks initiative defines hook scheduling and payload schemas. |

Each step is additive. No `schema_version` bump.

### 7. What this ADR does NOT decide

- **Hook-pipeline triggers vs H09 policy gates.** H09 is a *policy
  pipeline* — `pre_prompt`, `pre_tool_call`, etc. — that runs *within*
  a turn already in flight. A "hook fires a turn" is a different beast
  (a hook acting as an *initiator*). Reconciling the vocabulary is
  deferred to the v0.6 Hooks ADR; this ADR only commits to the
  abstraction shape (one variant per initiator kind).
- **Authorization / capability checking.** A future ADR will specify
  which callers may submit `SkillInvocation` / `HookFired` triggers
  (likely a capability token on `SessionHandle` or a separate
  `SystemSessionHandle`). For v0.1 `submit` is unrestricted; user code
  can construct any `TurnTrigger` variant.
- **Turn-trigger ordering / queueing.** Current behavior: if a turn is
  in flight, `submit` is silently a no-op (the existing
  `try_start_turn` guard). Whether to queue, reject, or coalesce
  pending triggers is a separate question that has not been a problem
  in v0.1 single-turn-in-flight design.

## Consequences

**Easier:**

- New trigger sources land without an API break. Each variant carries
  its own typed payload; consumers pattern-match exhaustively on the
  variants they handle and fall through for unknown ones
  (`#[non_exhaustive]` forces a `_ =>` arm).
- The `SessionHandle` surface stays compact: one canonical entry point
  (`submit`) plus a convenience for the common case (`submit_user_text`).
- External (Go / Python / Node) readers see a forward-compatible event
  log. Per ADR-0007 they already tolerate unknown variants; the new
  optional `origin` field plays by the same rules.

**Harder / cost:**

- One extra indirection in the per-session loop's `try_start_turn`: a
  `match` on `TurnTrigger` to project into `Vec<ContentBlock>` (and
  later, into `(user_input, origin)`). Trivial in v0.1; grows by one
  arm per new variant.
- Internal rename: `SessionCommand::Input(NewMessage)` →
  `SessionCommand::Trigger(TurnTrigger)`. Touches
  `runtime::session_loop::handle_command`,
  `runtime::handle::send_user`, and the corresponding unit tests.
  Bounded blast radius; clippy + tests catch any miss.
- A single-variant `#[non_exhaustive]` enum looks like over-engineering
  on first read. The pattern is justified by the migration table above
  and by ADR-0007; reviewers should not strip the
  `#[non_exhaustive]` attribute when "simplifying" the enum.

**Given up:**

- The simpler `send_user(text)`-only API. We pay one extra enum + one
  extra method on `SessionHandle` to buy open-closed extensibility
  before we strictly need it. This is a deliberate up-front cost; the
  alternative (a sprawl of `send_*` methods accreting one per new
  trigger kind) is the well-known Codex pattern we are explicitly
  avoiding (`AGENTS.md` §"Inviolable design principles" #6).

## References

- `AGENTS.md` §"Inviolable design principles" #6 (Brain only sees
  Hands / Session / Boundary through Protocol traits)
- ADR-0007 §"Additive variants for context-management lifecycle"
  (b-档 forward-compatibility rules)
- `crates/cogito-protocol/src/event.rs` — `EventPayload::TurnStarted`
- `crates/cogito-core/src/runtime/handle.rs` —
  `SessionHandle::submit_user_text`
- `crates/cogito-core/src/runtime/session_loop.rs` — `try_start_turn`
  / `record_turn_started`
- 2026-05-20 design discussion ("OCP for `SessionHandle.send_user`")
