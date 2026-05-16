# H01 · Turn Driver

> **Status**: 🚧 Not implemented · Sprint 2–3

## Role in Harness

Drive one iteration of the agent loop as an **explicit finite state
machine**. The only coordinator inside Brain — H02 through H10 do not call
each other; H01 calls them.

## State machine

```
Init → PromptBuilt → ModelCalling → ModelCompleted
                                      │
                                      ▼
              ┌─── ToolDispatching ───┤
              │                       │
              ▼                       ▼
        {Completed | Paused}        Failed
```

| State | Entered when | H01 calls |
|---|---|---|
| `Init` | Turn starts (new or resumed) | H10 (strategy), H03 (resume decision) |
| `PromptBuilt` | Prompt + tool surface assembled | H04 (compose), H05 (surface), H09 (`pre_prompt`) |
| `ModelCalling` | ModelGateway streaming started | H06 (demux) per chunk |
| `ModelCompleted` | Stream ended with `stop_reason` | H07 (resolve tool calls), H09 (`post_model`) |
| `ToolDispatching` | One or more tool calls present | H08 (invoke), H09 (`pre_dispatch`) |
| `Completed` | Model returned `end_turn` without tools | H09 (`post_turn`) |
| `Paused` | A tool returned `InvokeOutcome::Async(JobId)` | H09 (`post_turn`); turn re-entered when job finishes |
| `Failed` | Unrecoverable error or hook `Reject` | H09 (`on_error`) |

## Interface (design level)

- `Turn::run(req: TurnRequest) -> TurnOutcome`
- `TurnRequest { session_id, input: NewMessage(text) | ResumeAfter(job_id) }`
- `TurnOutcome { Completed | Paused { reason } | Failed { error_kind, message } }`
- Implementation is async; one in-flight turn per session at a time (enforced by Runtime, not by H01).

## Dependencies

**Calls (out)**:
- H03 Resume Coordinator — once on entry, decides starting state
- H10 Strategy Selector — once on entry, produces `HarnessStrategy` value
- H04 Prompt Composer, H05 Tool Surface Builder — at `Init → PromptBuilt`
- H06 Stream Demultiplexer — during `ModelCalling`
- H07 Tool Call Resolver — at `ModelCompleted`
- H08 Tool Dispatcher — at `ToolDispatching`
- H09 Hook Pipeline — at `pre_prompt`, `pre_dispatch`, `post_model`, `post_turn`, `on_error`
- H02 Step Recorder — at **every** state transition and every meaningful sub-step
- `ModelGateway::stream(model_input, ctx)` — at `PromptBuilt → ModelCalling`

**Called by**: Runtime layer (a session task spawned by `cogito-core::runtime`).

## Critical invariants

1. **Every state transition writes an event before the transition completes.** If a crash happens after the event is durably recorded but before the next call returns, H03 must be able to reconstruct state from the event alone.
2. **Brain never propagates `Err` upward from tool / hook calls.** All failures arrive as `ToolResult::Error` or `HookDecision::Reject` and are recorded as events; only Runtime-level errors (DI failure, store I/O failure) escape as `TurnOutcome::Failed`.
3. **Panics in tools, hooks, or gateways are caught at the Runtime boundary** (panic-catch around the H01 task). A panic fails one turn; it never brings down the process.
4. **Same session may be entered by another Brain instance after crash.** H03 alone decides where to resume; H01 must accept that decision and start from there without checking external state.
5. **Tools dispatched sequentially in v0.1.** Parallel dispatch is a 0.x option gated by a strategy flag.

## Open design questions

- Pause semantics for the consumer: does the consumer see `Paused { job_id }` and decide whether to await/cancel, or does the Runtime auto-resume on `JobCompleted`? Initial answer: Runtime auto-resumes via a subscription to `JobManager`; consumer just polls turn state.
- Multiple tool calls in one model response: dispatch order = order emitted by model. If one fails, do remaining tools still run? Initial answer: yes (all dispatched, each gets its own `ToolResult`); the model decides next turn based on full result set.

## Testing strategy

- **Unit**: each state-transition function tested in isolation with mocked dependencies (`MockToolProvider`, `MockModelGateway`, in-memory store).
- **Integration**: full turn against scripted mock model + scripted tool provider; verify the event sequence matches the golden trace.
- **Chaos** (`crates/cogito-core/tests/resume_chaos.rs`): inject a crash between every adjacent pair of events; on restart, H03 + H01 must reach a semantically equivalent end-state.
- **Property**: arbitrary turn scripts produce event sequences that satisfy "every state transition is preceded by its corresponding event".

## References

- ARCHITECTURE.md §"Turn state machine"
- ADR-0003 (state-machine Turn Driver)
- AGENTS.md §"Inviolable design principles" #1, #3, #4
