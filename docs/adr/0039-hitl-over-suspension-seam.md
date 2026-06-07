# ADR-0039: Human-in-the-loop is a consumer flow over the suspension seam

## Status

Proposed (2026-06-07) — design + boundary decision. The only proposed core
change is one additive, observation-only `JobStatus` variant (Decision 5);
everything else is documentation of a pattern that already works on existing
mechanisms. No HITL feature is added to core.

Related: ADR-0003 (FSM `Paused` state), ADR-0006 (Runtime resume-on-job-
completion), ADR-0025 (Hands sub-layer; `JobManager` / `LocalJobSubmitter`
split), ADR-0007 (additive wire-contract evolution), ADR-0014 (tool/exec authz
is out of core scope; tenant is a `SessionMeta` constant), ADR-0037
(`CommandGuardHook` — synchronous H09 admission), ADR-0034 (session reopen).

## Context

A controllable harness must let a turn **suspend pending an external party** and
**resume deterministically**. cogito already has exactly this primitive, built
for async background jobs:

```
ToolProvider::invoke → InvokeOutcome::Async(JobId)
  → dispatcher records EventPayload::JobSubmitted, registers JobManager::on_complete
  → TurnState::Paused { job_id }  →  EventPayload::TurnPaused { job_id } (persisted)
  → … process may crash and restart …
  → H03 replay() → ResumePoint::ResumePausedJob → re-register on_complete
  → JobCompletionEvent { JobOutcome::Success { result: ToolResult } }
  → ResumeAfterJobCompletion injects `result` as the tool result; turn resumes
```

The question this ADR settles: should cogito *implement* human-in-the-loop
flows — Manus-style `message_ask_user`, or Codex/Claude/Hermes-style tool
**approval gates**?

The decision frame is deliberately narrow. cogito owns the **controllable
harness mechanism**, not the **HITL flow or its policy**. The bar is only that a
HITL flow be *implementable on existing mechanisms*. It is. Both "ask the user a
question" and "wait for an approval decision" are structurally identical to an
async job — *the turn cannot proceed until an external party supplies a
resolution* — where the "job" happens to be a human. No new harness mechanism is
required for the common case.

## Decision

### 1. cogito does not add HITL features

No built-in `message_ask_user`, no approval UI, no approval policy, no
permission modes, no "ask to continue" prompt. HITL is a **consumer flow**. This
is the same boundary as ADR-0014 (tool/exec authorization out of core scope) and
ADR-0037 (the consumer composes policy through the exposed seams). This ADR
exists primarily to *record that boundary* so HITL is not later bolted into
core.

### 2. The async-suspension seam is the HITL substrate

Two reference patterns — documented here, **not shipped** as core code:

**(a) Ask-user (Manus `message_ask_user`).** The consumer registers an
`ask_user` `ToolProvider` (`ExecutionClass::AlwaysAsync`). On `invoke` it records
a pending question keyed to a `JobId` and submits a future
(`LocalJobSubmitter::submit_boxed`, or a durable submitter — Decision 4) that
**resolves when the human answers**, to `JobOutcome::Success { result:
ToolResult::text(answer) }`, and returns `InvokeOutcome::Async(job_id)`. The
harness pauses (`TurnPaused`) and, when the reply arrives, the registered
`on_complete` fires and the turn resumes with the answer as the tool result. A
CLI consumer resolves the future on `stdin`; a SaaS consumer resolves it on an
async reply event (webhook / queue). Manus's non-blocking `message_notify_user`
is simply a synchronous tool returning `InvokeOutcome::Sync` immediately.

**(b) Approval gate (Codex / Claude / Hermes).** The consumer wraps its real
`ToolProvider` in a decorator (a `CompositeToolProvider`-style shim). For a gated
call, instead of invoking the real tool synchronously, the decorator returns
`Async(job_id)` and parks a future pending a human decision: **approved** → run
the real tool and return its result; **denied** → `ToolResult::Error { message }`
(which the model sees and can react to, matching Codex `ReviewDecision` / Claude
`PermissionResultDeny` semantics). The approval *policy* (what needs approval)
and the *decision UI* are the consumer's.

### 3. Reuse the job seam rather than add a first-class human-suspension state

The job seam already provides durable pause, crash-safe resume (chaos-tested),
and result injection. Adding a parallel suspension state / event / resume arm
(e.g. generalizing `Paused { job_id }` into `Paused { reason }`) would duplicate
the **most safety-critical code in the system** — the H03 resume coordinator —
for no behavioral gain over "humans-as-jobs." The cost we accept is semantic: a
human wait is recorded in the log as a "job." That blur is softened by one
optional affordance (Decision 5), not by forking the resume path.

### 4. The durable-`JobManager` contract (the real requirement for SaaS HITL)

The harness pause/resume is durable-correct — `TurnPaused` is persisted before
any side effect, and `replay()` reconstructs `ResumePausedJob` and re-registers
`on_complete`. But the **bundled** `LocalJobManager` (`cogito-jobs`) drives the
waiting future *in-process* (`submit_boxed`). If the process restarts while
parked on a human (any SaaS deployment; human latency is minutes to days), that
in-process future is gone, `on_complete` re-registration hits
`JobError::UnknownJob`, and the turn is stranded.

This is an implementation property, **not a harness gap.** A consumer that needs
HITL to survive process restarts MUST inject a `JobManager` that guarantees:

- **Durable pending-resolution** — the pending question / approval is persisted
  (not an in-process future) and survives restart.
- **Resumable `on_complete`** — after restart, `on_complete(job_id, sink)`
  re-attaches to the persisted pending-resolution and still delivers **exactly
  one** `JobCompletionEvent` when the human eventually answers, honoring the
  resume coordinator's `ResumePausedJob` → re-register contract.
- **Durable `result()` / `status()`** — reflect persisted state, not in-process
  memory.
- **Abandonment / timeout policy** — the manager defines what happens if the
  human never answers (e.g. a timeout → `JobOutcome::Failed` / `Cancelled`, or an
  external `JobManager::cancel`). cogito imposes none; `JobOutcome` / `JobError`
  are `#[non_exhaustive]` and reserve room (e.g. `TimedOut`).

This is precisely the seam `job.rs` already anticipates ("Implementations live
in `cogito-jobs` (v0.1 local) and `cogito-jobs-distributed` (v0.4 Redis-
backed)"). cogito's contribution is to **specify** this contract, not to
implement it.

### 5. One additive, observation-only affordance: `JobStatus::AwaitingInput`

Reserve a new `JobStatus::AwaitingInput` variant. `JobStatus` is already
`#[non_exhaustive]` and its doc comment anticipates states like `Suspended`. It
is reported only by `JobManager::status()` — a **query**, never on the FSM or
resume path — so this is purely additive and off the critical path: no
`TurnPaused` payload change, no resume-coordinator change, no `SCHEMA_VERSION`
bump. A HITL-capable `JobManager` MAY report `AwaitingInput` for a job parked on
a human, so operators and SaaS dashboards can answer *"which sessions are waiting
on a person"* without overloading `Running`. Optional for implementations. This
is the only core change proposed by this ADR.

### 6. Out of scope (the consumer's, not core's)

The ask-user / approval tools themselves; the prompting and UX; who is allowed
to answer; tenant binding of the answerer (ADR-0014: tenant is a `SessionMeta`
constant captured at open time); approval policy and permission modes; the
durable `JobManager` implementation; HITL timeout/abandonment policy.

## Consequences

What becomes easier:

- HITL ships **today** on the existing mechanism. A CLI consumer builds
  `message_ask_user` in a few dozen lines (an `AlwaysAsync` tool + a stdin-
  resolved future). The Manus async ask-user semantics and the Codex/Claude
  approval semantics both reduce to the one suspension seam.
- Core stays small and policy-free; the mechanism-vs-policy boundary
  (ADR-0014/0037) is preserved and now explicit for HITL.
- SaaS builders get a written contract (Decision 4) to implement a durable
  `JobManager` against, plus an observability hook (Decision 5).

What we give up / accept:

- A human wait is modeled as a "job" (semantic blur), softened only by the
  optional `AwaitingInput` status.
- SaaS HITL is **gated on a durable `JobManager` the consumer must supply.** The
  bundled `LocalJobManager` is CLI-grade (in-process) and will strand a parked
  turn across a restart. Documented, not fixed here.
- Approval via the tool-decorator pattern places the approval point at the
  **tool boundary**, not the H09 `pre_dispatch` hook. A consumer wanting
  hook-layer (pre-dispatch) *async* approval would need a future `HookDecision`
  extension — noted as a possible later harness-control mechanism, deliberately
  not built now (see Alternatives).

## Alternatives considered

- **First-class human-suspension primitive** — generalize `Paused { job_id }` →
  `Paused { reason: AsyncJob | AwaitingUserInput | AwaitingApproval }`, add a
  `TurnSuspended` event, a new resume arm, and `SessionHandle::resolve()`.
  Rejected: it duplicates the chaos-tested resume coordinator and grows the event
  schema for **no behavioral gain** over the job seam, which already gives
  durable pause + crash-safe resume + result injection. The honesty win is
  captured far more cheaply by the optional `AwaitingInput` status. Revisit only
  if HITL semantics diverge materially from compute jobs (e.g. multi-party
  answers, streamed partial responses, per-question ACLs).
- **Ship a built-in `message_ask_user` / approval tool in `cogito-tools`.**
  Rejected: that is HITL flow/policy (prompting, UX, who-answers) — placed with
  the consumer by Decision 1. A reference implementation belongs in a consumer or
  an `examples/` directory, not in core.
- **Extend H09 `HookDecision` with an async `RequireApproval` / `Suspend`
  variant now.** Rejected for this cut: hooks are synchronous pure gates
  (Allow/Reject, ADR-0037); making a hook suspend the turn breaks that contract,
  and it is unnecessary because the tool-decorator pattern (Decision 2b) already
  implements approval on the existing seam. Reserved as a possible future
  mechanism if hook-layer pre-dispatch approval becomes a hard requirement.
- **Bake a turn-level "ask to continue" or a HITL timeout into core.** Rejected:
  timeout / abandonment is consumer policy owned by the durable `JobManager`.
  cogito reserves `JobOutcome` / `JobError` variants for it but imposes none.
