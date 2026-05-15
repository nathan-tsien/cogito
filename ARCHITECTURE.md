# cogito Architecture

> An experimental Rust project validating the 10-component Harness design.

## What is being validated

The "Harness" is the core subsystem inside an Agent Runtime that drives one
iteration of the agent loop. It must be:

1. **Resumable** — any instance can pick up any session and continue
2. **Stateless across turns** — all state in the event log
3. **Pluggable** — different models, tools, strategies via interfaces
4. **Observable** — every step recorded as an event
5. **Recoverable** — crashes are routine, not catastrophic

This codebase is *not* a product. It exists to verify the design before
we build a production agent platform.

## The 10 components

```
                  ┌─────────────────────────────┐
                  │   Agent Runtime (shell)     │
                  │  rehydrator · locks · bus   │
                  └──────────┬──────────────────┘
                             │
                  ┌──────────▼──────────────────┐
                  │       Harness               │
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

## Component responsibilities

| ID | Component | Single responsibility |
|---|---|---|
| H01 | Turn Driver | Drive one Loop iteration as a state machine |
| H02 | Step Recorder | Persist every step as an event |
| H03 | Resume Coordinator | Decide where to resume from an event log |
| H04 | Prompt Composer | Assemble the next ModelInput |
| H05 | Tool Surface Builder | Decide which tools the LLM sees this turn |
| H06 | Stream Demultiplexer | Split streaming response into typed events |
| H07 | Tool Call Resolver | Parse and validate model-emitted tool calls |
| H08 | Tool Dispatcher | Route to sync/async execution paths |
| H09 | Hook Pipeline | Trigger lifecycle hooks |
| H10 | Strategy Selector | Pick the HarnessStrategy for this model |

## Critical dependency constraints

```
H01 Turn Driver
 ├→ H03 Resume Coordinator  (on start)
 ├→ H10 Strategy Selector   (on start)
 ├→ H04 Prompt Composer     (PromptBuilt state)
 ├→ H05 Tool Surface Builder (PromptBuilt state)
 ├→ H06 Stream Demultiplexer (ModelCalling state)
 ├→ H07 Tool Call Resolver  (ModelCompleted state)
 ├→ H08 Tool Dispatcher     (ToolDispatching state)
 └→ H09 Hook Pipeline       (at lifecycle points)

H02 Step Recorder
 ← called by ALL components
 → depends on Conversation Service only

H10 Strategy Selector
 → no Harness dependencies; produces a Strategy value
 ← consumed by other components, but never calls them
```

**Critical rule**: H01 is the only coordinator. H02–H10 do not call each other.

## Workspace layout

| Crate | Role |
|---|---|
| `cogito-core` | Harness + Agent Runtime |
| `cogito-protocol` | Events, contracts, types |
| `cogito-conversation` | Conversation Service (SQLite + in-memory) |
| `cogito-model` | Model Gateway (Anthropic + OpenAI) |
| `cogito-tools` | Tool catalog and builtin tools |
| `cogito-sandbox` | Subprocess-based sandbox |
| `cogito-jobs` | Async job manager |
| `cogito-mcp` | MCP client (added Sprint 5+) |
| `cogito-cli` | CLI entry point |
| `cogito-tui` | TUI (Sprint 6+) |
| `testing/cogito-test-fixtures` | Test fixtures |
| `testing/cogito-mock-model` | Mock model for integration tests |

## Turn states

```
        ┌─────┐
        │ Init│
        └──┬──┘
           │
   ┌───────▼────────┐
   │  PromptBuilt   │
   └───────┬────────┘
           │
   ┌───────▼────────┐
   │  ModelCalling  │
   └───────┬────────┘
           │
   ┌───────▼────────┐
   │ ModelCompleted │
   └───────┬────────┘
           │
   ┌───────▼────────┐    ┌──────────┐
   │ToolDispatching ├───▶│  Failed  │
   └───────┬────────┘    └──────────┘
           │
      ┌────┴─────┐
      │          │
┌─────▼───┐ ┌────▼─────┐
│Completed│ │  Paused  │
└─────────┘ └──────────┘
```

Each transition writes an event to the Conversation Log *before* moving on.

## Design references

- Anthropic Managed Agents engineering blog (Brain / Hands / Session decoupling)
- OpenAI Codex Rust rewrite (workspace layout, lints, testing patterns)
- Our internal System Design v1.1 document

## What this project is NOT

- Not a production agent platform
- Not a multi-tenant SaaS
- Not optimized for token cost
- Not feature-complete (we'll cut features ruthlessly)
- Not optimized for performance until we have measurements

## Where to start

1. Read `AGENTS.md` for working rules
2. Read `ROADMAP.md` for the current sprint
3. Read the design doc for the component you're touching: `docs/components/H0X-*.md`
4. Run `just test` to verify your environment
5. Make the smallest change that validates a hypothesis
