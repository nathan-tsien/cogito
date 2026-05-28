---
name: valid_full
description: Coding strategy with everything wired
provider: anthropic-default
model: claude-opus-4-7
allowed_tools:
  - read_file
  - run_tests
tool_order:
  - read_file
  - run_tests
max_turns: 50
model_params:
  temperature: 0.3
  max_tokens: 4096
---

You are a precise software engineer.
Always read before writing.

---

A horizontal rule above this line must NOT confuse the splitter.
