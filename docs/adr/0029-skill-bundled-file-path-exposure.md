# ADR-0029: Expose the activated skill's bundled-file root to the model

## Status

Accepted — implemented 2026-06-01 (Phase 0 of the complete-skill-support
design). `SkillContent.root` added; `SkillRegistry` populates it from the
discovered skill directory; `SkillInjector` emits a `root="..."` attribute
plus a one-line resolution hint when present. Tests:
`cogito-skills::registry_get_carries_skill_own_directory_as_root`,
`cogito-context::injected_block_carries_skill_root_path` (and the `None`-path
guard in `user_channel_activates_from_turn_started`).

Scopes a narrow, additive fix that is a **precondition** for ADR-0023
(bundled-script execution) and for any script-bearing skill (e.g. the
upstream `pptx` skill) to function at all. Does **not** decide execution
policy — that remains ADR-0023's job.

## Context

ADR-0020 §4 (Skill loader) and ADR-0023 (bundled-script execution, deferred)
both rest on the same assumption:

> "The model can read script contents via `read_file` and execute via
> `bash` (subject to existing tool permissions)." — ADR-0020 §4, echoed in
> ADR-0023 lines 44 / 73-74.

That assumption has never held. The model is never told **where the skill
lives on disk**, so it cannot form a path to pass to `read_file` or `bash`.

Concretely, the activation data path drops the skill's own directory:

- Discovery already computes it: `DiscoveredSkill { dir, .. }` where
  `dir = entry.path()` is the skill's own folder containing `SKILL.md`
  (`crates/cogito-skills/src/discovery.rs:146,159`).
- But the protocol types that reach the Brain / model carry no such path.
  `SkillContent` (returned by `SkillProvider::get`, injected at activation)
  has only `name`, `source`, `body`
  (`crates/cogito-protocol/src/skill.rs:56-65`).
- `SkillSource::Repo { dir }` looks like it might carry it, but its doc is
  explicit: `dir` is the **workspace root** at which `.cogito/skills/` was
  found, "NOT the skill's own directory"
  (`crates/cogito-protocol/src/skill.rs:72-74`).
- The injector emits only the SKILL.md `body` text (plus the registry
  block); no path prefix (`crates/cogito-context/src/injector/skill.rs`).

So when a `SKILL.md` says "run `scripts/html2pptx.py`" or "read
`references/reference.md`", the model has nothing to resolve those relative
paths against. For a markdown-only skill (pure instructions) this is fine;
for a skill whose value **is** its bundled `scripts/` / `references/` /
`assets/` (the upstream `pptx`, `pdf`, `docx`, `xlsx` skills), the injected
SKILL.md is a manual referencing files the model cannot reach. This is the
deeper failure exposed once `/skill` routing was fixed (PR #35): activation
now injects the body correctly, but the body alone is inert for a
script-bearing skill.

The skill bundle structure that motivates this (ADR-0020 §1):

```
.cogito/skills/pptx/
  SKILL.md           # injected today
  scripts/*.py       # referenced by SKILL.md, currently unreachable
  references/*.md     # referenced by SKILL.md, currently unreachable
  assets/...
```

## Decision

Expose the activated skill's on-disk **root directory** to the model. Path
exposure only; execution policy stays in ADR-0023.

1. **Carry the root on `SkillContent`.** Add
   `root: Option<PathBuf>` to `cogito_protocol::skill::SkillContent` — the
   skill's own directory (the folder containing `SKILL.md`). `None` for
   skills with no on-disk bundle (e.g. future embedded `System` skills, or
   virtual/plugin skills materialized in memory). Discovery already has the
   value (`DiscoveredSkill.dir`); the registry stores it and
   `SkillProvider::get` returns it. Additive field; no event-schema impact
   (see point 4).

2. **Surface the path on the `<skill>` block.** When `SkillInjector` injects
   a skill body whose `root` is `Some`, it adds a `root="..."` attribute to
   the existing `<skill>` opening tag plus a one-line resolution hint, e.g.:

   ```
   <skill name="pptx" source="repo" root="/abs/.../.cogito/skills/pptx">
   Bundled files for this skill live under the root path above; resolve any
   relative path in the instructions below against it.
   ... SKILL.md body ...
   </skill>
   ```

   This is the minimum that turns the SKILL.md's relative references into
   something `read_file` / `bash` can act on. Skills with `root: None` get
   no attribute and no hint (behavior unchanged).

3. **Do not change `bash`'s default cwd.** `bash` keeps resolving `cwd`
   against the workspace root (`crates/cogito-jobs/src/bash.rs:54-56`). The
   model uses the absolute root from the header (or `cd`s explicitly).
   Rationale: a single turn may have multiple skills active, so there is no
   unambiguous "the skill cwd" to default to; an explicit absolute path is
   unambiguous.

4. **Keep the path out of the *durable identity* of a skill.** Do **not**
   add the root to the `SkillActivated` event payload (ADR-0020 §6) nor to
   `SkillMetadata`. The event log is a portable, cross-language,
   cross-machine contract (ADR-0007); machine-specific host paths must not
   become part of how a skill is identified or replayed. The root is
   resolved from the live registry at injection time. `SkillMetadata` stays
   path-free so discovery (progressive disclosure, name+description only)
   remains cheap and location-independent.

   **Caveat (be precise):** the `SkillContent.root` *field* is not itself a
   persisted event, but `SkillInjector` renders it into the system-prompt
   suffix, and that suffix **is** persisted in `SystemPromptInjected` and
   reused on resume (the injector is idempotent and reuses the existing
   event). So a concrete path does reach the event log *indirectly*, frozen
   into the rendered suffix. This is acceptable for single-machine resume
   (v0.1–v0.3). For machine-independent multi-replica resume (v0.4), the
   path must be re-resolved at prompt-build time instead of replayed from
   the frozen suffix — tracked as a v0.4 follow-up, not solved here.

5. **Scope precedence and `SkillSource::Repo.dir` are unchanged.** The
   workspace-root semantics of `SkillSource::Repo.dir` are load-bearing for
   scope precedence (ADR-0020 §2); this ADR leaves them alone and adds an
   orthogonal field rather than overloading `dir`.

This makes the ADR-0020 §4 / ADR-0023 "access via `read_file` + `bash`"
assertion actually true, and is the cheapest enabler of ADR-0023 Position A
(scripts-as-data) without committing to any execution model.

## Consequences

**Easier**:
- Script-bearing skills (`pptx`, `pdf`, `docx`, `xlsx`, …) become usable
  via the already-shipped `read_file` + `bash` tools, with no execution
  policy decision required.
- ADR-0020 §4 / ADR-0023's stated access model stops being aspirational.
- Unblocks ADR-0023 Position A as a follow-up (implicit invocation by
  script path) without forcing Position B/C.

**Harder**:
- `SkillContent` gains a field — additive, but `SkillProvider` impls and
  test fixtures must populate it.
- Injected context grows by ~1-2 lines per active skill (negligible vs. the
  existing per-skill body + 1024-char description cap).
- The model now sees a host path — minor host-layout disclosure.
  Acceptable: the model already runs `bash` on the host via `DirectExecutor`
  (ADR-0027), so this leaks nothing it could not already enumerate.
- A concrete path is frozen into the persisted `SystemPromptInjected` suffix
  (see Decision point 4 caveat); fine for single-machine resume, a tracked
  v0.4 follow-up for multi-replica.

**Given up**:
- Nothing structural. This is a strict superset of current behavior gated on
  `root.is_some()`.

## Open questions

1. Header wording / format — terse path line (above) vs. a structured
   fenced block. Lean terse to save context.
2. Structured alternative: also expose the root via a tool
   (e.g. `skill_root(name) -> path`) instead of / in addition to prompt
   text? Prompt text is zero-round-trip and matches ADR-0020 §1's
   "activation is injection, not a tool call" stance; a tool adds latency.
   Recommend prompt-text-only for now.
3. Embedded `System` skills (feature-gated, possibly `include_str!`'d) have
   no real path. When ADR-0023 lands and such a skill needs to execute
   bundled scripts, do we materialize it to a temp dir and set `root` to
   that? Deferred to ADR-0023; `root: Option` keeps the door open.
4. Should the header also list the actual bundled subdirectories
   (`scripts/`, `references/`, `assets/`) that exist, so the model does not
   guess? Costs a `read_dir` at injection time; possibly worth it.
5. Attribute escaping. The path is interpolated into the `root="..."`
   pseudo-XML attribute unescaped. A skill directory name containing `"`,
   `>`, or a newline would break the tag and inject text into the system
   prompt. Trusted for operator-authored dirs in v0.1 (a `TODO` marks the
   site); must be escaped or rejected at discovery before skill roots become
   tenant-controlled in the SaaS profile (Phase 3).

## References

- ADR-0020 — Skill loader (asserts the access model this ADR makes real;
  §4 + §6)
- ADR-0023 — Bundled-script execution (deferred; this ADR is its
  precondition, not its replacement)
- ADR-0007 — Event log as cross-language contract (why absolute paths stay
  out of the event payload)
- ADR-0027 — CommandExecutor seam and builtin scope (`bash` / `DirectExecutor`
  run on the host)
- Code touchpoints: `crates/cogito-skills/src/discovery.rs:146,159`
  (root already computed); `crates/cogito-protocol/src/skill.rs:56-74`
  (`SkillContent` / `SkillSource`); `crates/cogito-context/src/injector/skill.rs`
  (injection site); `crates/cogito-jobs/src/bash.rs:54-56` (`bash` cwd)
- Motivating skill: https://github.com/anthropics/skills/tree/main/skills/pptx
