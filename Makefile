# cogito — Makefile for local development and quick debugging
#
# Usage:
#   make chat              interactive REPL (openai-compat, uses MODEL from .env)
#   make chat-anthropic    interactive REPL (Anthropic)
#   make test              run all workspace tests
#   make test CRATE=cogito-core   run one crate
#   make ci                full CI gate (fmt + clippy + layer-check + test)
#   make clean             wipe build artifacts
#
# Variables can be overridden on the command line:
#   make chat MODEL=claude-opus-4-7 SESSION_ROOT=./my-sessions
#
# Prerequisites: Rust 1.85+, cargo.  Optional: just, cargo-nextest.

# ── Load .env if present ─────────────────────────────────────────────────────
-include .env
export

# ── Defaults (overridden by .env or command line) ────────────────────────────
MODEL        ?= sensenova-6.7-flash-lite
SESSION_ROOT ?= ./sessions
RUST_LOG     ?= info
CRATE        ?=

# ── Internal helpers ─────────────────────────────────────────────────────────
CARGO        := cargo
CLI_RUN      := $(CARGO) run -p cogito-cli --

# Detect nextest; fall back to cargo test
NEXTEST      := $(shell $(CARGO) nextest --version 2>/dev/null && echo yes || echo no)
ifeq ($(NEXTEST),yes)
TEST_CMD     = $(CARGO) nextest run
else
TEST_CMD     = $(CARGO) test
endif

.PHONY: default help \
        chat chat-anthropic chat-openai \
        test ci fmt fix clippy layer-check \
        bench bench-baseline chaos \
        clean sessions-clean \
        gen-schema gen-schema-check \
        env-check

default: help

# ── Help ─────────────────────────────────────────────────────────────────────
help:
	@echo ""
	@echo "cogito development targets"
	@echo ""
	@echo "  Chat / REPL"
	@echo "    make chat               openai-compat REPL (MODEL from .env)"
	@echo "    make chat-anthropic     Anthropic REPL (ANTHROPIC_API_KEY from .env)"
	@echo "    make chat-openai        explicit openai-compat REPL"
	@echo ""
	@echo "  Testing"
	@echo "    make test               all workspace tests"
	@echo "    make test CRATE=<name>  single crate"
	@echo "    make ci                 full CI gate"
	@echo "    make chaos              resume chaos tests (slow)"
	@echo ""
	@echo "  Code quality"
	@echo "    make fmt                rustfmt"
	@echo "    make fix                clippy --fix + fmt"
	@echo "    make fix CRATE=<name>   clippy --fix scoped to one crate"
	@echo "    make clippy             clippy -D warnings (read-only)"
	@echo "    make layer-check        ADR-0004 import-rule check"
	@echo ""
	@echo "  Misc"
	@echo "    make clean              cargo clean"
	@echo "    make sessions-clean     remove ./sessions/*.jsonl"
	@echo "    make env-check          print active env values (no secrets)"
	@echo ""

# ── Environment sanity check ─────────────────────────────────────────────────
env-check:
	@echo "MODEL        = $(MODEL)"
	@echo "SESSION_ROOT = $(SESSION_ROOT)"
	@echo "RUST_LOG     = $(RUST_LOG)"
	@echo "OPENAI_BASE_URL = $(OPENAI_BASE_URL)"
	@echo "ANTHROPIC_API_KEY set: $(if $(ANTHROPIC_API_KEY),yes,no)"
	@echo "OPENAI_API_KEY set:    $(if $(OPENAI_API_KEY),yes,no)"

# ── Chat / REPL ───────────────────────────────────────────────────────────────
chat: env-check
	$(CLI_RUN) chat \
		--model "$(MODEL)" \
		--provider openai-compat \
		--base-url "$(OPENAI_BASE_URL)" \
		--session-root "$(SESSION_ROOT)"

chat-anthropic: env-check
	$(CLI_RUN) chat \
		--model "$(MODEL)" \
		--provider anthropic \
		--session-root "$(SESSION_ROOT)"

chat-openai: env-check
	$(CLI_RUN) chat \
		--model "$(MODEL)" \
		--provider openai-compat \
		--base-url "$(OPENAI_BASE_URL)" \
		--session-root "$(SESSION_ROOT)"

# ── Tests ─────────────────────────────────────────────────────────────────────
test:
	@if [ -z "$(CRATE)" ]; then \
		$(TEST_CMD) --workspace --all-features; \
	else \
		$(TEST_CMD) -p $(CRATE) --all-features; \
	fi

# Key integration tests that don't need a real API key
test-integration:
	$(CARGO) test -p cogito-core  --test session_e2e
	$(CARGO) test -p cogito-core  --test turn_driver_text_only
	$(CARGO) test -p cogito-core  --test turn_driver_tool_call
	$(CARGO) test -p cogito-model --test anthropic_replay
	$(CARGO) test -p cogito-model --test openai_compat_replay

chaos:
	$(CARGO) test --test resume_chaos -p cogito-core --release -- --nocapture

# ── Code quality ─────────────────────────────────────────────────────────────
fmt:
	$(CARGO) fmt --all

fix:
	@if [ -z "$(CRATE)" ]; then \
		$(CARGO) clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged; \
	else \
		$(CARGO) clippy -p $(CRATE) --all-targets --all-features --fix --allow-dirty --allow-staged; \
	fi
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

layer-check:
	@./scripts/check-layer.sh

ci: fmt-check clippy layer-check test

fmt-check:
	$(CARGO) fmt --all -- --check

# ── Benchmarks ───────────────────────────────────────────────────────────────
bench:
	$(CARGO) bench --workspace

bench-baseline:
	$(CARGO) bench -p cogito-store-jsonl --bench append_throughput

# ── Schema ────────────────────────────────────────────────────────────────────
gen-schema:
	$(CARGO) run -p cogito-gen-schema --release -- \
		--output docs/schemas/conversation-event-v1.json

gen-schema-check:
	$(CARGO) run -p cogito-gen-schema --release -- \
		--output docs/schemas/conversation-event-v1.json \
		--check

# ── Cleanup ───────────────────────────────────────────────────────────────────
clean:
	$(CARGO) clean

sessions-clean:
	@echo "Removing session files from $(SESSION_ROOT)/ ..."
	@rm -f $(SESSION_ROOT)/*.jsonl && echo "Done." || echo "Nothing to remove."
