// Simple deck built following the pptx skill's pptxgenjs.md "create from scratch"
// guide. Title slide (dark) + content slide with bullets (light) — the skill's
// dark/light "sandwich" idea, one accent color.
const pptxgen = require("pptxgenjs");

const INK = "0B1F3A";    // dominant dark navy
const PAPER = "F5F7FA";  // light content background
const ACCENT = "E8552D"; // single sharp accent

const pres = new pptxgen();
pres.layout = "LAYOUT_WIDE";
pres.author = "cogito";
pres.title = "cogito — Skill Support";

// Slide 1 — title (dark)
const s1 = pres.addSlide();
s1.background = { color: INK };
s1.addText("cogito Skill Support", {
  x: 0.7, y: 2.2, w: 11.9, h: 1.2, fontSize: 44, bold: true, color: "FFFFFF",
});
s1.addText("Phase 2 — a script-bearing skill, end to end", {
  x: 0.7, y: 3.4, w: 11.9, h: 0.7, fontSize: 22, color: PAPER,
});
s1.addShape(pres.ShapeType.rect, { x: 0.7, y: 3.25, w: 2.4, h: 0.08, fill: { color: ACCENT } });

// Slide 2 — content (light) with bullets
const s2 = pres.addSlide();
s2.background = { color: PAPER };
s2.addText("How it works", {
  x: 0.7, y: 0.5, w: 11.9, h: 0.8, fontSize: 30, bold: true, color: INK,
});
s2.addText(
  [
    { text: "SKILL.md body injected into the system prompt", options: { bullet: true, breakLine: true } },
    { text: "Bundled files reached in place via read_file (ADR-0032)", options: { bullet: true, breakLine: true } },
    { text: "Scripts run via bash in the workspace (ADR-0023 / 0031)", options: { bullet: true, breakLine: true } },
    { text: "Missing deps self-healed by the agent loop (ADR-0033)", options: { bullet: true } },
  ],
  { x: 0.9, y: 1.5, w: 11.5, h: 4.0, fontSize: 20, color: "1B2A3A", lineSpacingMultiple: 1.3 },
);
s2.addShape(pres.ShapeType.rect, { x: 0.0, y: 0.0, w: 0.18, h: 7.5, fill: { color: ACCENT } });

pres.writeFile({ fileName: "cogito-skill-support.pptx" }).then((f) => {
  console.log("wrote", f);
});
