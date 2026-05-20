# cogito Architecture

> Production-grade Agent Runtime core, packaged as an embeddable Rust library.

## Positioning

cogito is **the core of an agent runtime — the brain of an agent — packaged
as an embeddable Rust workspace** that another Rust service depends on and
runs in-process. cogito provides:

- **Brain**: the Harness (H01–H10) that drives one iteration of the agent loop
- **Session contract**: the `ConversationStore` trait (event-sourced log) and a v0.1 backend (`cogito-store-jsonl`)
- **Hand / Boundary contracts**: `ToolProvider`, `JobManager`, `ModelGateway`, `HookHandler`, `StorageSystem`, `BrainSpawner` traits with reference implementations
- **Multimodal content**: `Vec<ContentBlock>` payloads (Anthropic Messages API shape) with URI-addressed bulk storage
- **Subagent**: recursive Brain hosting via a 4-tool `ToolProvider` (v0.3+)

cogito does **not** provide: deployment artifacts (Docker / Helm), inbound
HTTP/gRPC transport, end-user authentication, multi-tenant isolation, quota
/ billing, Web UI, RAG / vector store, or cross-session memory. Those are
the consumer's responsibility (or a future SaaS layer that wraps cogito).

The first production consumer target is **a single product feature backend**
(chat / IDE / code assistant / customer support / multimodal task agent).
Per-process replica capacity is the primary scaling unit; consumers run K
replicas behind a load balancer with `session_id` sticky routing. **cogito
does not coordinate across processes** — that is the consumer's deployment
concern.

cogito must be:

1. **Resumable** — any Brain instance can pick up any session and continue
2. **Stateless across turns** — all state in the event log
3. **Pluggable** — different stores, models, tools, strategies, storage via traits
4. **Observable** — every step recorded as an event + structured `tracing` span
5. **Recoverable** — single-session crashes are routine, never bring down the process

## The 11-component Brain

```
                  ┌─────────────────────────────┐
                  │   Agent Runtime (shell)     │
                  │  DI · panic catch · budget  │
                  └──────────┬──────────────────┘
                             │
                  ┌──────────▼──────────────────┐
                  │       Harness (Brain)       │
                  │  ─────────────────────────  │
   Orchestration: │   H01 Turn Driver           │
                  │   H02 Step Recorder         │
                  │   H03 Resume Coordinator    │
                  │  ─────────────────────────  │
        Input:    │   H11 Context Manage        │ ← decides what context
                  │   H04 Prompt Composer       │   the model sees
                  │   H05 Tool Surface Builder  │
                  │  ─────────────────────────  │
       Output:    │   H06 Stream Demultiplexer  │
                  │   H07 Tool Call Resolver    │
                  │  ─────────────────────────  │
     Execution:   │   H08 Tool Dispatcher       │
                  │   H09 Hook Pipeline         │
                  │  ─────────────────────────  │
       Control:   │   H10 Strategy Selector     │
                  └─────────────────────────────┘
```

Each component has a dedicated design doc in `docs/components/H0X-*.md`.

| ID | Component | Single responsibility |
|---|---|---|
| H01 | Turn Driver | Drive one Loop iteration as an explicit FSM; the only coordinator |
| H02 | Step Recorder | Persist every step as an event, immediately |
| H03 | Resume Coordinator | Pure function: event log → resume state |
| H04 | Prompt Composer | Assemble the next `ModelInput` (passive, deterministic) |
| H05 | Tool Surface Builder | Decide which tools the LLM sees this turn (strategy-static) |
| H06 | Stream Demultiplexer | Split streaming response into typed events |
| H07 | Tool Call Resolver | Parse and schema-validate model-emitted tool calls |
| H08 | Tool Dispatcher | Invoke `ToolProvider::invoke`; route on the outcome |
| H09 | Hook Pipeline | Brain-side policy gates (Allow / Modify / Reject) |
| H10 | Strategy Selector | Produce the `HarnessStrategy` value for this turn |
| H11 | Context Manage | Decide context shape: compaction, system-prompt injection, tool-filter overrides; **only Brain component allowed to do I/O for summarization** (slot reserved 2026-05-19; mechanism pending ADR-0008) |

## Critical dependency constraints

```
H01 Turn Driver
 ├→ H03 Resume Coordinator  (on entry)
 ├→ H10 Strategy Selector   (on entry; produces value consumed by H11/H04/H05/H09)
 ├→ H11 Context Manage      (Init → ContextManaged)
 ├→ H04 Prompt Composer     (ContextManaged → PromptBuilt)
 ├→ H05 Tool Surface Builder (ContextManaged → PromptBuilt)
 ├→ H06 Stream Demultiplexer (ModelCalling → ModelCompleted)
 ├→ H07 Tool Call Resolver  (ModelCompleted)
 ├→ H08 Tool Dispatcher     (ToolDispatching)
 └→ H09 Hook Pipeline       (lifecycle points)

H02 Step Recorder
 ← called by every component (including H01 on each state transition)
 → depends only on the `ConversationStore` trait
```

**Critical rule**: H01 is the only coordinator. H02–H10 never call each other.

## Turn state machine

```
        ┌─────┐
        │ Init│
        └──┬──┘
           │  H10 (strategy lookup)
   ┌───────▼──────────┐
   │ ContextManaged   │
   └───────┬──────────┘
           │  H11 (context decisions; may do I/O for summarization)
   ┌───────▼────────┐
   │  PromptBuilt   │
   └───────┬────────┘
           │  H04 + H05 + H09 (pre_prompt)
           │
           │  ModelGateway (streaming)
   ┌───────▼────────┐
   │  ModelCalling  │
   └───────┬────────┘
           │  H06 (stream → events)
   ┌───────▼────────┐
   │ ModelCompleted │
   └───────┬────────┘
           │  H07 (parse) + H08 (invoke)
   ┌───────▼────────┐    ┌──────────┐
   │ToolDispatching ├───▶│  Failed  │
   └───────┬────────┘    └──────────┘
           │
      ┌────┴─────┐
      │          │
┌─────▼───┐ ┌────▼─────┐
│Completed│ │  Paused  │ (async job in flight)
└─────────┘ └──────────┘
```

Each transition writes an event to the event log **before** moving on
(ADR-0003). H03 reconstructs state by replaying the log.

**What is a finite state machine in cogito?** See
`docs/components/H01-turn-driver.md` §"What is a Finite State Machine
here?" for a primer covering the states-as-values model, the
"event-before-transition" invariant, and why this design substitutes
for cross-turn state. The Rust realization (typed-state enum
`TurnState`, single-`run()` match loop, free transition functions per
state) is sized for Sprint 2 / 3 implementation; the Sprint 2 design
discussion that locked these choices lives in
`docs/superpowers/specs/2026-05-19-sprint-2-minimal-loop-design.md` §Q5.

**`ContextManaged` state** was added 2026-05-19 by PR #6 as an ADR-0006
amendment. v0.1 Sprint 2 implements it as a pass-through (H11 not yet
implemented; immediately transitions to `PromptBuilt`). The real H11
implementation lands with the Context Management initiative (ADR-0008,
pending). See `docs/components/H01-turn-driver.md` §"Init → ContextManaged
→ PromptBuilt sequence" for the canonical H10/H11/H04/H05/H09 walkthrough.

### Resume entry path

End-to-end recovery sequence for a single session after process restart:

```
┌─ Caller (CLI / consumer) ──────────────────────────────────────────────┐
│   runtime.open_session(id, SessionMode::Resume).await                  │
└──────────────────┬─────────────────────────────────────────────────────┘
                   │
                   ▼
┌─ Runtime::open_session ────────────────────────────────────────────────┐
│  ① check in-memory registry (prevent concurrent open of same session)  │
│  ② store.range(session_id, ..).await   ← pull all events               │
│  ③ events.is_empty()? → Err(ResumeFailed: no such session)             │
│  ④ spawn SessionActor::spawn(initial_events = events)                  │
└──────────────────┬─────────────────────────────────────────────────────┘
                   │
                   ▼
┌─ SessionActor::actor_main ─────────────────────────────────────────────┐
│  ⑤ schema check (fail-fast)                                            │
│  ⑥ let decision = harness::resume::replay(&initial_events)?            │
│  ⑦ state.event_seq.store(decision.last_event_seq + 1)                  │
│  ⑧ if New session: write SessionStarted                                │
│  ⑨ apply_resume_point(decision.point):                                 │
│     - FreshTurn               → in_flight=Idle                         │
│     - RestartCurrentTurn      → spawn TurnDriver (Init-like)           │
│     - ResumeFromModelCompleted→ spawn TurnDriver (ModelCompleted)      │
│     - ResumeFromToolDispatching→ spawn TurnDriver (ToolDispatching)    │
│     - ResumePausedJob         → in_flight=PausedOnJob;                 │
│                                 job_manager.on_complete(sink)          │
│     - ResumeAfterJobCompletion→ inject result → spawn TurnDriver       │
│                                 (ToolDispatching)                      │
│  ⑩ enter mailbox main loop                                             │
└────────────────────────────────────────────────────────────────────────┘
```

**Key invariants** (correctness requirements, not preferences):

- **Step ② completes entirely in the Runtime layer**, before the actor starts. Once `actor_main` begins, it never goes back to the store to pull history. This guarantees that the `events → state` mapping in `actor_main` is a deterministic pure function (unit-testable in isolation).
- **Step ⑦ must precede step ⑨**: any write that occurs before the sequence generator is initialized may produce an event with `seq < last_event_seq`, violating the ADR-0002 append-only invariant.
- **`ResumePausedJob` branch does not spawn a `TurnDriver`**. The turn deliberately paused waiting for an external job, not for the model; spawning a `TurnDriver` immediately would cause it to terminate at once, leaving the actor to register `on_complete` on the next cycle — a bug incubator.
- **`ResumeAfterJobCompletion` is a distinct branch** (not a sub-case of `ResumeFromToolDispatching`): the former derives its completed payload from a `JobCompletedRecorded` event; the latter derives from a `ToolResultRecorded` event — the data sources are different.

> Full algorithm: `docs/components/H03-resume-coordinator.md`. Decision rationale: spec 2026-05-20-sprint-3-resume-coordinator-design.md §3 + §4.

## Brain / Hands / Session boundaries

The 11-component design describes Brain's internal structure. The **crate
graph** encodes the larger decoupling. **ADR-0004 is the authoritative spec;
this section is a summary.**

### Layer responsibilities

- **Brain** decides. No syscalls, no network, no filesystem. Reads from
  Session, calls Boundary and Hands only through trait objects supplied by
  Runtime.
- **Session** persists. Append-only event log. Single source of truth for
  cross-turn state (ADR-0002).
- **Boundary** is Brain's interface to the external thinking-aid (the LLM).
  Not Hands — Hands act on the world; Boundary lets Brain think.
- **Hands** execute side effects. Each Hand crate implements a trait defined
  in Protocol.
- **Runtime** hosts Brain instances: dependency injection of Session /
  Boundary / Hands into Brain, panic-catch boundaries, per-session resource
  budgets, implements `BrainSpawner` for subagent execution.
- **Protocol** is the only crate every other crate may depend on. It holds
  traits, event types, and shared value types.
- **Surface** wires everything into an entry point (CLI / TUI / consumer's
  service).

### Import rules

```
Protocol  ← Brain · Session · Boundary · Hands · Runtime · Surface · Testing
Brain     ← Runtime
Session   ← Runtime
Boundary  ← Runtime
Hands     ← Runtime
Runtime   ← Surface
```

(Arrows point from imported to importer.) **Brain importing a Hand
directly is a build error.** When Brain needs a new capability, add a
trait to Protocol — do not relax the rule.

## Actor model — why and how

### Why an actor model?

cogito is an embedded library (ADR-0005 §1); a single process must serve
≥1000 concurrent sessions (ADR-0005 §3 SLO). This constraint directly
eliminates the Codex-style `Arc<Session> + Mutex<ActiveTurn>` shared-state
approach — once a mutex is poisoned inside one session, every code path
accessing the same mutex stalls, violating the "single-session failure
isolation" requirement.

Five concrete constraints drive the design toward an actor model:

- **Failure isolation**: a single-session panic must not affect other sessions.
- **Caller-injected tokio `Handle`**: cogito does not call `Runtime::new()`; it accepts an external `Handle`.
- **Cooperative cancellation**: Ctrl-C terminates the current turn, not the session.
- **Dual event streams**: the durable stream (backpressure) and the broadcast stream (low-latency, lossy) have contradictory delivery contracts that cannot share one channel.
- **Async job wake-up**: the actor must respond to mailbox messages while in `PausedOnJob` state.

All five constraints point to the same solution: one actor task per session.

### Four core invariants

cogito's actor model is defined by four invariants. These are correctness
requirements, not engineering preferences:

1. **Private state**: each session's runtime state is owned exclusively by **one** task. No cross-actor `Arc<Mutex<_>>`.
2. **Message-driven**: all interaction with an actor goes through channels — mailbox (commands), broadcast (events), persist (durable writes), job sink (async wake-up). A direct function call into actor internals is a design bug.
3. **Single mutable owner**: the actor task is the sole mutator of its private state. Subtasks (`TurnDriver`, `store_writer`) receive value copies or explicit handles through channels.
4. **Cooperative termination**: cancellation goes through `CancellationToken` + `select!`, never `task.abort()`. Every await point has a chance to drop RAII guards and flush pending events.

### Topology

```
                  Caller (CLI / consumer service)
                                │
                                ▼  Arc<Runtime>
                    ┌───────────────────────────┐
                    │         Runtime            │
                    │  · session_registry        │
                    │  · DI: store / model / ... │
                    │  · panic catch boundary    │
                    └────────────┬───────────────┘
                                 │ open_session
                                 ▼
 ┌───────────────── SessionActor (one task per session) ──────────────┐
 │                                                                     │
 │      mailbox (mpsc<SessionCommand>, cap 64)                         │
 │       Input / Shutdown / Cancel / JobCompleted                      │
 │              │                                                      │
 │              ▼ FIFO drain                                           │
 │       ┌───────────────┐                                             │
 │       │  actor_loop    │── private state (in_flight, seq, ...)      │
 │       └──┬──────┬──────┘                                            │
 │          │      │  events_out (broadcast<StreamEvent>, cap 256)     │
 │          │      └────────────────────────────────────►              │
 │          │             0..N live subscribers (lossy)                │
 │   spawn  │                                                          │
 │   per-   │      persist_tx (mpsc<PersistCommand>, cap 256)          │
 │   turn   │             │                                            │
 │          ▼             ▼                                            │
 │   TurnDriver task    store_writer subtask (serial fsync)            │
 │   (FSM run loop)              │                                     │
 │                               ▼                                     │
 │                       ConversationStore (JSONL / Postgres / ...)    │
 │                                                                     │
 │      job_completion_rx (mpsc<JobCompletionEvent>, cap 32)           │
 │       ◄── JobManager.on_complete(job_id, sink) callbacks            │
 └─────────────────────────────────────────────────────────────────────┘
```

### Advantages in cogito's context

- **Failure isolation falls to the scheduler layer**: tokio unwinds a panicking task independently; other sessions are completely unaffected. This is a prerequisite for the ADR-0005 §3 SLO of ≥1000 concurrent sessions.
- **Backpressure is first-class**: channel capacities (64 / 256 / 256) are explicit SLO knobs. Slow consumers observe `Lagged(n)` and self-diagnose; there is no silent unbounded growth.
- **Cancellation is verifiable**: every await point is guarded by `select!`, RAII guards drop normally — contrasted with `task.abort()`, which leaves half-written state.
- **Scaling unit is clear**: one process = N actors; multiple processes = sticky `session_id` routing. cogito does not coordinate across actors within a process (that is the consumer's deployment concern); scaling out actors adds almost zero coordination overhead.
- **Resume is local**: a single-session crash only requires rebuilding one actor (Sprint 3 H03 + `actor_main` flow). A shared-state design would require reconstructing cross-session lock state — a fundamentally different complexity class.

### Trade-offs

- **Per-session baseline memory**: tokio task stack + 3–4 channels + private state ≈ 10–30 KiB (idle, not running a turn). This is a known cost, not a surprise.
- **Mailbox FIFO vs. cancel priority**: `cancel_turn` cannot queue behind a large backlog — ADR-0006 §3 solves this with a direct `CancellationToken` signal that bypasses the mailbox.
- **Boilerplate**: managing 4 channel types + drain protocol is roughly 30% more LoC than `Arc<Mutex>`. The correctness guarantees justify this.
- **Cross-actor debugging requires structured tracing**: each actor needs its own span; otherwise mailbox-ordered log lines are misleading. This is a mandatory operational discipline, not optional.

> Cross-refs: ADR-0006 §1 (decision), §3 (cancellation), §4 (channels), §5 (job wake-up); spec 2026-05-20-sprint-3-resume-coordinator-design.md §7.

## Hands layer internal structure

Hands has **three internal levels**. Only Level 1 is visible to Brain.

```
                    Brain (Harness)
                          │
                          │ uses only the protocol-level traits
                          ▼
   ┌────────────────────────────────────────────────────┐
   │  Level 1 · Brain-facing contracts (in protocol)    │
   │    · ToolProvider                                   │
   │    · JobManager                                     │
   │    · HookHandler                                    │
   └─────────────────────────┬──────────────────────────┘
                             │ implemented by
                             ▼
   ┌────────────────────────────────────────────────────┐
   │  Level 2 · Hand crates                             │
   │    · cogito-tools  → BuiltinToolProvider          │
   │    · cogito-mcp    → McpToolProvider              │
   │    · cogito-jobs   → JobManager impls             │
   │    · cogito-subagent → SubagentToolProvider (0.3) │
   │    · cogito-tools-multimedia (0.2+)               │
   └─────────────────────────┬──────────────────────────┘
                             │ internally use
                             ▼
   ┌────────────────────────────────────────────────────┐
   │  Level 3 · Hand-internal primitives (NOT in proto)│
   │    · Sandbox (cogito-sandbox)                     │
   │    · HTTP / FS adapters                           │
   └────────────────────────────────────────────────────┘
```

Design notes:

- **`ToolProvider::invoke(name, args, ctx)`** returns `InvokeOutcome::Sync(ToolResult)` or `InvokeOutcome::Async(JobId)`. The provider implementation decides which path; H08 dispatches based on the variant.
- **`JobManager`** exposes `status` / `result` / `cancel`. It does **not** expose `submit` — async tool implementations are the only producers of jobs; they own the submit path internally.
- **`Sandbox`** is Hands-internal. Brain never holds a `dyn Sandbox`. Tool implementations that need subprocess isolation inject one.
- **`StorageSystem`** and **`BrainSpawner`** are Brain-adjacent: in `cogito-protocol`, but Brain does not call them directly. Tools receive `StorageSystem` via `ExecCtx`; `SubagentToolProvider` receives `BrainSpawner` via DI from Runtime.
- Multiple providers are composed by `CompositeToolProvider` (utility in `cogito-tools`); the consumer constructs the composite and hands it to Runtime as a single `Arc<dyn ToolProvider>`.

## Content blocks

Every message payload (user messages, assistant responses, tool results)
in cogito is a **`Vec<ContentBlock>`** (Anthropic Messages API shape), not
a plain string. This is the foundation for multimodal content support and
matches the wire format of the primary target provider.

### ContentBlock variants (in `cogito-protocol`)

```text
· Text(String)
    The default. Existing event paths default to this; serde defaults lift
    a legacy `text` field to `[Text(text)]` for backward compatibility.

· ToolUse { call_id, name, args: serde_json::Value }
    Model-emitted tool invocation. Nests inside the assistant message's
    content list (Anthropic-style, not OpenAI Responses' separate-item).

· ToolResult { call_id, content: Vec<ContentBlock>, is_error: bool }
    Tool result returned to the model. Content is recursive — a tool
    result can carry text plus an image, for example.

· Image { uri: String, mime: String, source_hint: SourceHint }
    Model-visible image content. URI is opaque. Tool implementations
    that produce model-visible images must set
    `ToolDescriptor.outputs_model_visible_multimodal = true`.

· (Reserved) Video / Audio
    Variant slots reserved for when LLM providers natively support
    video/audio content blocks. Out of v0.x scope.
```

### Constraints

- **`Vec<ContentBlock>` carries URI strings only, never raw bytes.** When the LLM provider requires raw bytes (e.g., Anthropic image blocks), the `ModelGateway` adapter resolves the URI via `StorageSystem` at serialization time and base64-encodes inline. The internal cogito surface stays URI-only.
- **Tool outputs are `Text` by default.** Tools that want to produce model-visible multimodal content must opt in via the `outputs_model_visible_multimodal` flag on `ToolDescriptor`. H05 / H10 / hooks can check this flag to refuse such tools when the active model has no native multimodal capability.
- **Schema evolution is additive.** New `ContentBlock` variants are b-档 compatible (serde tagged union with `#[serde(other)]`).

The model maps 1:1 to Anthropic Messages API. The OpenAI adapter
unwraps `ToolUse` / `ToolResult` blocks to top-level Responses API items
at serialization time — that is a per-provider adapter concern, not a
protocol concern.

## StorageSystem — the third protocol pillar

Beyond `ConversationStore` (Session) and `ToolProvider` (Hands), cogito
has a **third top-level protocol abstraction**: `StorageSystem`, which
mediates all non-text I/O via opaque URI strings.

### Why

Multimodal scenarios (video / audio / large file content) cannot ride
inline in the event log — a 30s 1080p video is ~30 MB raw or ~40 MB
base64-encoded; inlining it in every prompt is prohibitive. cogito
therefore decouples bulk content from the event log: **events carry URI
strings only**; the bytes live in a storage backend the URI resolves
against.

### Trait shape (design level)

```text
StorageSystem behaviors:

· resolve(uri) -> BlobMeta
    Probe the URI; return size, mime, etag, canonical_uri.
    For https:// may issue HEAD; for file:// is stat().

· open(uri) -> AsyncRead
    Streaming read. Implementations may fetch+cache or stream-proxy.

· create(mime, AsyncRead) -> Uri
    Streaming write; returns canonical URI (typically `blob://...`).

No pin / unpin / gc — lifecycle is the backend's internal concern, not a
protocol contract.
```

### URI scheme conventions (protocol-level)

| Scheme | Meaning | Producer |
|---|---|---|
| `file:///abs/path` | Local FS path | User input / workspace tools |
| `http(s)://...` | Remote URL; backend chooses caching | User input / tools |
| `blob://<id>` | Backend-internalized blob | `StorageSystem::create()` |
| `s3://bucket/key` | Object storage (later backend) | Future |
| `mcp://server/resource/...` | MCP resource (later) | `cogito-mcp` |
| (custom) | Consumer-defined backends | Consumer |

**Brain does not parse URIs.** URIs are opaque strings to Brain;
resolution is `StorageSystem` + tool implementation territory.

### Lifecycle (deliberately not in protocol)

Unlike `ConversationStore` events (which are durable forever),
**`StorageSystem` URI resolvability is not guaranteed across time**:

- A `file://` path may be deleted by the user
- An `https://` URL may link-rot
- A `blob://` may be GC'd by the backend per its own policy

This matches how Claude Code and Codex handle their bulk content (both
rely on the user's filesystem with no separate tracking). If a specific
URI must remain resolvable across replay, consumers should either use a
backend that retains content (S3 with versioning, vs ephemeral local
cache), or pin the content by copying it into a controlled backend at
ingestion time.

cogito does not guarantee replay fidelity for sessions referencing URIs
that have since become unresolvable. The fundamentals — event log and
job state — remain durable; URI content is a consumer/backend concern.

### Interaction with `ExecCtx`

Every tool invocation receives an `ExecCtx` that includes
`storage: Arc<dyn StorageSystem>`. Tools call `ctx.storage.open(...)` and
`ctx.storage.create(...)` to read user inputs and write artifacts. Brain
never accesses storage directly.

### Interaction with subagents

A subagent's Runtime is given the same `Arc<dyn StorageSystem>` as the
parent. Blobs created by a child are visible to the parent (URIs are
process-wide handles). When a child returns `ContentBlock::Image { uri }`
in its result, the URI is portable — the parent can pass it to other
tools or feed it to its model.

## Tool execution classes

Tools vary on two orthogonal axes — **time** (how long the work takes)
and **output type** (what the result is shaped like):

|  | Inline value | Blob | Resource |
|---|---|---|---|
| **Instant** (µs–s) | **A** `read_file`, `now`, `parse_json` | **B** `dump_logs`, `read_large_file` | **C** `spawn_dev_server` (returns a handle) |
| **Delayed** (min–hr) | **D** `run_tests`, `transcribe_audio` | **E** `build_release` (binary + huge log) | **F** `provision_vm` |

**v0.1 covers A + D.** Classes B / E (large outputs) are unlocked by
**v0.2** when `StorageSystem` lands (blob outputs reference URIs). Classes
C / F (long-lived resources) are deferred to **v1.x** via a future
`ResourceRegistry` (P4 plane).

v0.1's compromise for class B is **inline truncation**: payloads above 1
MiB must be truncated by the tool implementation, with a
`truncation_marker` left in the event. v0.2 lifts this by routing big
outputs through `StorageSystem::create()` and returning a `blob://` URI.

## State storage planes

cogito has **five logical storage planes**. Each has a clear owner and
lifecycle. Confusing them is a common source of design bugs.

| Plane | Stores | Owner | Cross-turn? | Cross-process resume? |
|---|---|---|---|---|
| **P1 · Event log** | All events + small `ToolResult::Output` (inline text + URIs) | `ConversationStore` (JSONL in v0.1; Postgres / HTTP later) | ✅ | ✅ |
| **P2 · Job state** | Async job lifecycle (Pending / Running / Completed / Failed) + final result | `JobManager` (local in v0.1; distributed in v0.4+) | ✅ | ✅ |
| **P3 · Storage system** | Non-text bulk content (audio / video / large blobs) addressed by URI | `StorageSystem` (cogito-storage-local in v0.2; S3 / HTTP later) | depends on backend | depends on backend |
| **P4 · Resource registry** | Long-lived resource handles (running processes, attached workspaces) | **Deferred to v1.x** (new trait + new ADR) | ✅ | partial |
| **P5 · Workspace files** | Files the agent edits / creates in the working tree | **Consumer / filesystem (never cogito)** | ✅ | ✅ |

P5 is never cogito's concern. Consumers point cogito at a workspace
root; cogito records paths in events but does not manage directory
contents.

## Subagent layer (v0.3+)

A subagent is **a recursive Brain instance hosted by the same Runtime**.
From the parent's perspective, the subagent is exposed as a `ToolProvider`
with four tools. From cogito's perspective, the subagent's lifecycle is
managed entirely by Runtime + `JobManager` — no new top-level concept.

### Tools exposed to the LLM (via `cogito-subagent`)

| Tool | Outcome | Pauses parent? |
|---|---|---|
| `spawn_agent(role, task, handed_tools?)` | `Sync(SubagentHandle { agent_id })` | ❌ child runs in background |
| `wait_agent(agent_id, timeout?)` | `Async(JobId)` | ✅ until child completes or timeout |
| `send_input(agent_id, message)` | `Sync("queued")` | ❌ |
| `cancel_agent(agent_id)` | `Sync("cancelled" \| "already_done")` | ❌ |

The decoupled spawn/wait pattern naturally supports fan-out: parent
spawns N children, then `wait_agent` for each. No batch-spawn tool is
needed at v0.3; it can be added later if a workload demands it.

### Session tree model

```text
session_root (depth=0)
  ├── session_a1 (depth=1, parent=root, role=planner)
  │     └── session_a1a (depth=2, parent=a1, role=worker)
  ├── session_a2 (depth=1, parent=root, role=coder)
  └── session_a3 (depth=1, parent=root, role=critic)
```

Event attribution:

- **Subagent lifecycle** events (`SubagentSpawned`, `SubagentInputSent`, `SubagentCompleted`) are written to the **parent** session log only.
- **Subagent internal** events (`TurnStarted`, `ModelCallCompleted`, etc.) are written to the **child** session log only.
- Cross-session relation is recoverable from either side (parent log carries `child_session_id`; child metadata carries `parent_session_id`).

### `BrainSpawner` trait — the layer-rule seam

Hands cannot import Runtime (ADR-0004 layer rule). To let
`SubagentToolProvider` spawn a child Brain, `cogito-protocol` defines a
`BrainSpawner` trait:

```text
trait BrainSpawner {
    fn spawn(&self, child_session_id, strategy, parent_depth) -> JobId;
    fn cancel(&self, job_id);
}
```

`cogito-core::runtime` implements `BrainSpawner`; `SubagentToolProvider`
receives an `Arc<dyn BrainSpawner>` via DI at construction. Brain itself
remains protocol-only.

### Crash recovery

| Failure | Recovery |
|---|---|
| Parent Brain panics | Runtime catch_unwind; parent turn → Failed; children continue independently |
| Child Brain panics | Runtime catch_unwind on child task; `JobFailed { reason: ChildPanicked }`; parent sees `ToolResult::Error { kind: AsyncFailed }` |
| Process restart | Runtime enumerates Paused sessions; queries `JobManager` and child session state; resumes parents whose children completed; restarts children from their own event logs if mid-turn |

Crash recovery uses no subagent-specific logic — it's the standard
event-sourcing + `JobManager` recovery model from ADR-0002 + ADR-0003.

### Depth limit

Each session's metadata carries `depth`. Spawning a child sets
`child.depth = parent.depth + 1`. Strategy can override
`max_subagent_depth` (default 3). Exceeding the limit returns
`ToolResult::Error { kind: DepthExceeded }`.

### Hand passing

`spawn_agent`'s optional `handed_tools` parameter exposes a subset of the
parent's `ToolProvider` to the child, in addition to the child role's
default toolset. Implementation composes a derived `CompositeToolProvider`
for the child. Deferred to a later 0.x release; v0.3 ships
role-defaults only.

### Strategy = role

Subagent roles are not a new concept. A subagent role is a
`HarnessStrategy` (loaded from `strategies/*.yaml`) with two extra
optional fields:

- `spawnable_as_subagent: bool` (default `false`) — explicit opt-in to spawnable role
- `max_subagent_depth: u32` (default 3) — per-role depth budget

H10 Strategy Selector owns strategy loading; subagent spawn just asks
for a strategy by name.

## Workspace layout

| Crate | Layer | When | Role |
|---|---|---|---|
| `cogito-protocol` | Protocol | v0.1 | All traits, `ConversationEvent`, `ContentBlock`, `ExecCtx`, `ToolDescriptor`, `InvokeOutcome`, value types. No internal cogito deps. |
| `cogito-core` | Brain + Runtime | v0.1 | `harness/` is Brain (H01–H10), may only `use cogito_protocol::*`. `runtime/` is the hosting platform (DI, panic catch, resource budget, `BrainSpawner` impl). |
| `cogito-store-jsonl` | Session | v0.1 | First backend: per-session JSONL files, `fsync` per event. Layout: `<root>/sessions/<session_id>.jsonl`. |
| `cogito-store-postgres` | Session | v0.4 | Production multi-replica backend. |
| `cogito-store-http` | Session | v0.6 | Generic HTTP-backed adapter against the Storage HTTP wire protocol (ADR-0015). |
| `cogito-model` | Boundary | v0.1 | `ModelGateway` impls (Anthropic + OpenAI). Handles ContentBlock ↔ provider format serialization. |
| `cogito-tools` | Hands | v0.1 | `BuiltinToolProvider` + `CompositeToolProvider` utility. |
| `cogito-tools-multimedia` | Hands | v0.2+ | Audio / video / image tools (transcribe, summarize, extract_frames, describe_image, ...). |
| `cogito-sandbox` | Hands (internal primitive) | v0.1 | `Sandbox` trait + subprocess impl. **Not visible to Brain**. |
| `cogito-jobs` | Hands | v0.1 | `JobManager` impl: tokio task + JSONL job log. |
| `cogito-mcp` | Hands | v0.2 | MCP `ToolProvider` adapter. |
| `cogito-subagent` | Hands | v0.3 | `SubagentToolProvider` implementing the 4 subagent tools. |
| `cogito-storage-local` | Hands (Storage) | v0.2 | First `StorageSystem` backend: local FS + HTTP fetch with cache + `blob://` mapped to local cache dir. |
| `cogito-storage-s3` | Hands (Storage) | v0.4 | S3-compatible object storage backend. |
| `cogito-storage-http` | Hands (Storage) | v0.6 | Generic HTTP-backed storage adapter. |
| `cogito-cli` | Surface | v0.1 | CLI binary; wires runtime + store + gateway. |
| `cogito-tui` | Surface | v0.2 | TUI. |
| `cogito-observability-otel` | Surface (optional) | v0.4 | OpenTelemetry adapter that ships `MetricsRecorder` impl + trace exporter. |
| `crates/testing/cogito-test-fixtures` | Testing | v0.1 | Shared fixtures, tmp JSONL store helper. |
| `crates/testing/cogito-mock-model` | Testing | v0.1 | `ModelGateway` mock with scripted responses. |

Notes:

- `cogito-conversation` (a placeholder in earlier drafts) is **superseded** by `cogito-store-jsonl`. The trait lives in `cogito-protocol`; no separate "session machinery" crate remains.
- `cogito-core` will split into `cogito-core` (Brain) + `cogito-runtime` (Runtime) when ADR-0004 §4 triggers fire (e.g., a second Runtime is needed, or Brain is tempted to peek into Runtime internals). Today the boundary is enforced by module discipline.

## Trait contracts in `cogito-protocol`

| Trait | Implemented by | Defines | When |
|---|---|---|---|
| `ConversationStore` | `cogito-store-*` crates + consumer | Append-only event log read / append / range / tail | v0.1 |
| `ConversationEvent` (type) | (value type) | Wire format of every event, with `schema_version: u32` and `Vec<ContentBlock>` content | v0.1 |
| `ContentBlock` (type) | (value type) | Tagged union of `Text` / `ToolUse` / `ToolResult` / `Image` / ... | v0.1 (Text + ToolUse + ToolResult); `Image` lands v0.2 |
| `ModelGateway` | `cogito-model::anthropic` + `cogito-model::openai_compat` (v0.1 Sprint 2); future provider adapters | `async fn stream(input, ctx) -> BoxStream<Result<ModelEvent, ModelError>>`; provider adapter pre-aggregates per-content-block sealed events (`TextBlockCompleted`, `ToolUseCompleted`, `MessageCompleted`). See `cogito-protocol::gateway`. | v0.1 |
| `ModelInput` / `ModelOutput` / `ModelEvent` / `Message` / `ModelParams` / `StopReason` / `Usage` / `ModelError` (types) | (value types in `cogito-protocol::gateway`) | Provider-agnostic shapes consumed by `ModelGateway`; `Message` is `User { content: Vec<ContentBlock> }` ｜ `Assistant { content: Vec<ContentBlock> }` — tool_result lives inside `Message::User` per Anthropic semantics | v0.1 |
| `HarnessStrategy` / `ToolFilter` (types) | (value types in `cogito-protocol::strategy`) | Per-turn behavior knobs: name, system_prompt, allowed_tools, tool_order, model_params, max_turns. v0.1 Sprint 2 ships `default_with_model` factory; Sprint 5 adds YAML registry. | v0.1 |
| `ExecCtx` (type) | (value type in `cogito-protocol::exec_ctx`) | Per-invocation context handed to every tool/hook: `session_id`, `turn_id`, `deadline: Option<Instant>`, `cancel: CancellationToken`. v0.2 adds `storage`; v0.4 adds `tenant`. | v0.1 |
| `ToolProvider` | `cogito-tools` / `cogito-mcp` / `cogito-subagent` / consumer | Tool catalog + `invoke(name, args, ctx) → InvokeOutcome` | v0.1 |
| `JobManager` | `cogito-jobs` / consumer | Async work state tracking (`status` / `result` / `cancel`) plus mailbox-injected completion callback (`on_complete`). Submission lives on the concrete `LocalJobManager` type per ADR-0004 (Hands-internal). | v0.1 |
| `HookHandler` | (Sprint 6) | Brain-side policy gates (see H09) | v0.1 |
| `StreamEvent` (type) | (value type) | Real-time event stream observable via `SessionHandle::subscribe()`; broadcast fanout; per-chunk text deltas; not persisted (see spec §7) | v0.1 |
| `ExecutionClass` (type) | (value type) | `ToolDescriptor.execution_class` ∈ {`AlwaysSync`, `AlwaysAsync`, `Adaptive`}; H08 uses it to validate `InvokeOutcome` variant (see spec §6) | v0.1 |
| `TurnOutcome` / `TurnFailureReason` (types) | (value types) | Terminal turn states + structured failure reasons returned by the actor (see spec §9) | v0.1 |
| `StorageSystem` | `cogito-storage-*` / consumer | Non-text I/O via URI strings: `resolve` / `open` / `create` | v0.2 |
| `BrainSpawner` | `cogito-core::runtime` | Recursive Brain spawning — used only by `cogito-subagent` | v0.3 |
| `MetricsRecorder` | `cogito-observability-otel` / consumer | Pluggable metrics sink (no hard Prometheus dep) | v0.4 |

> **Harness-internal value types** (`TurnState`, `TurnCtx`, `TurnDeps`,
> `ResumeDecision`, `ToolInvocation`, `ResolvedCall`, `DispatchOutcome`)
> live in `cogito-core::harness::*` rather than `cogito-protocol`. They
> are not part of the cross-crate / cross-language contract — they are
> the Brain's internal wiring between H01 and H03/H07/H08. See the
> Sprint 2 design spec §Q1 for the placement rationale.

Hand-internal primitives (`Sandbox`, HTTP clients, FS adapters) do **not**
live in Protocol. They are scoped inside their owning Hand crate and used
only by Tool implementations within Hands. Brain never holds a
`dyn Sandbox`.

## Version evolution path

cogito's roadmap is version-driven, not experiment-driven. Each version
adds a specific capability without breaking prior protocol guarantees
(within the b-档 compatibility window for 0.x).

| Version | Theme | What's added |
|---|---|---|
| **v0.1** | Foundation | 7 core crates + JSONL store + Anthropic gateway + minimal tools (`read_file`, etc.) + 11-component Brain skeleton + state machine + chaos test |
| **v0.2** | Storage + Multimodal | `StorageSystem` trait + `cogito-storage-local` + full `Vec<ContentBlock>` upgrade + `ExecCtx.storage` field + `cogito-tools-multimedia` starter (one tool: `transcribe_audio`) + MCP adapter |
| **v0.3** | Subagent | `BrainSpawner` trait + `cogito-subagent` crate + 4 subagent tools + session metadata (`parent_session_id`, `depth`) + new `ConversationEvent` variants |
| **v0.4** | SaaS-ready | `cogito-store-postgres` + `cogito-storage-s3` + `TenantContext` (optional field on `ExecCtx`) + `MetricsRecorder` trait + `cogito-observability-otel` + resource budget enforcement + ADR-0012 / 0013 (sandbox lifecycle, credential isolation) |
| **v0.5** | Multimedia breadth | Expand `cogito-tools-multimedia` (extract_frames, summarize_video, describe_image, analyze_frame, synthesize_speech) + opt-in `model_visible` ContentBlock wired through ModelGateway adapters |
| **v0.6** | Hardening | Hook policy maturity + load tests + soak tests + migration tooling docs + `cogito-storage-http` + Storage HTTP wire protocol (ADR-0015) |
| **v1.0** | API freeze | Public API stability commitment + event log forward-compat strict mode + 1.0 GA release |
| **v1.x+** | Advanced | Resource Registry (P4) + cross-brain hand sharing + real-time video + generative video + MCP resources/prompts/sampling |

### ADR docket

| ADR | Subject | Status / Trigger |
|---|---|---|
| ADR-0001 | Workspace layout | Accepted (v0.1) |
| ADR-0002 | Event-sourced conversation log | Accepted (v0.1) |
| ADR-0003 | State-machine Turn Driver | Accepted (v0.1) |
| ADR-0004 | Brain / Hands / Session crate boundaries | Accepted (v0.1) |
| **ADR-0005** | **Production scope, quality gates, SLO posture, compatibility commitments** | **Accepted (v0.1)** |
| **ADR-0006** | **Runtime + H01 Turn Driver execution model** | **Accepted (v0.1); amended 2026-05-19 (PR #6) — `ContextManaged` FSM state added** |
| **ADR-0007** | **Event log as cross-language storage contract** | **In flight (PR #6, v0.1 Sprint 1)** |
| ADR-0008 | Context Management (`H11 Context Manage` mechanism: compaction, system-prompt injection, tool-filter overrides) | TBD — initiative post-Sprint 2; architectural slot locked by ADR-0006 amendment |
| ADR-0009 | `StorageSystem` trait + URI scheme + `ContentBlock` upgrade | TBD (v0.2) — renumbered from ADR-0007 by PR #6 |
| ADR-0010 | Multimedia tool conventions (MIME, `model_visible` flag, etc.) | TBD (v0.2) — renumbered from ADR-0008 |
| ADR-0011 | Subagent execution model (BrainSpawner + 4 tools + session tree) | TBD (v0.3) — renumbered from ADR-0009 |
| ADR-0012 | Sandbox lifecycle (lazy provisioning, pets-vs-cattle) | TBD (v0.4) — renumbered from ADR-0010 |
| ADR-0013 | Credential isolation (sandbox proxy pattern, vault integration) | TBD (v0.4) — renumbered from ADR-0011 |
| ADR-0014 | TenantContext propagation + multi-tenant SaaS conventions | TBD (v0.4) — renumbered from ADR-0012 |
| ADR-0015 | Storage HTTP wire protocol (originally ADR-0006 → ADR-0013 → ADR-0015 across renumberings) | TBD (v0.6) |

## v0.1 scope (IN / OUT)

| Concern | v0.1 in | later 0.x | permanently out | notes |
|---|:---:|:---:|:---:|---|
| Brain (H01–H10) | ✅ | | | sprint 1–6 range |
| Event sourcing + `ConversationEvent::schema_version` | ✅ | | | day 1 |
| `cogito-store-jsonl` backend | ✅ | | | sole v0.1 store |
| Postgres / HTTP storage backends | | ✅ | | v0.4 / v0.6 |
| `Vec<ContentBlock>` (Text + ToolUse + ToolResult) | ✅ | | | day 1 |
| `ContentBlock::Image` + opt-in multimodal | | ✅ | | v0.2 / v0.5 |
| `StorageSystem` trait | | ✅ | | v0.2 (ADR-0007) |
| Anthropic + OpenAI gateways | ✅ | | | reference Boundary impls |
| Builtin tools + subprocess sandbox | ✅ | | | reference Hands impls |
| Async `JobManager` (local) | ✅ | | | sprint 4 |
| MCP client as `ToolProvider` | | ✅ | | v0.2 |
| Subagent layer (`cogito-subagent`) | | ✅ | | v0.3 (ADR-0011) |
| Hooks (H09) | ✅ | | | sprint 6 |
| TUI surface | | ✅ | | may slide to v0.2 |
| Observability (`tracing` + `MetricsRecorder` trait) | ✅ | | | day 1 |
| OTel / Prometheus adapters | | ✅ | | v0.4 |
| Per-session resource budget (timeout / mem) | ✅ | | | day 1 |
| Process-level panic catch boundary | ✅ | | | day 1 |
| Secret / PII redaction | trait + default no-op | full policy | | trait day-1; default redactor v0.2 |
| Blob store (P3) — via `StorageSystem` | | ✅ | | v0.2 |
| Resource registry (P4) | | tbd | | v1.x; ADR pending |
| Multi-tenant isolation | | | ❌ | consumer / future SaaS |
| End-user authentication | | | ❌ | consumer |
| Inbound HTTP / gRPC transport | | | ❌ | consumer |
| Deployment artifacts | | | ❌ | consumer |
| Quota / billing | | | ❌ | consumer |
| Web UI | | | ❌ | not runtime concern |
| Vector store / RAG | | | ❌ | Hand concern, consumer-side |
| Cross-session persistent memory | | tbd | | future, separate ADR |

## Compatibility commitments

See **ADR-0005** for the authoritative version of these commitments.

- **Rust API**: pre-1.0 SemVer (0.x.y). Breaking changes allowed in minor versions; documented in `CHANGELOG.md`. At 1.0 we commit to SemVer-strict.
- **Event log schema**: every `ConversationEvent` carries `schema_version: u32` from day 1. 0.x allows breaking changes if accompanied by a migration tool. At 1.0 we switch to strict forward-compat (any future version must read any past version).
- **Content blocks**: new variants are additive (b-档 compatible). Removing variants is a major version event.
- **StorageSystem URI resolvability**: not guaranteed across time; lifecycle is the backend's concern.
- **Storage HTTP wire protocol**: defined at v0.6 (ADR-0015); independent versioning from event log schema.

## Design references

- Anthropic Managed Agents engineering blog — Brain / Hands / Session decoupling, event-sourced session
- Anthropic Messages API — `ContentBlock` shape
- OpenAI Codex Rust rewrite — workspace layout, lints, testing patterns; subagent execution model reference
- Claude Code — multi-typed subagent system, agent definition format

## Where to start

1. Read `AGENTS.md` for working rules and inviolable principles
2. Read `ROADMAP.md` for the current version and sprint
3. Read the design doc for the component you're touching: `docs/components/H0X-*.md`
4. Read the relevant ADR — especially **ADR-0004** for layer / import rules, **ADR-0005** for quality gates, and **ADR-0006** for the runtime + H01 execution model
5. For runtime / threading / lifecycle questions, the detailed reference is `docs/superpowers/specs/2026-05-18-runtime-h01-execution-model-design.md` (ADR-0006 is the durable contract; the spec is the full discussion)
6. Run `just test` to verify your environment
