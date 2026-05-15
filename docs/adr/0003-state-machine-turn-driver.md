# ADR-0003: Turn Driver as explicit state machine

## Status

Accepted

## Context

The Turn Driver (H01) executes one iteration of the agent loop. This
involves: prompt composition → model call → stream processing → tool call
resolution → dispatch. A naive implementation is a function chain.

Function chains have a fatal flaw for our use case: when the process
crashes mid-chain, we cannot resume. The chain has no addressable "current
position." We can only restart from the beginning.

## Decision

Implement Turn Driver as an explicit finite state machine with states:

```
Init → PromptBuilt → ModelCalling → ModelCompleted
                                       │
                                       ▼
              ┌─── ToolDispatching ────┤
              │                        │
              ▼                        ▼
        Completed/Paused            Failed
```

Each transition writes an event to the log *before* the transition. The
Resume Coordinator (H03) is a pure function from event log to state,
returning the state at which to resume.

## Consequences

- **Easier**: crash recovery is well-defined; tests can target specific
  transitions; chaos testing can inject faults at named points
- **Harder**: more boilerplate than a function chain
- **Given up**: the "natural" Rust style of `async fn` chains for this code

We accept this because the experiment is specifically validating
resumability — making it implicit defeats the purpose.
