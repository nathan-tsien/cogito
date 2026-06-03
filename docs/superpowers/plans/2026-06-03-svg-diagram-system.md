# SVG Diagram System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ASCII box-drawing diagrams in the finalized docs (README, ARCHITECTURE, component docs, and the configuration overview) with a uniform set of hand-authored SVG figures, with zero new build dependencies.

**Architecture:** Each diagram is a standalone `.svg` text file under `docs/diagrams/`, drawn from a shared CSS-class design system embedded in every file (self-contained light "card" background so it reads on GitHub light *and* dark themes; color = layer, nested box = responsibility scope, arrow = flow). Markdown embeds the file via `<img alt="…">`. Information-dense diagrams keep their original ASCII in a collapsed `<details>` block for grep/accessibility. SVGs are produced by reproducing the existing ASCII source (referenced by file:line) using the shared template; nothing is invented.

**Tech Stack:** Hand-written SVG 1.1 + inline CSS. No toolchain. Dev-only verification with stdlib `xml.dom.minidom` (XML well-formedness) and an optional `cairosvg` render-to-PNG for visual eyeballing (never committed, never in CI). Spec: `docs/superpowers/specs/2026-06-03-svg-diagram-system-design.md`.

---

## Conventions used by every task

**Canonical style block** — every new `.svg` starts by copying this `<style>` + `<defs>` and a full-viewBox `card` rect. It is the single source of the design tokens; do not fork values per file.

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 W H" role="img" aria-label="ONE-SENTENCE DESCRIPTION">
  <style>
    text { font-family: -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif; fill: #1f2328; }
    .mono { font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace; }
    .h1 { font-size: 15px; font-weight: 700; }
    .sub { font-size: 11px; fill: #57606a; }
    .grp { font-size: 12px; font-weight: 700; fill: #5a6b8c; }
    .note { font-size: 10.5px; fill: #6e7781; font-style: italic; }
    .edge { font-size: 11px; fill: #57606a; }
    .card { fill: #f6f8fa; stroke: #d0d7de; }
    .div  { stroke: #dde3ea; stroke-width: 1; }
    .flow { stroke: #7d8590; stroke-width: 2; fill: none; }
    /* layer fills: copy only the ones a given diagram uses */
    .runtime  { fill: #e7ebf0; stroke: #7d8590; }   /* Runtime  = slate  */
    .brain    { fill: #eef1fe; stroke: #4c6ef5; }   /* Brain    = indigo */
    .session  { fill: #e6f4ea; stroke: #2da44e; }   /* Session  = green  */
    .boundary { fill: #f3eefe; stroke: #8250df; }   /* Boundary = purple */
    .hands    { fill: #fdf3e3; stroke: #d4a72c; }   /* Hands    = amber  */
    .protocol { fill: #e3f1f1; stroke: #1b9e9e; }   /* Protocol = teal   */
    .comp { fill: #ffffff; stroke: #c7d0e0; }
    .ok   { fill: #e6f4ea; stroke: #2da44e; }
    .pause{ fill: #fdf3e3; stroke: #d4a72c; }
    .fail { fill: #fbe9e9; stroke: #cf222e; }
  </style>
  <defs>
    <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M0,0 L10,5 L0,10 z" fill="#7d8590"/>
    </marker>
  </defs>
  <rect class="card" x="2" y="2" width="W-4" height="H-4" rx="12"/>
  <!-- diagram body -->
</svg>
```

The two existing samples `docs/diagrams/harness-layers.svg` and `docs/diagrams/turn-fsm.svg` are the reference implementations of this style — open them when in doubt.

**Embed snippet** (relative path depends on the doc's location):

```markdown
<img src="REL/diagrams/NAME.svg" alt="ALT TEXT" width="W">
```
- From `README.md` and `ARCHITECTURE.md` (repo root): `REL = ./docs`
- From `docs/components/*.md` and `docs/configuration/*.md`: `REL = ..`

**`<details>` fallback** (only for diagrams flagged "dense"): place directly under the `<img>`, keeping the original ASCII verbatim:

````markdown
<details><summary>Text version</summary>

```text
...original ASCII block, unchanged...
```
</details>
````

**Validate-all command** (run after editing any svg):

```bash
for f in docs/diagrams/*.svg; do python3 -c "import xml.dom.minidom; xml.dom.minidom.parse('$f')" && echo "OK   $f" || echo "FAIL $f"; done
```
Expected: every line `OK …`, no `FAIL`.

**Render-and-eyeball** (dev only; PNG goes to `/tmp`, never committed):

```bash
pip install --quiet --user cairosvg 2>/dev/null
python3 -c "import cairosvg; cairosvg.svg2png(url='docs/diagrams/NAME.svg', write_to='/tmp/NAME.png', output_width=900)"
```
Then open `/tmp/NAME.png` (Read tool) and confirm: no text overflow outside boxes, no overlapping labels, arrows land on box edges, colors match the layer mapping.

**"Verify-or-skip" guard:** A few flagged blocks may be Rust code or trait signatures rather than true diagrams. Before converting any block, read it. If it is source code / a type signature (not boxes-and-arrows), **leave it as-is and check the box noting "kept as code".** Only genuine boxes-and-arrows diagrams become SVG.

**Layer → color reference:** Runtime=slate, Brain=indigo, Session=green, Boundary=purple, Hands=amber, Protocol=teal, Surface=slate. Semantic terminals: Completed=green, Paused=amber, Failed=red.

---

## Task 1: Foundation — diagrams directory conventions

**Files:**
- Create: `docs/diagrams/README.md`
- (Already present from brainstorming: `docs/diagrams/harness-layers.svg`, `docs/diagrams/turn-fsm.svg`)

- [ ] **Step 1: Write the conventions doc**

Create `docs/diagrams/README.md`:

```markdown
# docs/diagrams

Hand-authored SVG figures for the finalized docs (README, ARCHITECTURE,
`docs/components/*`, `docs/configuration/overview.md`). No build step: each
`.svg` is committed directly and rendered by GitHub and editors.

## Conventions

- One diagram per file, `kebab-case.svg`.
- Copy the shared `<style>` + `<defs>` block from `harness-layers.svg`; do not
  fork the design tokens. Color = layer (Runtime slate, Brain indigo, Session
  green, Boundary purple, Hands amber, Protocol teal). Nested box = scope.
  Arrow = flow. Monospace (`.mono`) for code identifiers.
- Every file has a full-viewBox `.card` background so it reads on GitHub light
  and dark themes. Do not maintain separate dark variants.
- Embed with `<img src="..." alt="...">`; never inline `<svg>` (GitHub strips it).
- Information-dense diagrams keep their original ASCII in a `<details>` block
  beneath the image for grep/accessibility.

## Verifying

```bash
# XML well-formedness (stdlib, no deps)
for f in docs/diagrams/*.svg; do python3 -c "import xml.dom.minidom; xml.dom.minidom.parse('$f')"; done
```

Visual check is dev-only via `cairosvg` (not committed, not in CI).
```

- [ ] **Step 2: Validate the two existing samples**

Run the Validate-all command.
Expected: `OK   docs/diagrams/harness-layers.svg` and `OK   docs/diagrams/turn-fsm.svg`.

- [ ] **Step 3: Commit**

```bash
git add docs/diagrams/README.md docs/diagrams/harness-layers.svg docs/diagrams/turn-fsm.svg
git commit -m "docs(diagrams): add diagrams dir, conventions, and first two SVG figures"
```

---

## Task 2: ARCHITECTURE.md — layers, FSM, dependency constraints, import rules

**Files:**
- Create: `docs/diagrams/harness-deps.svg`, `docs/diagrams/crate-import-rules.svg`
- Reuse: `docs/diagrams/harness-layers.svg`, `docs/diagrams/turn-fsm.svg`
- Modify: `ARCHITECTURE.md` (blocks at `:40`, `:106`, `:85`, `:232`)

Source ASCII → target SVG:
- `ARCHITECTURE.md:40` (under "## The 11-component Brain") → reuse **harness-layers.svg**
- `ARCHITECTURE.md:106` (under "## Turn state machine") → reuse **turn-fsm.svg**
- `ARCHITECTURE.md:85` (under "## Critical dependency constraints") → new **harness-deps.svg** (H01 as root, arrows to H03/H10/H11/H04/H05/H06/H07/H08/H09 with the entry-point labels; H02 shown as called-by-all). Brain indigo.
- `ARCHITECTURE.md:232` (under "### Import rules") → new **crate-import-rules.svg** (layered: Protocol at base, arrows Protocol←all, Brain←Runtime, Session←Runtime, Boundary←Runtime, Hands←Runtime, Runtime←Surface; color each node by its layer)

- [ ] **Step 1: Author `harness-deps.svg`** from the ASCII at `ARCHITECTURE.md:85` using the canonical style. `aria-label`: "H01 Turn Driver calls H03, H10, H11, H04, H05, H06, H07, H08, H09 at defined points; H02 Step Recorder is called by every component."
- [ ] **Step 2: Author `crate-import-rules.svg`** from `ARCHITECTURE.md:232`. `aria-label`: "Import rules: every layer depends on Protocol; Brain, Session, Boundary and Hands are imported by Runtime; Runtime is imported by Surface."
- [ ] **Step 3: Validate** — run Validate-all. Expected: all `OK`.
- [ ] **Step 4: Render-and-eyeball** `harness-deps` and `crate-import-rules`. Fix overflow/overlap if any.
- [ ] **Step 5: Embed in `ARCHITECTURE.md`.** Replace each of the four ```` ``` ```` ASCII blocks with the `<img>` snippet (`REL = ./docs`). Alt text = the matching `aria-label`. None of these four need a `<details>` fallback (they are compact and the surrounding prose already lists the components).
  - layers block → `<img src="./docs/diagrams/harness-layers.svg" alt="..." width="780">`
  - FSM block → `<img src="./docs/diagrams/turn-fsm.svg" alt="..." width="580">`
  - deps block → `harness-deps.svg`
  - import-rules block → `crate-import-rules.svg`
- [ ] **Step 6: Check embeds resolve** —

```bash
for n in harness-layers turn-fsm harness-deps crate-import-rules; do test -f docs/diagrams/$n.svg && echo "ok $n" || echo "MISSING $n"; done
grep -c 'diagrams/.*\.svg' ARCHITECTURE.md
```
Expected: four `ok`, and grep count ≥ 4.

- [ ] **Step 7: Commit**

```bash
git add docs/diagrams/harness-deps.svg docs/diagrams/crate-import-rules.svg ARCHITECTURE.md
git commit -m "docs(arch): SVG figures for layers, turn FSM, dependency constraints, import rules"
```

---

## Task 3: ARCHITECTURE.md — resume entry path & actor topology (dense, with fallback)

**Files:**
- Create: `docs/diagrams/resume-entry-path.svg`, `docs/diagrams/actor-topology.svg`
- Modify: `ARCHITECTURE.md` (blocks at `:164`, `:278`)

Both are information-dense → **keep the original ASCII in a `<details>` block** under the image.

- [ ] **Step 1: Author `resume-entry-path.svg`** from `ARCHITECTURE.md:164` (under "### Resume entry path"): three stacked swimlane boxes (Caller → `Runtime::open_session` R1–R4 → `run_session` A1–A6), arrows top-to-bottom. Runtime slate, mono for step ids. `aria-label`: "Resume sequence: caller opens a session in Resume mode; Runtime pulls all events then spawns the session actor, which replays the log and applies the resume point."
- [ ] **Step 2: Author `actor-topology.svg`** from `ARCHITECTURE.md:278` (under "### Topology"): the per-session actor with its channels (mailbox / broadcast / persist / job sink) and subtasks (TurnDriver, store_writer). `aria-label`: "Per-session actor topology: one actor task owns private state and communicates only through mailbox, broadcast, persist and job-sink channels."
- [ ] **Step 3: Validate** — Validate-all → all `OK`.
- [ ] **Step 4: Render-and-eyeball** both. Dense diagrams — confirm no label collisions; widen viewBox if cramped.
- [ ] **Step 5: Embed + fallback.** Replace the two ASCII blocks with `<img>` (`REL = ./docs`) immediately followed by a `<details><summary>Text version</summary>` block containing the **unchanged** original ASCII.
- [ ] **Step 6: Verify fallback present** —

```bash
grep -c '<details><summary>Text version' ARCHITECTURE.md
```
Expected: ≥ 2.

- [ ] **Step 7: Commit**

```bash
git add docs/diagrams/resume-entry-path.svg docs/diagrams/actor-topology.svg ARCHITECTURE.md
git commit -m "docs(arch): SVG figures for resume entry path and actor topology (with text fallback)"
```

---

## Task 4: ARCHITECTURE.md — structure diagrams (verify-or-skip)

**Files:**
- Create (only for genuine diagrams): `docs/diagrams/hands-internal.svg`, `docs/diagrams/content-blocks.svg`, `docs/diagrams/storagesystem-trait.svg`, `docs/diagrams/subagent-session-tree.svg`
- Modify: `ARCHITECTURE.md` (blocks at `:365`, `:416`, `:467`, `:632`)

Apply the **verify-or-skip guard** to each — `:416` (ContentBlock variants) and `:467` (StorageSystem trait shape) may be type listings rather than diagrams.

- [ ] **Step 1: Inspect all four blocks.** Read `ARCHITECTURE.md:365,416,467,632`. For each, decide diagram vs. code. Record the decision in the commit message.
- [ ] **Step 2: `hands-internal.svg`** from `:365` (under "## Hands layer internal structure") — Hands amber container with the internal Hand crates as boxes. `aria-label`: "Hands layer internal structure: tool providers, jobs, sandbox, storage and MCP composed behind Protocol traits."
- [ ] **Step 3: `subagent-session-tree.svg`** from `:632` (under "### Session tree model") — parent session node with child subagent session nodes; Brain indigo. `aria-label`: "Subagent session tree: a parent session spawns child subagent sessions, each its own event log."
- [ ] **Step 4: `content-blocks.svg` / `storagesystem-trait.svg`** — only if Step 1 judged them genuine diagrams; otherwise skip and leave the block as code.
- [ ] **Step 5: Validate** — Validate-all → all `OK`.
- [ ] **Step 6: Render-and-eyeball** each created svg.
- [ ] **Step 7: Embed** the created ones (`REL = ./docs`); add `<details>` fallback only if a block exceeds ~25 lines. Leave skipped blocks untouched.
- [ ] **Step 8: Commit**

```bash
git add docs/diagrams/*.svg ARCHITECTURE.md
git commit -m "docs(arch): SVG figures for hands structure and session tree (kept type listings as code)"
```

---

## Task 5: README.md — system overview hero diagram

**Files:**
- Create: `docs/diagrams/system-overview.svg`
- Modify: `README.md` (insert after the intro paragraph, before "## What's inside" at `README.md:15`)

- [ ] **Step 1: Author `system-overview.svg`** — the six layers as labeled, color-coded boxes (Surface → Runtime → Brain, with Session / Boundary / Hands attached through Protocol), one line each summarizing the layer's job, matching the "## What's inside" bullets. Use all six layer colors from the reference table. `aria-label`: "cogito layers: a Surface drives the Runtime, which injects Session, Boundary and Hands into the Brain through Protocol traits."
- [ ] **Step 2: Validate** — Validate-all → `OK`.
- [ ] **Step 3: Render-and-eyeball** at width 820. Confirm legibility and that the six layer colors are distinguishable.
- [ ] **Step 4: Embed in README.** After line 13 (end of intro paragraph), insert:

```markdown

<img src="./docs/diagrams/system-overview.svg" alt="cogito layers: a Surface drives the Runtime, which injects Session, Boundary and Hands into the Brain through Protocol traits" width="820">
```
No `<details>` needed (README stays visual; the "## What's inside" list is the text equivalent).

- [ ] **Step 5: Commit**

```bash
git add docs/diagrams/system-overview.svg README.md
git commit -m "docs(readme): add SVG system-overview hero diagram"
```

---

## Task 6: H01 Turn Driver component doc

**Files:**
- Create: `docs/diagrams/h01-init-sequence.svg`, `docs/diagrams/h01-module-structure.svg`, `docs/diagrams/h01-call-graph.svg`
- Reuse: `docs/diagrams/turn-fsm.svg`
- Modify: `docs/components/H01-turn-driver.md` (blocks at `:52`, `:117`, `:270`, `:303`)

Source → target (`REL = ..`):
- `:52` (under "## State machine") → reuse **turn-fsm.svg** (same FSM as ARCHITECTURE)
- `:117` (under "## Init → ContextManaged → PromptBuilt sequence") → **h01-init-sequence.svg**, dense (63 lines) → `<details>` fallback
- `:270` (under "### Module structure (Sprint 2)") → **h01-module-structure.svg**
- `:303` (under "### Call graph") → **h01-call-graph.svg**, dense (33 lines) → `<details>` fallback

- [ ] **Step 1: Author the three new svgs** from their source blocks using the canonical style (Brain indigo, `.mono` for state/fn names). aria-labels: sequence = "Canonical Init→ContextManaged→PromptBuilt sequence showing H10, H11, H04, H05 and H09 ordering"; module-structure = "H01 turn-driver module structure: typed-state enum, run() match loop, per-state transition functions"; call-graph = "H01 call graph across one turn".
- [ ] **Step 2: Validate** — Validate-all → all `OK`.
- [ ] **Step 3: Render-and-eyeball** all three.
- [ ] **Step 4: Embed.** `:52` → `<img src="../diagrams/turn-fsm.svg" …>`. The other three → their svgs; add `<details>` fallback for `h01-init-sequence` and `h01-call-graph`.
- [ ] **Step 5: Verify** — `grep -c 'diagrams/.*\.svg' docs/components/H01-turn-driver.md` ≥ 4.
- [ ] **Step 6: Commit**

```bash
git add docs/diagrams/h01-*.svg docs/components/H01-turn-driver.md
git commit -m "docs(H01): SVG figures for state machine, init sequence, module structure, call graph"
```

---

## Task 7: H03 Resume Coordinator component doc (verify-or-skip)

**Files:**
- Create (genuine diagrams only): `docs/diagrams/h03-resume-decision.svg`, `docs/diagrams/h03-oracle-assertions.svg`
- Modify: `docs/components/H03-resume-coordinator.md` (blocks at `:19`, `:303`)

`:19` (86 lines, under "## Interface") is large — apply the **verify-or-skip guard**: if it is a Rust trait/interface listing, keep it as code; if it is a boxes-and-arrows algorithm sketch, convert to `h03-resume-decision.svg` with a `<details>` fallback.

- [ ] **Step 1: Inspect `:19` and `:303`.** Decide diagram vs. code for each.
- [ ] **Step 2: Author svgs** for whichever blocks are genuine diagrams. `:303` (under "### Four oracle assertions") → **h03-oracle-assertions.svg** if diagrammatic. aria-labels describe the resume decision flow / oracle relationships. Session green where it depicts the event log; Brain indigo for the replay logic.
- [ ] **Step 3: Validate** — Validate-all → all `OK`.
- [ ] **Step 4: Render-and-eyeball** created svgs.
- [ ] **Step 5: Embed** created svgs (`REL = ..`); `<details>` fallback for any block > ~25 lines. Leave code blocks untouched.
- [ ] **Step 6: Commit**

```bash
git add docs/diagrams/h03-*.svg docs/components/H03-resume-coordinator.md
git commit -m "docs(H03): SVG figures for resume coordinator (kept trait listings as code)"
```

---

## Task 8: Remaining component docs — H09, H11, subagent, TUI

**Files:**
- Create: `docs/diagrams/h09-lifecycle-timeline.svg`, `docs/diagrams/h11-state-placement.svg`, `docs/diagrams/subagent-position.svg`, `docs/diagrams/tui-position.svg`
- Modify: `docs/components/H09-hook-pipeline.md` (`:158`), `docs/components/H11-context-manage.md` (`:89`), `docs/components/cogito-subagent.md` (`:23`), `docs/components/cogito-tui.md` (`:10`)

Source → target (`REL = ..`), all compact (≤ 18 lines) → **no fallback needed**:
- H09 `:158` (under "### Lifecycle timeline") → **h09-lifecycle-timeline.svg** — the five hook points on a turn timeline. aria-label: "Hook lifecycle timeline: the five hook points across one turn."
- H11 `:89` (under "## State machine placement") → **h11-state-placement.svg** — where ContextManaged sits in the FSM. aria-label: "H11 Context Manage placement between Init and PromptBuilt in the turn state machine."
- subagent `:23` (under "## Position") → **subagent-position.svg** — subagent module between Brain and Hands. aria-label: "cogito-subagent position: a Hands-layer ToolProvider that spawns child Brain instances via BrainSpawner."
- tui `:10` (under "## Position") → **tui-position.svg** — TUI as a Surface over the Runtime. aria-label: "cogito-tui position: a Surface crate driving the Runtime, parallel to cogito-cli."

- [ ] **Step 1: Author the four svgs** using the canonical style and the layer colors appropriate to each (Brain/Hands/Surface).
- [ ] **Step 2: Validate** — Validate-all → all `OK`.
- [ ] **Step 3: Render-and-eyeball** all four.
- [ ] **Step 4: Embed** each (`REL = ..`), replacing the ASCII block. No fallback.
- [ ] **Step 5: Verify** — each of the four docs has ≥ 1 `diagrams/*.svg` reference:

```bash
for f in H09-hook-pipeline H11-context-manage cogito-subagent cogito-tui; do echo "$f: $(grep -c 'diagrams/.*\.svg' docs/components/$f.md)"; done
```
Expected: each ≥ 1.

- [ ] **Step 6: Commit**

```bash
git add docs/diagrams/h09-*.svg docs/diagrams/h11-*.svg docs/diagrams/subagent-position.svg docs/diagrams/tui-position.svg docs/components/H09-hook-pipeline.md docs/components/H11-context-manage.md docs/components/cogito-subagent.md docs/components/cogito-tui.md
git commit -m "docs(components): SVG figures for H09 lifecycle, H11 placement, subagent and TUI position"
```

---

## Task 9 (extension): configuration/overview.md

**Files:**
- Create: `docs/diagrams/config-sources-composition.svg`, `docs/diagrams/config-crate-map.svg`, `docs/diagrams/config-data-flow.svg` (and `config-secret-flow.svg` only if `:175` is a real diagram)
- Modify: `docs/configuration/overview.md` (blocks at `:118`, `:175`, `:408`, `:501`)

Source → target (`REL = ..`):
- `:118` (under "## 4. Sources and composition") → **config-sources-composition.svg** — the precedence/merge of config sources.
- `:175` (2 lines, under "## 5. Secret handling") → tiny; apply verify-or-skip — likely an inline arrow, convert to **config-secret-flow.svg** only if it stands as a figure, else leave.
- `:408` (under "## 9. Crate map") → **config-crate-map.svg** — which crate owns which config concern; color by layer.
- `:501` (57 lines, under "## 11. Data flow") → **config-data-flow.svg**, dense → `<details>` fallback.

- [ ] **Step 1: Inspect the four blocks**, apply verify-or-skip to `:175`.
- [ ] **Step 2: Author svgs** using the canonical style; the crate map should use the layer colors so it visually agrees with `crate-import-rules.svg`.
- [ ] **Step 3: Validate** — Validate-all → all `OK`.
- [ ] **Step 4: Render-and-eyeball** each.
- [ ] **Step 5: Embed** (`REL = ..`); `<details>` fallback for `config-data-flow`.
- [ ] **Step 6: Commit**

```bash
git add docs/diagrams/config-*.svg docs/configuration/overview.md
git commit -m "docs(config): SVG figures for sources/composition, crate map, and data flow"
```

---

## Task 10: Final sweep & spec close-out

**Files:**
- Modify: `docs/superpowers/specs/2026-06-03-svg-diagram-system-design.md` (status), `docs/diagrams/README.md` (index, optional)

- [ ] **Step 1: Validate every svg** — run Validate-all. Expected: every file `OK`, zero `FAIL`.
- [ ] **Step 2: Check every embed resolves.** For each doc, confirm each referenced svg exists:

```bash
grep -rhoE 'diagrams/[a-z0-9-]+\.svg' README.md ARCHITECTURE.md docs/components docs/configuration | sort -u | while read p; do test -f "docs/${p#*diagrams/../}" -o -f "docs/diagrams/${p##*/}" && echo "ok ${p##*/}" || echo "MISSING ${p##*/}"; done
```
Resolve any `MISSING`.

- [ ] **Step 3: Check no embed has empty alt** —

```bash
grep -rnE '<img[^>]*alt=""' README.md ARCHITECTURE.md docs/components docs/configuration && echo "EMPTY ALT FOUND" || echo "all alts non-empty"
```
Expected: `all alts non-empty`.

- [ ] **Step 4: Confirm no stray ASCII diagrams remain in in-scope docs** (outside `<details>` fallbacks). Spot-check: every former diagram block is now either an `<img>` or wrapped in `<details>` or deliberately kept as code (verify-or-skip). List any leftover for a follow-up.
- [ ] **Step 5: Contact sheet eyeball.** Render all svgs to `/tmp` and skim for any with overflow/overlap that slipped through. Fix in place, re-validate.
- [ ] **Step 6: Update spec status** in `docs/superpowers/specs/2026-06-03-svg-diagram-system-design.md`: change `状态: Draft（待评审）` to `状态: Implemented`.
- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "docs(diagrams): final validation sweep; mark SVG diagram system implemented"
```

---

## Self-review notes

- **Spec coverage:** design language → Task 1 + Conventions block; embedding + dark-mode card → every embed step; grep fallback → `<details>` in Tasks 3/4/6/7/9; core scope 21 diagrams → Tasks 2–8; configuration extension → Task 9; ADR/spec left untouched → not in any task's file list; verification → Task 10. All spec sections covered.
- **Verify-or-skip** applied to the blocks that may be code rather than diagrams (ARCH `:416`/`:467`, H03 `:19`, config `:175`) so the executor never force-converts a Rust listing.
- **Reuse** of `harness-layers.svg` (ARCH) and `turn-fsm.svg` (ARCH + H01) is explicit; no diagram is drawn twice.
