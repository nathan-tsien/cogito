# Experiments

Each sprint produces an experiment that validates (or invalidates) a
specific design hypothesis. Reports live here.

## Index

- E01 — Step Recorder performance (Sprint 1)
- E02 — Tool call error rate (Sprint 2)
- E03 — Resume correctness under chaos (Sprint 3)
- E04 — Async job pause/resume (Sprint 4)
- E05 — Multi-model strategy decoupling (Sprint 5)
- E06 — Hook pipeline latency impact (Sprint 6)
- E07 — Stream interruption and reconnect (TBD)
- E08 — Prefix cache hit rate (TBD)
- [Skill Support — Phase 2 end-to-end](./2026-06-02-skill-support-phase2.md) — script-bearing skill (`pptx`) runs end to end; zero Brain change (2026-06-02)

## Report template

```markdown
# E0X · Experiment Name

## Hypothesis
The design assumption being tested.

## Method
What was measured and how.

## Data
Raw numbers, ideally with plots in `data/E0X/`.

## Result
Hypothesis confirmed / refuted / partial.

## Implications for v1.1
Concrete design changes to propose.
```
