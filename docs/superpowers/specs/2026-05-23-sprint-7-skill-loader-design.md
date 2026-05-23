# Sprint 7 · Skill Loader (`cogito-skills`) — design spec

**Status**: Proposed
**Sprint**: v0.1 / Sprint 7
**Budget**: 1.5–2 days
**ADR**: [`ADR-0020`](../../adr/0020-skill-loader.md) (Proposed; ratified at end of this sprint)
**Predecessors**: Sprint 6 (`cogito-context`, `SystemPromptInjector`), Sprint 5 (Hook pipeline), 2026-05-22 roadmap rebalance §2.5–§2.6
**Successors**: Sprint 12 / ADR-0021 (Plugin loader extends this); ADR-0023 (bundled script execution)

---

## 1. Goals

Ship a **userland skill loader** so team members can extend agent behaviour via
markdown + YAML, no Rust required. Concretely, after Sprint 7 lands:

1. A team member drops `~/.cogito/skills/my-skill/SKILL.md` (or
   `.cogito/skills/...` in a repo); the next `cogito chat` run picks it up.
2. The model reads an "Available Skills" registry in its system prompt and can
   activate any skill by emitting `$skill-name` in its text reply.
3. The user can activate the same skill manually with `/skill <name>`.
4. The full `SKILL.md` body lands in the next turn's system prompt; the model
   continues with the skill's instructions in scope.
5. Crash mid-flow: the resume coordinator replays cleanly with the same skill
   set active.

All of this happens through the trait surface frozen in Sprint 6
(`SystemPromptInjector`) — no new H11-level orchestration.

## 2. Non-goals

- **Bundled script execution.** `SKILL.md` may reference `scripts/foo.py`, but
  cogito does NOT auto-register or auto-execute. The model uses `read_file` /
  `bash` like any other file. Tracked separately as ADR-0023.
- **Plugin scope discovery.** Requires the Plugin loader (Sprint 12 / ADR-0021).
  v0.1 ships Repo + User scopes only. The `SkillSource::Plugin` enum variant is
  defined for forward-compat but never produced.
- **Hot reload.** Registry is built once at `RuntimeBuilder::build()`. Changes
  to `SKILL.md` files require a restart.
- **Skill versioning conflict resolution beyond fatal-on-duplicate.** Two
  skills named `foo` in the same scope = startup error.
- **Cross-skill dependencies.** Skills cannot `import` other skills. The model
  can still activate multiple via successive sigils.

## 3. Locked decisions (refer to ADR-0020)

ADR-0020 already ratifies six core decisions; the spec below assumes them:

| Topic | Decision |
|---|---|
| Activation channel | K5 — sigil `$SkillName` + user `/skill <name>` slash |
| Scope precedence (high → low) | Repo > User > Plugin > System (v0.1 ships Repo + User) |
| Frontmatter required | `name`, `description` |
| Frontmatter optional | `disable-model-invocation`, `user-invocable`, `version` |
| Bundled scripts | Deferred (no auto-exec); ADR-0023 placeholder |
| Crate layout | New `cogito-skills` crate (Hands layer) |

This spec lays down the implementation detail ADR-0020 leaves open: the precise
trait shapes, event flow, regex, CLI grammar, char caps, idempotency rules, and
resume semantics.

## 4. Architectural overview

```
┌─────────────────────────────────────────────────────────────────────┐
│  RuntimeBuilder                                                     │
│    ├─ scans .cogito/skills/ and ~/.cogito/skills/  ──► SkillRegistry │
│    └─ wraps as Arc<dyn SkillProvider>                                │
│                                                                     │
│      ┌───────────────────────────────────────────────────────┐     │
│      │  cogito_context::build_pipeline(config, provider)     │     │
│      │     ContextPipeline {                                  │     │
│      │       injector: SkillInjector { provider } | None      │     │
│      │       ...                                              │     │
│      │     }                                                  │     │
│      └───────────────────────────────────────────────────────┘     │
│                                                                     │
│  cogito-cli / cogito chat:                                          │
│    repl line ──► parse:                                             │
│       "/skill foo bar do X" ──► TurnTrigger::SkillActivation        │
│                                    { names: [foo, bar],             │
│                                      user_text: Some("do X") }      │
│       "plain text"          ──► TurnTrigger::UserText("plain text") │
└─────────────────────────────────────────────────────────────────────┘

Turn N+1 — Init → ContextManaged:
  1. Compactor.maybe_compact()
  2. SkillInjector.inject(input):
       a. read TurnStarted.activate_skills (user-channel names)
       b. scan history's TextBlockRecorded events from the previous turn(s)
          for $sigil tokens, code-fence-aware
       c. filter to registered names via provider.is_registered()
       d. dedupe: drop names already in prior SkillActivated events
       e. for each remaining name: write SkillActivated event
       f. write one SystemPromptInjected event with suffix =
            "## Available Skills\n- ... (registry block, every turn)\n"
            + "\n\n<skill name=\"foo\">…SKILL.md body…</skill>\n…"
              (one wrapped block per newly-activated skill)
  3. ToolFilterOverrider.override_filter()
  4. ContextDecisionRecorded, ContextManageCompleted
  5. PromptComposed via H04 — picks up the system suffix unchanged

Turn N — model emits "$invoice-parser please run":
  6. H06 stream:
       a. accumulate text deltas; maintain code-fence state (``` toggles;
          inline backticks tracked per-line)
       b. on `$<ident>` match outside code: emit
          ModelEvent::SkillActivationRequested { name } — informational,
          surfaces via StreamEvent for telemetry but NOT persisted
       c. text itself is recorded normally as TextBlockRecorded
  → Turn N completes; activation is processed on Turn N+1 (step 2 above).
```

## 5. New protocol types (`cogito-protocol`)

### 5.1 `SkillProvider` trait

```rust
// crates/cogito-protocol/src/skill.rs

use std::sync::Arc;

/// Read-only handle on the registered skill set. Built by the Surface
/// (Runtime construction) and consumed by H04/H06/H11 via `Arc<dyn
/// SkillProvider>` injection.
pub trait SkillProvider: Send + Sync {
    /// Lightweight metadata for the "Available Skills" registry block.
    /// Called once per turn by the SkillInjector. MUST be cheap.
    fn list(&self) -> Vec<SkillMetadata>;

    /// Full skill body (SKILL.md text, frontmatter stripped) for
    /// activation. `None` if the name is not registered. May read from
    /// disk on first call; implementations SHOULD cache.
    fn get(&self, name: &str) -> Option<SkillContent>;

    /// O(1) check used by H06 sigil filter — only registered names
    /// activate; unknown $X is literal text.
    fn is_registered(&self, name: &str) -> bool;
}

/// Lightweight skill descriptor (no body).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    pub disable_model_invocation: bool,
    pub user_invocable: bool,
    pub version: Option<String>,
}

/// Full skill body for activation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillContent {
    pub name: String,
    pub source: SkillSource,
    /// SKILL.md body with frontmatter stripped (already validated UTF-8).
    pub body: String,
}

/// Where a skill was discovered. Forward-compat with v0.2 plugins.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillSource {
    /// `<workspace>/.cogito/skills/<name>/`
    Repo { dir: PathBuf },
    /// `~/.cogito/skills/<name>/`
    User,
    /// `<plugin>/skills/<name>/` — never produced in v0.1.
    Plugin { plugin_id: String },
    /// cogito-bundled (feature-gated; off by default in v0.1).
    System,
}
```

### 5.2 New `EventPayload` variant — `SkillActivated`

```rust
EventPayload::SkillActivated {
    /// Bare name (Repo / User / System) or "<plugin_id>:<name>" (Plugin).
    skill_name: String,
    /// Where this skill was discovered.
    source: SkillSource,
    /// Channel that triggered the activation, for telemetry / debugging.
    channel: SkillActivationChannel,
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillActivationChannel {
    /// Model emitted `$Name` in stream text.
    ModelSigil,
    /// User typed `/skill <name>`.
    UserSlash,
}
```

Additive under ADR-0007 / `#[non_exhaustive]`. No `SCHEMA_VERSION` bump.

### 5.3 Extended `EventPayload::TurnStarted`

```rust
EventPayload::TurnStarted {
    user_input: Vec<ContentBlock>,
    /// User-requested skill activations carried via TurnTrigger::SkillActivation.
    /// Empty for UserText triggers. Independent from sigil-based activations
    /// (which are re-derived from previous-turn text).
    #[serde(default)]
    activate_skills: Vec<String>,
}
```

`#[serde(default)]` keeps backward-compat with v0.1 pre-Sprint-7 JSONL fixtures.

### 5.4 New `TurnTrigger` variant — `SkillActivation`

```rust
#[non_exhaustive]
pub enum TurnTrigger {
    UserText(String),  // unchanged

    /// User invoked one or more skills via `/skill <name>` (optionally
    /// with trailing text). `user_text` is the leftover after slash parsing
    /// (None when the user typed only `/skill foo`).
    SkillActivation {
        names: Vec<String>,
        user_text: Option<String>,
    },
}
```

The session loop's projection — `TurnTrigger → TurnStarted` — handles
`SkillActivation` thus:

| Trigger | `TurnStarted.user_input` | `TurnStarted.activate_skills` |
|---|---|---|
| `UserText("hi")` | `[Text("hi")]` | `[]` |
| `SkillActivation { names: ["foo"], user_text: None }` | `[]` | `["foo"]` |
| `SkillActivation { names: ["foo"], user_text: Some("hi") }` | `[Text("hi")]` | `["foo"]` |
| `SkillActivation { names: ["foo","bar"], user_text: None }` | `[]` | `["foo", "bar"]` |

When `user_input` is empty AND `activate_skills` is non-empty, the upcoming
turn's user message is just the skill body wrapped in the system-prompt suffix —
no top-level user message is appended to `messages`. The model sees the activated
content and produces a turn (likely echoing acknowledgement + waiting for next
user input). This matches Claude Code's `/skill` UX.

### 5.5 New `ModelEvent` variant — `SkillActivationRequested`

```rust
#[non_exhaustive]
pub enum ModelEvent {
    // ... existing variants
    /// H06 detected `$<registered>` outside code blocks. In-memory only;
    /// surfaces via StreamEvent for telemetry. NOT persisted.
    SkillActivationRequested { name: String },
}
```

Per Q1 (re-derive), this event is never written to the conversation store.
H11's SkillInjector reconstructs the same set by scanning text blocks.

### 5.6 New `SystemPromptInjectorConfig` variant

```rust
pub enum SystemPromptInjectorConfig {
    None,
    /// SkillInjector — wires in the SkillProvider injected at Runtime build.
    Skill,
}
```

The factory in `cogito-context::build_pipeline` takes
`Option<Arc<dyn SkillProvider>>` as a second argument; when
`SystemPromptInjectorConfig::Skill` is selected and the provider is `None`, the
build fails fast with a configuration error.

## 6. `cogito-skills` crate

```
crates/cogito-skills/
  Cargo.toml
  src/
    lib.rs            -- pub use of SkillRegistry, ScanConfig, SkillRegistryError
    discovery.rs      -- scope-based filesystem walker
    metadata.rs       -- YAML frontmatter parser (serde_yaml)
    registry.rs       -- SkillRegistry impl of SkillProvider
    sigil.rs          -- regex compile, code-fence-aware match
  tests/
    discovery.rs
    metadata.rs
    sigil.rs
    fixtures/
      .cogito/skills/...
```

Layer: **Hands**. Dependencies allowed: `cogito-protocol`, `serde_yaml`,
`regex`, `walkdir`, `thiserror`, `tracing`. No `cogito-core` (Brain can only
see Protocol traits).

### 6.1 `SkillRegistry` shape

```rust
pub struct ScanConfig {
    pub workspace_root: Option<PathBuf>,   // cwd by default; walked up to git/cogito.toml root
    pub user_dir: Option<PathBuf>,         // ~/.cogito/skills/ by default; None disables
    pub include_system: bool,              // feature-gated; default false
}

pub struct SkillRegistry {
    by_name: HashMap<String, Arc<SkillRecord>>,
    sigil_regex: Regex,
}

impl SkillRegistry {
    /// Eager scan. Returns first error encountered (fatal on duplicate name
    /// in same scope; warning + skip on bad frontmatter).
    pub fn scan(config: ScanConfig) -> Result<Self, SkillRegistryError> { ... }
}

impl SkillProvider for SkillRegistry { ... }
```

Frontmatter parsing failures: log `warn!` + skip (don't fail startup over one
bad SKILL.md). Duplicate-name rules:

- **Within a single directory**: a name collision (two `SKILL.md` files
  declaring `name: foo` under one `.cogito/skills/`) is fatal at Runtime build.
- **Across directories within Repo scope** (monorepo walk-up chain): closer
  (deeper, nearer cwd) directory wins on bare-name conflict; the further match
  is dropped with a `debug!` log.
- **Across scopes** (e.g., Repo `foo` vs User `foo`): silent precedence — Repo
  > User > Plugin > System. Lower-scope skill is dropped with a `debug!` log.

### 6.2 Repo-scope walk-up rule

Starting from cwd, walk parent directories until **any** of:
- `.git/` directory exists (git project root)
- `cogito.toml` file exists (cogito project root)
- filesystem root reached

Each directory along the path is checked for `.cogito/skills/`. Multiple repo
roots can contribute (monorepo: a parent `.cogito/skills/` shadows nothing in
nested sub-paths, but the closer (deeper) dir wins on bare-name conflict).

### 6.3 Sigil regex + code-fence state machine

Regex: `\$([A-Za-z][A-Za-z0-9_:-]{0,63})` — captures `name` group; max 64 chars
to match agentskills.io name conventions.

Code-fence awareness (`sigil.rs::find_sigils_outside_code`):

- Track three states: `Normal`, `InFenced(closing_marker)`, `InInline`.
- Tokens: `^```\s*\w*\n` opens fence; matching closing fence ends it; backtick
  (`` ` ``) toggles `InInline` within a logical line.
- Sigils matched only in `Normal` state.
- Streaming-friendly: state survives across deltas. H06 maintains state per
  block, reset on `text_block_start`.

The function signature:

```rust
pub fn find_sigils_outside_code(
    state: &mut FenceState,
    chunk: &str,
) -> Vec<SigilHit>;

pub struct SigilHit {
    pub name: String,
    pub byte_offset: usize,
}
```

## 7. `cogito-context::injector::skill` (new module)

```
crates/cogito-context/src/injector/
  mod.rs       (existing — add `pub mod skill;`)
  none.rs      (existing)
  skill.rs     (NEW)
```

```rust
// crates/cogito-context/src/injector/skill.rs

pub struct SkillInjector {
    provider: Arc<dyn SkillProvider>,
    description_cap_chars: usize,
}

impl SkillInjector {
    pub fn new(provider: Arc<dyn SkillProvider>) -> Self {
        Self { provider, description_cap_chars: 1024 }
    }
}

#[async_trait]
impl SystemPromptInjector for SkillInjector {
    async fn inject(&self, input: InjectionInput<'_>) -> Result<EventId, ContextError> {
        // 1. Idempotency: scan input.history for SystemPromptInjected of this turn;
        //    if present, return early.
        if let Some(eid) = find_existing_injection(input.history, input.turn_id) {
            return Ok(eid);
        }

        // 2. Collect activation candidates.
        //    a. User channel: TurnStarted.activate_skills for current turn
        //    b. Model channel: scan TextBlockRecorded from previous turn
        let user_names  = collect_user_channel(input.history, input.turn_id);
        let model_names = collect_model_channel(input.history, input.turn_id, &*self.provider);

        // 3. Dedupe: subtract names already in prior SkillActivated events.
        let prior_activations = collect_prior_activations(input.history);
        let to_activate = (user_names.iter()
            .map(|n| (n.clone(), SkillActivationChannel::UserSlash))
            .chain(model_names.into_iter()
                .map(|n| (n, SkillActivationChannel::ModelSigil))))
            .filter(|(n, _)| !prior_activations.contains(n))
            .collect::<Vec<_>>();

        // 4. Write one SkillActivated event per (deduped within this turn).
        let mut seen_this_turn = HashSet::new();
        for (name, channel) in &to_activate {
            if !seen_this_turn.insert(name.clone()) { continue; }
            let source = self.provider.get(name)
                .map(|c| c.source.clone())
                .unwrap_or(SkillSource::User);  // defensive; should not happen
            input.recorder
                .record_skill_activated(input.turn_id, name.clone(), source, *channel)
                .await?;
        }

        // 5. Build suffix:
        //    - "Available Skills" block (always when registry non-empty)
        //    - <skill name="..."> body wrappers for newly-activated skills
        let suffix = build_suffix(&*self.provider, &seen_this_turn, self.description_cap_chars);
        let contributors = seen_this_turn.iter().cloned().collect();

        let event_id = input.recorder
            .record_system_prompt_injected(input.turn_id, suffix, contributors, "skill")
            .await?;
        Ok(event_id)
    }

    fn id(&self) -> &'static str { "skill" }
}
```

`record_skill_activated` is a new method on the `EventRecorder` trait:

```rust
async fn record_skill_activated(
    &mut self,
    turn_id: TurnId,
    skill_name: String,
    source: SkillSource,
    channel: SkillActivationChannel,
) -> Result<EventId, StoreError>;
```

(Like the other record-* methods, the default impl can be a route through
`append_payload`.)

### 7.1 Suffix shape

```
## Available Skills

- invoice-parser: Parses invoices ... (capped at 1024 chars)
- report-writer: ...

<skill name="invoice-parser" source="repo">
[full SKILL.md body, frontmatter stripped]
</skill>
```

If no skills activated: the "Available Skills" block is the only content.
If no skills registered at all: empty suffix (equivalent to NoneInjector).

## 8. H06 changes (`cogito-core::harness::stream_demux`)

Today's H06 only handles text + thinking + tool-use streaming. Sprint 7 adds:

- A `Option<Arc<dyn SkillProvider>>` injected into the demuxer (None ⇒ skill
  detection disabled — feature-clean for existing tests).
- A `FenceState` carried per `text_block` (reset on `text_block_start`).
- On each `text_delta`: feed delta to `find_sigils_outside_code`; for each hit,
  call `provider.is_registered(name)`; if true, emit
  `ModelEvent::SkillActivationRequested { name }` once per (name, block).
- The `text_delta` is still passed through unchanged; sigil detection is
  side-effect-only.

In-block dedup: `HashSet<String>` per block. Cross-block dedup is unnecessary;
H11's SkillInjector dedupes via prior `SkillActivated` events.

## 9. CLI changes (`cogito-cli::chat`)

Slash grammar (REPL line parsing):

```
line.trim() starts with "/skill " ?
  yes ─► tokens = line[7..].split_whitespace()
         names  = collect tokens while they parse as a valid skill name
                  (against the registry — unknown names error before submit)
         rest   = remaining input joined by ' '; None if empty
         emit TurnTrigger::SkillActivation { names, user_text: rest }
  no  ─► emit TurnTrigger::UserText(line)
```

Behavior:
- `/skill foo` — activates foo, no user message.
- `/skill foo do this` — activates foo, sends "do this" as user message.
- `/skill foo bar` — activates foo + bar (if both registered), no message.
- `/skill foo bar do this` — activates foo + bar, sends "do this".
- `/skill unknown` — REPL prints error inline (does NOT submit a turn).
- `/skill foo unknown bar` — ambiguous; v0.1 rule: scan tokens left-to-right
  while they match registered names; first unknown token starts `user_text`.
  So `unknown bar` becomes user text. (Documented in CLI `--help`.)

## 10. Event log additions (`docs/data-model/jsonl-v1.md`)

Two additive entries — no schema bump per ADR-0007:

- `turn_started` payload gains optional `activate_skills: string[]` (defaults to
  `[]` on read for older events).
- New `skill_activated` payload: `{ skill_name, source, channel }`.

Canonical fixture `crates/testing/cogito-test-fixtures/fixtures/sessions/sample-skill-v1.jsonl`
demonstrates both. CI's schema-drift gate (`make gen-schema`) regenerates
`docs/schemas/conversation-event-v1.json`.

## 10b. `cogito.toml` opt-in

A new top-level `[skills]` section configures the loader; `[context]` selects
the SkillInjector. All fields optional with sensible defaults:

```toml
[skills]
# Enable / disable scope sources; defaults shown.
enabled        = true
user_dir       = "~/.cogito/skills"   # set to "" to disable user scope
include_system = false                # opt-in to bundled (feature-gated)

[context.system_prompt_injector]
kind = "skill"                        # vs "none" (default)
```

When `[skills] enabled = false`, `RuntimeBuilder::build()` skips the filesystem
scan and registers no SkillProvider; setting `kind = "skill"` in that state
is a config error at build time. The default `cogito.toml` (no `[skills]`
section) keeps the existing `injector = none` behavior — Sprint 7 is opt-in,
not silently breaking.

## 11. Resume semantics

Both activation channels are derivable from durable state:

| Channel | Source of truth on resume |
|---|---|
| User slash | `TurnStarted.activate_skills` of the resumed turn |
| Model sigil | `TextBlockRecorded` from previous turn(s) + registry |

The SkillInjector's idempotency check (find existing `SystemPromptInjected` for
this turn) cuts off re-injection on crash-after-write. If crash happens
*between* SkillActivated and SystemPromptInjected:

- On resume, scan finds prior `SkillActivated` events → those names are filtered
  out of `to_activate` (step 3 above).
- Replay writes only the missing `SystemPromptInjected` event.
- Net: same final history, no double-injection.

Resume-chaos test scenario `text_then_skill_then_tool` (new):

1. Turn 1: user `"hi"`, model `"sure $invoice-parser"`. Crash at:
   - (a) after TextBlockRecorded (no activation yet) → on resume,
     SkillInjector activates on Turn 2.
   - (b) after SkillActivated, before SystemPromptInjected → on resume,
     re-derive sees prior SkillActivated, skips writing again, writes the
     missing SystemPromptInjected.
   - (c) after SystemPromptInjected of Turn 2, before PromptComposed → idempotent
     re-entry returns existing event_id.

All four oracles (prefix-immutable / terminal-equivalent / tool-mapping /
final-text) MUST pass for each boundary.

## 12. Test plan

### Unit (in `cogito-skills`)

- `metadata.rs`: frontmatter parser — required fields, optional fields, kebab-
  case name validation, version semver-shape (loose), oversized description.
- `discovery.rs`: scope walk — git-root stop, cogito.toml stop, fs-root stop,
  multiple repo roots in monorepo, missing user dir = no error, duplicate
  name in same dir = error, cross-scope shadow.
- `sigil.rs`: regex correctness (anchored on letter, kebab/colon allowed), max
  length, FenceState transitions (open ``` → close, inline ` ` toggle,
  multi-line fence, escaped backticks not treated specially in v0.1).

### Unit (in `cogito-context`)

- `injector::skill::tests`: NoneInjector-equivalent contract for empty registry,
  registry-only emits "Available Skills" block, user-channel-only path,
  model-channel-only path, dedupe across channels, dedupe vs prior
  SkillActivated, idempotent on existing SystemPromptInjected, char cap on
  description.

### Integration (in `cogito-core`)

- `tests/h06_skill_sigil_detection.rs`: mock model emits text streams with
  sigils inside / outside code fences; assert exactly the expected set of
  `ModelEvent::SkillActivationRequested` surfaces.
- `tests/h11_skill_injection.rs`: end-to-end H11 pass with SkillInjector
  configured — covers all three activation paths.
- `tests/turn_driver_skill_activation_user.rs`: TurnTrigger::SkillActivation
  flows through to a TurnStarted with activate_skills + (optional) user_input.

### Cross-crate

- `cogito-skills/tests/discovery.rs` with file fixtures under
  `tests/fixtures/.cogito/skills/` and `tests/fixtures/home-cogito/skills/`.
- `cogito-cli/tests/slash_skill.rs`: REPL-level command parsing.

### Chaos

- `cogito-core/tests/resume_chaos.rs::text_then_skill_then_tool` — see §11.

### E2E

- `cogito-cli` against `MockModelGateway` emitting a sigil reply; assert
  end-to-end SkillActivated event written + next turn's prompt contains the
  skill body.

## 13. Implementation defaults (settled inline, surfaced for review)

| Decision | Default |
|---|---|
| Repo-root stop condition | `.git/` OR `cogito.toml` OR fs root (first hit wins) |
| User dir default | `~/.cogito/skills/` (override via cogito.toml `[skills] user_dir = ...`) |
| Description char cap | 1024 chars per skill in the registry block |
| Skill name max length | 64 chars (matches sigil regex) |
| Sigil regex | `\$([A-Za-z][A-Za-z0-9_:-]{0,63})` (kebab + colon for plugin ns) |
| Code-fence handling | Fenced (```) + inline (`` ` ``) detection (Q3 = A) |
| Within-turn sigil dedup | Same `$Name` 3× ⇒ inject once (HashSet in injector) |
| Cross-turn dedup | Skip if any `SkillActivated { skill_name: name }` exists earlier |
| Same-directory duplicate name | Fatal at Runtime build |
| Same-scope, different-dir duplicate (Repo monorepo walk) | Closer dir wins; further dropped (debug log) |
| Cross-scope same name | Higher scope wins; lower-scope dropped (debug log) |
| Frontmatter validation failure | `warn!` + skip skill; do NOT fail Runtime build |
| Discovery timing | Eager at `RuntimeBuilder::build()`; no hot reload |
| System skills feature gate | `cogito-skills` Cargo feature `system-skills`, off by default |

## 14. Open implementation questions (to settle during plan-writing)

The following are *details* I want to surface but don't think need a design
decision before the plan exists:

1. **Whether to wrap skill bodies in `<skill name="…">…</skill>` XML tags vs a
   Markdown delimiter.** Codex uses XML; Claude Code uses Markdown-style. XML
   is unambiguous and easy to grep in transcripts; recommend XML. Plan task
   can carry the final wording.
2. **Whether the "Available Skills" block character cap counts the rendered
   bullet line or only the description text.** Lean towards: cap the
   description text alone; the leading "- name: " is free.
3. **Order of `<skill>` blocks when multiple activate in same turn.** Insertion
   order = registration order (user_slash names first in trigger order, then
   model_sigil names in occurrence order). Plan task locks the exact iteration.
4. **Whether `SkillInjector::id()` returns `"skill"` or `"skill-injector"`.**
   Sprint 6 uses `"none"` / `"truncate"` style — pick `"skill"`.

## 15. Out of scope (explicit reminders)

These come up naturally during Sprint 7 discussion; they're NOT in scope:

- Auto-registering `scripts/*` files as runnable tools. (ADR-0023.)
- Skills referencing other skills (dependency graph). (Future, no ADR yet.)
- Per-strategy skill allowlist. (Future, post-v0.4 polish.)
- `MetricsRecorder` instrumentation for skill activation latency. (Add when
  `MetricsRecorder` plays a broader role; v0.4 SaaS scope.)
- TUI `/skill` command parity. (Sprint 9 TUI work picks this up trivially.)
- File-system watcher / hot-reload of `SKILL.md`. (Out of v0.1.)

## 16. Documentation deliverables

Within Sprint 7:

- This spec (commit at sprint start).
- ADR-0020 promoted from "Proposed — placeholder" to "Accepted"; §1/§2/§3
  fleshed out with the regex + char-cap + crate-layout decisions from this spec.
- `docs/components/H11-context-manage.md`: note that `SkillInjector` is the
  second `SystemPromptInjector` impl shipping, alongside `NoneInjector`.
- `docs/components/H06-stream-demux.md`: note the sigil-detection side-channel
  and code-fence state machine.
- `docs/components/H04-prompt-composer.md`: no change required — the
  "Available Skills" block reaches H04 through the injector suffix, not via a
  direct H04 dependency on SkillProvider.
- New `docs/skills/overview.md` (Hands-layer doc; cogito-skills is not a
  numbered Brain component, so it lives outside `docs/components/H*-*.md`).

## 17. References

- ADR-0020 placeholder: `docs/adr/0020-skill-loader.md`
- ADR-0007 (additive variants, no schema bump): `docs/adr/0007-event-log-as-cross-language-contract.md`
- ADR-0008 + Sprint 6 spec (context pipeline + SystemPromptInjector contract):
  `docs/adr/0008-context-management.md` and
  `docs/superpowers/specs/2026-05-23-sprint-6-context-management-design.md`
- 2026-05-22 roadmap rebalance §2.5 (K5 sigil), §2.6 (B-defer scripts), §7.1
  (sigil edge cases), §7.4 (namespace UX).
- agentskills.io specification (external standard cogito follows).
- Codex `codex-rs/core-skills/` (sigil regex reference; pattern attribution
  per Sprint 4 MCP precedent).
