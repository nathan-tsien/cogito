# cogito Architecture

> Production-grade Agent Runtime core, packaged as an embeddable Rust library.

## Positioning

cogito is **the core of an agent runtime — the brain of an agent — packaged
as an embeddable Rust workspace** that another Rust service depends on and
runs in-process. cogito provides:

- **Brain**: the Harness (H01–H10) that drives one iteration of the agent loop
- **Session contract**: the `ConversationStore` trait (event-sourced log) and a v0.1 backend (`cogito-store-jsonl`)
- **Hand / Boundary contracts**: `ToolProvider`, `JobManager`, `ModelGateway`, `HookHandler` traits with reference implementations

cogito does **not** provide: deployment artifacts (Docker / Helm), HTTP/gRPC
inbound transport, end-user authentication, multi-tenant isolation, quota /
billing, Web UI, RAG / vector store, or cross-session memory. Those are the
consumer's responsibility (or a future SaaS layer that wraps cogito).

The first production consumer target is **a single product feature backend**
(chat / IDE / code assistant / customer support). Per-process replica capacity
is the primary scaling unit; consumers run K replicas behind a load balancer
with `session_id` sticky routing. **cogito does not coordinate across
processes** — that is the consumer's deployment concern.

cogito must be:

1. **Resumable** — any Brain instance can pick up any session and continue
2. **Stateless across turns** — all state in the event log
3. **Pluggable** — different stores, models, tools, strategies via traits
4. **Observable** — every step recorded as an event + structured `tracing` span
5. **Recoverable** — single-session crashes are routine, never bring down the process

## The 10-component Brain

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
        Input:    │   H04 Prompt Composer       │
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
| H04 | Prompt Composer | Assemble the next `ModelInput` |
| H05 | Tool Surface Builder | Decide which tools the LLM sees this turn |
| H06 | Stream Demultiplexer | Split streaming response into typed events |
| H07 | Tool Call Resolver | Parse and schema-validate model-emitted tool calls |
| H08 | Tool Dispatcher | Invoke `ToolProvider::invoke`; route on the outcome |
| H09 | Hook Pipeline | Brain-side policy gates (Allow / Modify / Reject) |
| H10 | Strategy Selector | Produce the `HarnessStrategy` value for this turn |

## Critical dependency constraints

```
H01 Turn Driver
 ├→ H03 Resume Coordinator  (on entry)
 ├→ H10 Strategy Selector   (on entry)
 ├→ H04 Prompt Composer     (Init → PromptBuilt)
 ├→ H05 Tool Surface Builder (Init → PromptBuilt)
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
           │  H04 + H05 + H10
   ┌───────▼────────┐
   │  PromptBuilt   │
   └───────┬────────┘
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

## Workspace layout (v0.1)

| Crate | Layer | Role |
|---|---|---|
| `cogito-protocol` | Protocol | All traits, `ConversationEvent`, `ExecCtx`, `ToolDescriptor`, `InvokeOutcome`, value types. No internal cogito deps. |
| `cogito-core` | Brain + Runtime | `harness/` is Brain (H01–H10), may only `use cogito_protocol::*`. `runtime/` is the hosting platform (DI, panic catch, resource budget). |
| `cogito-store-jsonl` | Session | v0.1 sole backend: per-session JSONL files, `fsync` per event. Layout: `<root>/sessions/<session_id>.jsonl`. |
| `cogito-model` | Boundary | `ModelGateway` impls (Anthropic + OpenAI). |
| `cogito-tools` | Hands | `BuiltinToolProvider` + `CompositeToolProvider` utility. |
| `cogito-sandbox` | Hands (internal primitive) | `Sandbox` trait + subprocess impl. **Not visible to Brain** (see Hands layer section). |
| `cogito-jobs` | Hands | `JobManager` impl: tokio task + JSONL job log. |
| `cogito-mcp` | Hands (deferred) | MCP `ToolProvider` impl. Lands 0.1 late or 0.2. |
| `cogito-cli` | Surface | CLI binary; wires runtime + store + gateway. |
| `cogito-tui` | Surface (deferred) | TUI; may slide to 0.2. |
| `testing/cogito-test-fixtures` | Testing | Shared fixtures, tmp JSONL store helper. |
| `testing/cogito-mock-model` | Testing | `ModelGateway` mock with scripted responses. |

Notes:

- `cogito-conversation` (a placeholder in earlier drafts) is **superseded** by `cogito-store-jsonl`. The trait lives in `cogito-protocol`; no separate "session machinery" crate remains.
- `cogito-core` will split into `cogito-core` (Brain) + `cogito-runtime` (Runtime) when ADR-0004 §4 triggers fire. Today the boundary is enforced by module discipline.

## Brain / Hands / Session boundaries

The 10-component design describes Brain's internal structure. The **crate
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
  budgets.
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

### Trait contracts in `cogito-protocol`

| Trait | Implemented by | Defines |
|---|---|---|
| `ConversationStore` | `cogito-store-jsonl` (+ consumer-supplied) | Append-only event log read / append / range / tail |
| `ConversationEvent` (type) | (value type) | Wire format of every event, with `schema_version: u32` |
| `ModelGateway` | `cogito-model` | Streamed turn against an external LLM |
| `ToolProvider` | `cogito-tools`, `cogito-mcp`, consumer | Tool catalog + `invoke(name, args, ctx) → InvokeOutcome` |
| `JobManager` | `cogito-jobs`, consumer | Async work state tracking (no `submit` — submit happens inside ToolProvider impls) |
| `HookHandler` | (Sprint 6) | Brain-side policy gates (see H09) |

Hand-internal primitives (`Sandbox`, HTTP clients, FS adapters) do **not**
live in Protocol. They are scoped inside their owning Hand crate and used
only by Tool implementations within Hands. Brain never holds a `dyn Sandbox`.

## Hands layer internal structure

Hands has **three internal levels**. Only Level 1 is visible to Brain.

```
                    Brain (Harness)
                          │
                          │ uses only these two trait objects
                          ▼
   ┌────────────────────────────────────────────────────┐
   │  Level 1 · Brain-facing contracts (in protocol)    │
   │    · ToolProvider                                   │
   │    · JobManager                                     │
   └─────────────────────────┬──────────────────────────┘
                             │ implemented by
                             ▼
   ┌────────────────────────────────────────────────────┐
   │  Level 2 · Hand crates                             │
   │    · cogito-tools  → BuiltinToolProvider          │
   │    · cogito-mcp    → McpToolProvider              │
   │    · cogito-jobs   → JobManager impls             │
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
- Multiple providers are composed by `CompositeToolProvider` (utility in `cogito-tools`); the consumer constructs the composite and hands it to Runtime as a single `Arc<dyn ToolProvider>`.

## Tool execution classes

Tools vary on two orthogonal axes — **time** (how long the work takes) and
**output type** (what the result is shaped like):

|  | Inline value | Blob | Resource |
|---|---|---|---|
| **Instant** (µs–s) | **A** `read_file`, `now`, `parse_json` | **B** `dump_logs`, `read_large_file` | **C** `spawn_dev_server` (returns a handle) |
| **Delayed** (min–hr) | **D** `run_tests` (returns pass/fail count) | **E** `build_release` (binary + huge log) | **F** `provision_vm` |

**v0.1 covers A + D.** B/C/E/F are deferred to 0.2+.

v0.1's compromise for class B is **inline truncation**: payloads above 1 MiB
must be truncated by the tool implementation, with a `truncation_marker` left
in the event. Classes C and F (long-lived resources) and E (large async
output) require a `BlobStore` trait + `ResourceRegistry` trait, both
introduced in 0.2 (ADR-0007, ADR-0008, both TBD).

## State storage planes

cogito has **five logical storage planes**. Each has a clear owner and
lifecycle. Confusing them is a common source of design bugs.

| Plane | Stores | v0.1 owner | Cross-turn? | Cross-process resume? |
|---|---|---|---|---|
| **P1 · Event log** | All events + small `ToolResult::Output` | `ConversationStore` (JSONL impl) | ✅ | ✅ |
| **P2 · Job state** | Async job lifecycle (Pending / Running / Completed / Failed) + final result | `JobManager` (local impl) | ✅ | ✅ |
| **P3 · Blob store** | Tool artifacts above 1 MiB inline cap | **Deferred to 0.2** (new trait) | ✅ | ✅ |
| **P4 · Resource registry** | Long-lived resource handles (running processes, attached workspaces) | **Deferred to 0.2** (new trait) | ✅ | partial |
| **P5 · Workspace files** | Files the agent edits / creates | **Consumer / filesystem (never cogito)** | ✅ | ✅ |

P5 is never cogito's concern. Consumers point cogito at a workspace root;
cogito records paths in events but does not manage directory contents.

## v0.1 scope (IN / OUT)

| Concern | v0.1 in | later 0.x | permanently out | notes |
|---|:---:|:---:|:---:|---|
| Brain (H01–H10) | ✅ | | | sprint 1–6 range |
| Event sourcing + `ConversationEvent::schema_version` | ✅ | | | day 1 |
| `cogito-store-jsonl` backend | ✅ | | | sole v0.1 store |
| Postgres / HTTP storage backends | | ✅ | | 0.2 / 0.3 |
| Anthropic + OpenAI gateways | ✅ | | | reference Boundary impls |
| Builtin tools + subprocess sandbox | ✅ | | | reference Hands impls |
| Async `JobManager` (local) | ✅ | | | sprint 4 |
| MCP client as `ToolProvider` | | ✅ | | sprint 5 / 0.2 |
| Hooks (H09) | ✅ | | | sprint 6 |
| TUI surface | | ✅ | | may slide to 0.2 |
| Observability (`tracing` + `MetricsRecorder` trait) | ✅ | | | day 1 |
| OTel / Prometheus adapters | | ✅ | | optional crate |
| Per-session resource budget (timeout / mem) | ✅ | | | day 1 |
| Process-level panic catch boundary | ✅ | | | day 1 |
| Secret / PII redaction | trait + default no-op | full policy | | trait day-1; default redactor 0.2 |
| Blob store (P3) | | ✅ | | ADR-0007 (TBD) |
| Resource registry (P4) | | ✅ | | ADR-0008 (TBD) |
| Multi-tenant isolation | | | ❌ | consumer / future SaaS |
| End-user authentication | | | ❌ | consumer |
| Inbound HTTP / gRPC transport | | | ❌ | consumer |
| Deployment artifacts | | | ❌ | consumer |
| Quota / billing | | | ❌ | consumer |
| Web UI | | | ❌ | not runtime concern |
| Vector store / RAG | | | ❌ | Hand concern, consumer-side |
| Cross-session persistent memory | | tbd | | future, separate ADR |
| Skill autonomous creation | | tbd | | future, separate ADR |

## Compatibility commitments (v0.1)

- **Rust API**: pre-1.0 SemVer (0.x.y). Breaking changes allowed in minor versions; documented in `CHANGELOG.md`.
- **Event log schema**: every `ConversationEvent` carries `schema_version: u32` from day 1. 0.x allows breaking changes if accompanied by a migration tool. At 1.0 we switch to strict forward-compat (any future version must read any past version).
- **Storage HTTP wire protocol**: not in v0.1 scope (no HTTP backend yet). When introduced (0.3), versioned alongside event schema. (See ADR-0006, TBD.)

## Design references

- Anthropic Managed Agents engineering blog — Brain / Hands / Session decoupling, event-sourced session
- OpenAI Codex Rust rewrite — workspace layout, lints, testing patterns
- Our internal System Design v1.1 document

## Where to start

1. Read `AGENTS.md` for working rules and inviolable principles
2. Read `ROADMAP.md` for the current sprint
3. Read the design doc for the component you're touching: `docs/components/H0X-*.md`
4. Read the relevant ADR — especially **ADR-0004** for layer / import rules
5. Run `just test` to verify your environment
