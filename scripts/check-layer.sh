#!/usr/bin/env bash
# Enforce ADR-0004 import rule: cogito-core/src/harness/** may not
# import any concrete Hand, Boundary, or Session crate. The Cargo.toml
# already forbids the dependency; this grep also catches stray refs in
# example code and comments-turned-real-imports.
#
# Test code is exempt: ADR-0004 governs production code. Tests under
# `#[cfg(test)] mod tests { ... }` may use any dev-dependency. We strip
# those blocks before grepping (heuristic: skip from the first line
# matching `^\s*mod tests\s*{` to end of file — matches the codebase
# convention that test modules live at the bottom of each .rs file).

set -euo pipefail

FORBIDDEN_PATTERN='use cogito_(tools|model|sandbox|jobs|store_jsonl|store_postgres|store_http|mcp|subagent|storage_local|storage_s3|storage_http)'

TMP_PROD="$(mktemp -t layer-check-prod.XXXXXX.rs)"
trap 'rm -f "$TMP_PROD"' EXIT

violation=0
while IFS= read -r -d '' f; do
    awk '/^[[:space:]]*mod tests[[:space:]]*\{/{exit} {print}' "$f" > "$TMP_PROD"
    if grep -En "$FORBIDDEN_PATTERN" "$TMP_PROD" >/tmp/layer-check-matches 2>/dev/null; then
        echo "ERROR in $f (production code):" >&2
        cat /tmp/layer-check-matches >&2
        violation=1
    fi
done < <(find crates/cogito-core/src/harness/ -name '*.rs' -print0)
rm -f /tmp/layer-check-matches

if [ "$violation" -ne 0 ]; then
    echo "ERROR: ADR-0004 violation — Brain (harness/) imported a concrete Hand/Boundary/Session crate" >&2
    exit 1
fi

echo "OK: ADR-0004 layer import rule respected in cogito-core/src/harness/ (test code excluded)"
