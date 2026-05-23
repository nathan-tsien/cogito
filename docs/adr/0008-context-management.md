# ADR-0008: Context Management

## Status

Accepted (2026-05-23) — Sprint 6

## Context

cogito's Turn Driver (H01) has a `ContextManaged` FSM state reserved by ADR-0006 but
left as a pass-through until this ADR fills it in. Three converging pressures force that
work into Sprint 6 rather than a later version:

1. Sprint 7 (Skill loader) needs a stable `SystemPromptInjector` trait to inject
   `SKILL.md` content into the system prompt every turn.
2. Sprint 9 (multi-model strategy) needs `ContextConfig` in `strategy.yaml` to switch
   Compactors based on which model is active.
3. v0.2 (Plugin) lets team members contribute `Compactor` / `Injector` / `Overrider`
   implementations as plugins — the trait surface must be frozen before plugins ship.

Without freezing these traits in Sprint 6, every subsequent version would require an
ADR amendment. The spec section §2 ("Why Sprint 6 is a critical path") gives the full
analysis. The H09 Hook Pipeline (Sprint 5) is not a substitute: hooks are pure policy
gates with no I/O authority; context management requires I/O (model calls for
summarization), event writes, and per-turn decision records (spec §2.3).

## Decision

cogito's Context Management surface, defined in `cogito-protocol::context`, consists of
four traits with distinct invariants:

1. `Compactor` — async, may do I/O (including model calls for summarization-style
   compactors). Writes 0 or 1 `ContextCompacted` events per turn via `StepRecorder`.
   Failures degrade: H11 records the error but does not block the turn.

2. `HistoryProjector` — pure synchronous function. Projects the event log to
   `Vec<Message>` for `ModelInput`. Must implement the set-union covered-range semantics
   defined below. No I/O, no event writes.

3. `SystemPromptInjector` — async (filesystem I/O needed by Sprint 7 Skill loader).
   Computes a per-turn system-prompt suffix and persists exactly one `SystemPromptInjected`
   event per turn, even when the suffix is empty (audit invariant, spec §4.1).

4. `ToolFilterOverrider` — async. Decides a per-turn tool filter override on top of
   `strategy.allowed_tools` and persists exactly one `ToolFilterOverridden` event per
   turn, even when the decision is `Inherit` (same audit invariant).

H11 orchestrates these four traits in fixed order each `ContextManaged` transition:
Compactor → SystemPromptInjector → ToolFilterOverrider → H11 writes
`ContextDecisionRecorded`. A trait failure degrades (error recorded in the `errors` field of
`ContextDecisionRecorded`); only a failure of H11's own fallback write propagates up and
causes `TurnFailed`.

Merging any two traits would violate one side's invariant (spec §3): Compactor and
HistoryProjector cannot merge because Projector would gain I/O authority; Injector and
Projector cannot merge because Injector would be forced to understand history structure.

### Crate layout

- `cogito-protocol::context` — trait definitions, `ContextPipeline`, config types,
  shared value types (`CompactionInput`, `CompactionApplied`, `CompactionReplacement`,
  `ToolFilterOverrideMode`, `ContextError`, `TokenThreshold`).
- `cogito-context` (new umbrella crate) — no-op defaults (`NoneCompactor`,
  `StandardProjector`, `NoneInjector`, `NoneOverrider`), `TruncateCompactor`, and the
  `build_pipeline(&ContextConfig) -> ContextPipeline` factory. Per CLAUDE.md
  "Tagged-config factories": the factory lives in the crate that owns the
  implementations; surface crates call `build_pipeline` and receive a trait object.
- `cogito-core::harness` — H11 `transitions::context_managed.rs` calls through the
  pipeline; H04 uses `HistoryProjector`; H05 reads `ToolFilterOverridden` events.
- `cogito-core::runtime` — `open_session` calls `build_pipeline` and injects the
  resulting `Arc<ContextPipeline>` into `SessionShared`.

### v0.1 shipped implementations

| Slot | v0.1 implementation | Config tag |
|---|---|---|
| Compactor | `NoneCompactor`, `TruncateCompactor` | `none`, `truncate` |
| HistoryProjector | `StandardProjector` | `standard` |
| SystemPromptInjector | `NoneInjector` | `none` |
| ToolFilterOverrider | `NoneOverrider` | `none` |

Note: `HistoryProjector` is invoked by H04 (Prompt Composer) at prompt-build time, not
by H11's orchestration pipeline. The three traits that H11 runs in sequence are
Compactor, SystemPromptInjector, and ToolFilterOverrider.

Sprint 7 adds `SkillsInjector`; v0.2 Plugin adds more — no ADR amendment needed because
`CompactorConfig`, `SystemPromptInjectorConfig`, etc. are `#[non_exhaustive]` enums.

## Event surface

Four additive `EventPayload` variants (all `#[non_exhaustive]`):

- `ContextCompacted { turn_id, replaced_seq_range, produced_by, replacement,
  token_estimate_before, token_estimate_after }` — written by Compactor only when it
  actually compacts (no-op Compactor does not write).
- `SystemPromptInjected { turn_id, suffix, contributors, produced_by }` — written every
  turn by SystemPromptInjector (suffix may be empty for no-op impls).
- `ToolFilterOverridden { turn_id, mode, contributors, produced_by }` — written every
  turn by ToolFilterOverrider (mode = `Inherit` for no-op impls).
- `ContextDecisionRecorded { turn_id, compactions, system_prompt_event,
  tool_filter_event, errors }` — written by H11 itself after all three traits finish.

These variants belong to `EventCategory::ContextDecision` (a new `category()` helper on
`EventPayload` that backends and analytics tools may use for physical partitioning, spec
§4.2). Brain code never branches on `category()`.

No `SCHEMA_VERSION` bump — additive variants do not break the forward-compatibility
contract established in ADR-0007.

External readers (Go / Python / Node consuming JSONL directly) must tolerate unknown
`type` values per ADR-0007's additive variant precedent.

## Projection semantics (HistoryProjector contract)

`StandardProjector` implements the following algorithm (full pseudocode in spec §5):

1. Build `covered`: the set-union of all `ContextCompacted.replaced_seq_range` values
   across the entire event log (including ranges from compaction events that were
   themselves later covered by a newer compaction).

2. Walk events in seq order. If `ev.seq` is in `covered`, skip it.

3. When encountering an uncovered `ContextCompacted` event, emit its `replacement`:
   - `Drop` — emit nothing.
   - `Summary { text, model }` — emit one user-role message:
     `<conversation_summary>\n{text}\n</conversation_summary>`.

4. Conversation events (`TurnStarted`, `AssistantMessageAppended`, `ToolUseRecorded`,
   `ToolResultRecorded`, `ThinkingBlockRecorded`) emit messages in the normal way. All
   other meta events are ignored.

5. The system prompt is `strategy.system_prompt` optionally appended with the suffix
   from the current turn's `SystemPromptInjected` event (`\n\n` separator).

Key invariants enforced by `StepRecorder` at write time (spec §5.5):

- `replaced_seq_range.1 < self.seq` — a compaction event cannot include its own seq
  (no self-reference).
- `replaced_seq_range.0` must align to a `TurnStarted.seq` (turn boundary at start).
- `replaced_seq_range.1` must align to the last conversation event of that turn (turn
  boundary at end).
- At most one `ContextCompacted` per `turn_id` (v0.1; relaxable in v0.2).

Violation returns `StoreError::InvariantViolated`; the Compactor receives `Err` and H11
degrades without blocking the turn.

Cascade semantics: a newer `ContextCompacted` whose range includes a prior compaction's
seq causes the prior compaction's `replacement` to be suppressed in projection (its seq
is in `covered`), but its `replaced_seq_range` continues to contribute to `covered` —
the older compacted history is not "released". This is the natural consequence of the
set-union rule; no `max_depth` concept is needed (spec §5.4).

Backward compatibility: pre-Sprint-6 sessions have no `ContextCompacted` or
`SystemPromptInjected` events. `covered` is empty, no suffix is appended, and projection
behaves identically to the pre-Sprint-6 code path.

## Cross-provider adaptive threshold

`ModelGateway` gains a new method (additive, default impl provided so existing
implementations need not change immediately):

```rust
fn model_limits(&self) -> ModelLimits {
    ModelLimits { model_id: self.provider_id().into(), context_window_tokens: 32_768 }
}
```

`ModelLimits` carries `model_id: String` and `context_window_tokens: u64`. It does not
carry `max_output_tokens` — output ceiling is a strategy-dimension decision already
covered by `strategy.model_params.max_tokens` (spec §6.7).

Provider implementation responsibilities:

- `AnthropicGateway`: parse `[<size>]` suffix first; otherwise look up the base model id
  in a static table (opus-4-7 / sonnet-4-6 / haiku-4-5, all 200k default); fall back to
  200k with a warning for unknown ids. The suffix is kept verbatim when calling the
  Anthropic API (Anthropic accepts it as the actual model id).
- `OpenAiCompatGateway`: parse suffix first; otherwise read
  `OpenAiCompatProviderConfig.context_window_tokens: Option<u64>`; fall back to 32_768
  with a warning if both are absent. The suffix is stripped before the API call
  (`api_model_id()` private helper) because vLLM / SGLang do not recognise the suffix.
- `MockModelGateway`: default impl (32k); individual tests may inject custom `ModelLimits`
  via the builder.

### Model id `[<size>]` suffix convention

A model id may carry a `[<size>]` suffix to declare its context window. The suffix
grammar is `\[(\d+)([kKmM])?\]$`; `k` = 1 000, `m` = 1 000 000; bare integer = exact
value. Examples: `claude-opus-4-7[1m]` → 1 000 000, `Llama-3.3-70B[32k]` → 32 000,
`gpt-4o[128000]` → 128 000.

`parse_context_window_suffix(model_id: &str) -> Option<u64>` is a public helper in
`cogito-protocol::gateway` shared by all gateway crates.

The convention lets a single string carry both the model identity and context window fact
— `strategy.yaml` does not need a separate `context_window_tokens` field, and switching
the model id automatically updates the compaction threshold.

## `TruncateCompactor` adaptive threshold

`TruncateConfig.max_tokens: TokenThreshold` has two variants:

- `Ratio { of_context_window: f32, safety_headroom: u64 }` — derives the token budget
  from `model_gateway.model_limits().context_window_tokens`. Default: `Ratio { 0.75,
  8192 }`. When the model changes, the threshold follows automatically.
- `Absolute(u64)` — ignores `model_limits`; useful for cost caps or tests requiring a
  precise threshold.

Token estimation uses `last_usage.prompt_tokens` when available (true value from the
previous model call); otherwise falls back to `chars / 4` (accuracy ±20%). The default
headroom is sized to absorb this variance for typical 4–8k output models.

## Resume / idempotency

No new `ResumePoint` variant is needed. H03 handles a `ContextManageEntered` with no
paired `ContextManageCompleted` by choosing `ResumeFromInit`, which re-runs the
`ContextManaged` transition. All three traits are idempotent on `turn_id`:

- `Compactor`: before calling the model, checks whether a `ContextCompacted` event for
  the current `turn_id` already exists; if so, returns the existing `CompactionApplied`
  without further I/O.
- `SystemPromptInjector`: checks for an existing `SystemPromptInjected` for the current
  `turn_id`; if found, returns the existing `EventId` without re-writing.
- `ToolFilterOverrider`: same pattern for `ToolFilterOverridden`.

A crash mid-summarization (before the `ContextCompacted` event is flushed) causes H11 to
re-run the Compactor on resume, which re-issues the model call. This is semantically
correct and the only scenario where work is duplicated (spec §12.3).

## Configuration

`HarnessStrategy.context: ContextConfig` holds four tagged-config enums:

```
ContextConfig {
    compactor:              CompactorConfig,              // none | truncate
    history_projector:      HistoryProjectorConfig,       // standard
    system_prompt_injector: SystemPromptInjectorConfig,   // none
    tool_filter_overrider:  ToolFilterOverriderConfig,    // none
}
```

Each enum is `#[non_exhaustive]` with `#[serde(tag = "kind")]`. Adding a new variant
(e.g., `Summary` Compactor in v0.2) requires editing only `cogito-context` and its
config enum; surface crates remain untouched. This follows the tagged-config factory
rule in CLAUDE.md.

## Consequences

Positive:

- The four-trait boundary makes the extension model explicit: any team member can write a
  fifth `Compactor` or a second `Injector` without reading the full H11 module.
- All context decisions are durable in the event log — auditors can replay "what the
  model saw" for any turn without reading source code.
- Crash recovery is simple: trait idempotency means re-running H11 after any partial
  failure is always safe (spec §12).
- `ModelGateway::model_limits()` lets one string in `strategy.yaml` carry both model
  identity and context-window capacity, eliminating a whole class of "forgot to update
  the threshold when switching models" mistakes.
- No `SCHEMA_VERSION` bump; existing JSONL readers tolerate the new event variants by
  ignoring unknown `type` values (ADR-0007 forward-compat bargain).

Negative:

- Every `ContextManaged` transition writes 2–3 trait events plus 2 FSM events plus 1
  decision summary = 5–6 events per turn (~80–200 bytes each). For a 1 000-turn session
  that is roughly 1 MB of metadata — acceptable in v0.1 dev/debug but worth watching
  at scale.
- Token estimation via `chars / 4` can be ±20% from the true `prompt_tokens`. The
  default 25% headroom absorbs this for most models, but long-output scenarios
  (16k+ tokens) may need a manually raised `safety_headroom` or switch to `Absolute`
  thresholding.
- A new `cogito-context` crate is added to the workspace. Future Compactor or Injector
  implementations must live there (or in separate Hands crates in v0.2+), not inline in
  `cogito-core`.

## Alternatives considered

Three architectural alternatives for splitting the trait surface were evaluated (spec §3):

- Merge Compactor + HistoryProjector: rejected because Projector would gain I/O
  authority, violating its pure-function invariant (H04's design contract).
- Merge HistoryProjector + SystemPromptInjector: rejected because Injector would be
  forced to understand history structure, breaking single-responsibility.
- Omit ToolFilterOverrider (handle tool surface in H05 directly): rejected because
  `strategy.allowed_tools` is static and H05 has no mechanism for per-turn dynamic
  override; v0.2 Plugin and v0.3 Subagent both need dynamic tool surface control.

Four projection algorithm options were considered (spec §5, options S1–S4):

- S1 — store the final projected messages in the event log: rejected, high storage
  amplification and cross-language coupling to Rust `Vec<Message>` shape.
- S2 — pointer-based compaction (store references rather than ranges): rejected, complex
  GC semantics across concurrent resume scenarios.
- S3 — per-turn snapshot: rejected, O(n^2) storage growth.
- S4 (chosen) — strict event sourcing with covered-range set-union: accepted. Single
  stream, auditable, language-neutral, O(1) storage overhead per compaction event.

Model-limits placement options evaluated:

- Store context window capacity in `HarnessStrategy`: rejected, creates a footgun when
  switching models (users must remember to update a separate field). Context window is a
  model fact, not a strategy choice.
- Hardcode per-provider constants in the Compactor: rejected, Compactor would need to
  know about providers, violating the Brain/Hands/Session boundary (ADR-0004).
- `ModelGateway::model_limits()` with default impl (chosen): additive, no forced
  migration of existing implementations, provider adapters override where they have
  accurate data.

## References

- ADR-0004 (Brain/Hands/Session layer boundaries)
- ADR-0006 (H01 FSM; `ContextManaged` state originally reserved here)
- ADR-0007 (event log forward-compat; additive variant precedent)
- `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md` (full
  design rationale, pseudocode, test plan, and open questions)
- `docs/components/H11-context-manage.md` (component-level implementation notes)
- `docs/components/H04-prompt-composer.md` (HistoryProjector dispatch)
- `docs/components/H05-tool-surface.md` (ToolFilterOverridden integration)
