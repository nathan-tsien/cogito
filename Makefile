# cogito — Makefile for local development and quick debugging
#
# Usage:
#   make chat              interactive REPL (provider/model from cogito.toml)
#   make test              run all workspace tests
#   make test CRATE=cogito-core   run one crate
#   make ci                full CI gate (fmt-check + clippy + layer-check + test)
#   make clean             wipe build artifacts
#
# Provider / model selection lives in `cogito.toml`. The file is loaded
# from (priority order):
#   1. $COGITO_CONFIG
#   2. ./cogito.toml
#   3. $XDG_CONFIG_HOME/cogito/config.toml   (defaulted below)
#
# CLI args (`--model`, `--provider`, `--base-url`) still work if you
# need a one-off override; pass them through as `make chat -- --model foo`
# is NOT supported, run `cargo run -p cogito-cli -- chat --model foo`
# directly for that.
#
# Prerequisites: Rust 1.85+, cargo.  Optional: cargo-nextest.

# Load .env if present (e.g. ANTHROPIC_API_KEY, COGITO_MCP_*_TOKEN).
-include .env
export

# cogito-config's file loader only consults XDG_CONFIG_HOME when it is
# set and non-empty (no implicit ~/.config/ fallback). Default it here
# so `~/.config/cogito/config.toml` is picked up automatically.
export XDG_CONFIG_HOME ?= $(HOME)/.config

# Internal helpers
CARGO        := cargo
CLI_RUN      := $(CARGO) run -p cogito-cli --

# Detect nextest; fall back to cargo test
NEXTEST      := $(shell $(CARGO) nextest --version 2>/dev/null && echo yes || echo no)
ifeq ($(NEXTEST),yes)
TEST_CMD     = $(CARGO) nextest run
else
TEST_CMD     = $(CARGO) test
endif

CRATE        ?=

.PHONY: default help \
        chat \
        test test-integration ci fmt fmt-check fix clippy layer-check \
        bench bench-baseline chaos \
        clean sessions-clean \
        gen-schema gen-schema-check \
        env-check

default: help

help:
	@echo ""
	@echo "cogito development targets"
	@echo ""
	@echo "  Chat / REPL"
	@echo "    make chat               interactive REPL (config from cogito.toml)"
	@echo ""
	@echo "  Testing"
	@echo "    make test               all workspace tests"
	@echo "    make test CRATE=<name>  single crate"
	@echo "    make test-integration   curated integration suite (no API key required)"
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
	@echo "    make sessions-clean SESSION_ROOT=...  remove *.jsonl under SESSION_ROOT"
	@echo "    make env-check          print active env values (no secrets)"
	@echo ""

# Environment sanity check (no secrets printed; just yes/no flags).
env-check:
	@echo "XDG_CONFIG_HOME = $(XDG_CONFIG_HOME)"
	@echo "RUST_LOG        = $(RUST_LOG)"
	@echo "ANTHROPIC_API_KEY set: $(if $(ANTHROPIC_API_KEY),yes,no)"
	@echo "OPENAI_API_KEY set:    $(if $(OPENAI_API_KEY),yes,no)"

# Chat / REPL — provider/model from cogito.toml. Override at the CLI
# layer by invoking `cargo run -p cogito-cli -- chat --model X` directly.
chat:
	$(CLI_RUN) chat

# Tests
test:
	@if [ -z "$(CRATE)" ]; then \
		$(TEST_CMD) --workspace --all-features; \
	else \
		$(TEST_CMD) -p $(CRATE) --all-features; \
	fi

# Curated integration tests that don't need a real API key.
test-integration:
	$(CARGO) test -p cogito-core  --test session_e2e
	$(CARGO) test -p cogito-core  --test turn_driver_text_only
	$(CARGO) test -p cogito-core  --test turn_driver_tool_call
	$(CARGO) test -p cogito-model --test anthropic_replay
	$(CARGO) test -p cogito-model --test openai_compat_replay

chaos:
	$(CARGO) test --test resume_chaos -p cogito-core --release -- --nocapture

# Code quality
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

# Benchmarks
bench:
	$(CARGO) bench --workspace

bench-baseline:
	$(CARGO) bench -p cogito-store-jsonl --bench append_throughput

# Schema
gen-schema:
	$(CARGO) run -p cogito-gen-schema --release -- \
		--output docs/schemas/conversation-event-v1.json

gen-schema-check:
	$(CARGO) run -p cogito-gen-schema --release -- \
		--output docs/schemas/conversation-event-v1.json \
		--check

# Cleanup
clean:
	$(CARGO) clean

# Remove session JSONL files under SESSION_ROOT (caller supplies the path).
SESSION_ROOT ?=
sessions-clean:
	@[ -n "$(SESSION_ROOT)" ] || (echo "usage: make sessions-clean SESSION_ROOT=<dir>" && exit 1)
	@echo "Removing session files from $(SESSION_ROOT)/ ..."
	@rm -f $(SESSION_ROOT)/*.jsonl && echo "Done." || echo "Nothing to remove."
