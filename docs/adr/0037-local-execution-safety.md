# ADR-0037: Local execution safety — TUI default command-guard + env scrub

## Status

Accepted (2026-06-03), implemented. Scoped to the local developer surface
(TUI). The multi-tenant / untrusted-code story (real isolation, Credential
Broker) remains DEFERRED under ADR-0012 / ADR-0013.

Related: ADR-0027 (`CommandExecutor` seam; admission is H09's job), ADR-0012
(sandbox lifecycle, deferred), ADR-0013 (credential isolation / env-policy,
deferred — this ADR pulls forward its cheap env-allowlist half for local use),
ADR-0036 (`RuntimeBuilder` injectable seams).

## Context

`bash` is wired into the TUI tool surface by default (`cogito-tui`
`runtime_build.rs`) and runs through `DirectExecutor` (`cogito-sandbox`), which
is explicitly "not a security boundary." Two sharp edges existed for a local
developer driving an agent that can author shell commands:

1. **No command admission.** The H09 `pre_dispatch` hook point exists and can
   reject a tool call before it runs, but the runtime wired the hook pipeline
   with `Vec::new()` and `RuntimeBuilder` exposed no setter — so no guard could
   be injected. A model-authored `rm -rf /` (or a fat-fingered one) ran
   unimpeded. The `bash_audit` example hook only counts invocations; it never
   rejects.
2. **Full environment inheritance.** `DirectConfig` defaulted to
   `inherit_env: true`, so every `sh -c` child saw the entire parent
   environment — including `ANTHROPIC_API_KEY`, MCP bearer tokens, cloud
   credentials. The only alternative was `env_clear()`, which also drops
   `PATH` / `HOME` / `LANG` / `TMPDIR` and breaks ordinary commands and skill
   scripts. There was no in-between.

This ADR is the answer to the bash-exposure question that gated ADR-0012/0013
for the local case: the consumer wants `bash` available in the TUI, but with
guardrails against catastrophic operations and against reading host secrets.
The host is operator-trusted and single-tenant, so the heavy multi-tenant
isolation machinery (namespaces / containers / forward credential proxy) is
not in scope here — that stays deferred to ADR-0012/0013 for when cogito runs
untrusted or attacker-reachable code.

## Decision

Ship a light, local-only hardening layer made of three additive parts. No
Brain change beyond an injection seam; no `cogito-protocol` wire change; no
`ConversationEvent` `SCHEMA_VERSION` bump.

### 1. `RuntimeBuilder::hooks(Vec<Arc<dyn HookHandler>>)` — make H09 injectable

The hook pipeline was unreachable. Add a runtime-level setter mirroring
`RuntimeBuilder::metrics()` (ADR-0036): the runtime holds the handlers and
clones them into every session's `CompositeHookPipeline` at `open_inner`
instead of the previous hardcoded `Vec::new()`. Default stays empty (no hooks).
This is the seam every later hook (the guard below, plugin hooks, consumer
policy) plugs into.

### 2. `CommandGuardHook` — a denylist accident guard (not a security boundary)

A builtin `HookHandler` (`cogito-core::harness::hooks::command_guard`,
`name = "command-guard"`). On `pre_dispatch`, for the `bash` tool only, it reads
the `command` arg and rejects a curated denylist of catastrophic patterns
(recursive-force `rm` targeting `/`, `/*`, `~`, `$HOME`, or top-level system
dirs; fork bomb; `mkfs`; `dd of=/dev/...`; redirect to a raw block device;
`chmod -R` on root). Project-local deletes (`rm -rf ./build`, `rm -rf target`)
are allowed. All other tools and commands pass through.

It is **a denylist, by deliberate choice, not an allowlist** — a strict
command allowlist would cripple a developer TUI, and the threat model here is
accident and model mistake, not an adversary. The hook is documented in-code
as trivially bypassable (encodings, alternate tools, env indirection) and
explicitly **not a security boundary**. It complements, and does not replace,
the H09 admission seam reserved by ADR-0027 or the real isolation deferred to
ADR-0012/0013.

### 3. `EnvPolicy::Allowlist` on `DirectConfig` — default-deny secrets to `bash`

Add `EnvPolicy { InheritAll, Allowlist(Vec<String>) }` to `cogito-sandbox`.
`DirectExecutor` applies it: `InheritAll` preserves the exact v0.1 behavior
(honoring `inherit_env`); `Allowlist(keys)` starts the child from an empty
environment and copies in only the listed keys that exist in the parent.
`cogito-sandbox::default_safe_env_allowlist()` returns the curated set
(`PATH HOME LANG LC_ALL LC_CTYPE TMPDIR USER LOGNAME SHELL TERM PWD`) — the
shape ADR-0013 specified, default-deny for everything else (the secrets).

`DirectConfig::env_policy` defaults to `InheritAll` and is `#[serde(skip)]`, so
the `cogito.toml` schema is unchanged and every existing config keeps its
current behavior. The policy is set programmatically, not from TOML.

### 4. Wiring is TUI-only

`cogito-tui` is the only surface that opts in, via two small helpers in
`runtime_build.rs`: `local_safety_hooks()` (the command guard) passed to
`.hooks(...)`, and `harden_sandbox_env(...)` which sets
`EnvPolicy::Allowlist(default_safe_env_allowlist())` on the `Direct` sandbox
before `build_executor`. The CLI chat surface is intentionally left unchanged
(full inheritance, no guard) in this cut; the seams now exist to harden it the
same way later if desired.

## Consequences

What becomes easier:

- A TUI user can keep `bash` on without a fat-finger or model mistake wiping
  the host, and without `bash` reading the API keys / tokens in the host
  environment. The protection is on by default in the TUI.
- The hook pipeline is finally injectable, which is reusable far beyond this
  guard (consumer policy hooks, plugin hooks).
- The env-policy seam matches the shape ADR-0013 already designed, so the
  multi-tenant Credential Broker can extend the same `EnvPolicy` rather than
  inventing a new mechanism.

What we give up / accept:

- The command guard is a denylist and is **not a security boundary** — it stops
  accidents, not a determined or adversarial actor. That is acceptable for a
  trusted local operator and is documented as such; untrusted/multi-tenant
  execution still requires the deferred ADR-0012/0013 isolation before bash may
  be exposed to attacker-influenceable input.
- The denylist needs occasional curation as new catastrophic footguns surface;
  it is intentionally conservative (high-signal, low false-positive) and may
  miss novel patterns.
- CLI chat is not hardened in this cut (by scope choice), so its `bash` still
  inherits the full environment. The seams to fix that exist.

## Alternatives considered

- **Strict command allowlist instead of a denylist.** Rejected for the local
  TUI: it would reject the long tail of legitimate developer commands and push
  users to disable the guard entirely. A denylist of catastrophic operations
  matches the accident threat model.
- **Flip `inherit_env` to `false` by default.** Rejected: `env_clear()` also
  drops `PATH`/`HOME`/`LANG`/`TMPDIR` and breaks normal commands and skill
  scripts. The allowlist is the correct middle ground (ADR-0013 reasoning).
- **Build the real sandbox / Credential Broker now (ADR-0012/0013).** Rejected
  as out of scope: the local host is operator-trusted and single-tenant; heavy
  isolation has no trigger here. Those ADRs stay deferred until cogito runs
  untrusted or attacker-reachable code.
- **Wire the guard globally (CLI + TUI + every consumer).** Rejected for this
  cut: the decision was explicitly TUI-scoped. Consumers compose their own
  policy through the now-public `.hooks()` seam.
