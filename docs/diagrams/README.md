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
