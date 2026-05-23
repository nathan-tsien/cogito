# Skill Loader

Sprint 7 introduces `cogito-skills`, a Hands-layer userland extension surface.
Team members ship knowledge packs as markdown + YAML; the model activates via
`$Name` sigils and users via `/skill <name>`.

See:
- [`ADR-0020`](../adr/0020-skill-loader.md) — locked decisions
- [Sprint 7 design spec](../superpowers/specs/2026-05-23-sprint-7-skill-loader-design.md) — implementation detail
- [`H06 stream-demux`](../components/H06-stream-demux.md) — sigil detection
- [`H11 context-manage`](../components/H11-context-manage.md) — SkillInjector

## Authoring a skill

1. Pick a kebab-case name (`my-helper`).
2. Create `<scope>/.cogito/skills/my-helper/SKILL.md` where `<scope>` is your
   workspace root (Repo) or `~/` (User).
3. Write the SKILL.md:

   ```markdown
   ---
   name: my-helper
   description: Short description shown in the model's "Available Skills" registry.
   version: 0.1.0
   ---

   # My Helper

   Detailed instructions for the model when this skill is activated.
   ```

4. Restart `cogito chat`. The model can now emit `$my-helper` to activate.

## Activation channels

- Model: write `$my-helper` in a reply. H06 detects, H11 of next turn injects.
- User: type `/skill my-helper` in `cogito chat`. CLI emits a
  `TurnTrigger::SkillActivation` trigger; the same H11 path injects.

Both produce a `SkillActivated` event in the conversation log; one event per
session per name (cross-turn dedup).

## Scope precedence

Repo > User > Plugin > System. Higher scope wins on bare-name conflict.
Plugin scope is gated on Sprint 12 / ADR-0021.
