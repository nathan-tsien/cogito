#!/usr/bin/env bash
# Enforce ADR-0004 import rule: cogito-core/src/harness/** may not
# import any concrete Hand, Boundary, or Session crate. The Cargo.toml
# already forbids the dependency; this grep also catches stray refs in
# example code and comments-turned-real-imports.

set -euo pipefail

FORBIDDEN_PATTERN='use cogito_(tools|model|sandbox|jobs|store_jsonl|store_postgres|store_http|mcp|subagent|storage_local|storage_s3|storage_http)'

if grep -rEn "$FORBIDDEN_PATTERN" crates/cogito-core/src/harness/ 2>/dev/null; then
    echo "ERROR: ADR-0004 violation — Brain (harness/) imported a concrete Hand/Boundary/Session crate" >&2
    exit 1
fi

echo "OK: ADR-0004 layer import rule respected in cogito-core/src/harness/"
