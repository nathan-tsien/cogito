default:
    @just --list

# Format all Rust code
fmt:
    cargo fmt --all

# Run clippy with autofix; pass a crate name to scope
fix crate="":
    #!/usr/bin/env bash
    if [ -z "{{crate}}" ]; then
        cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged
    else
        cargo clippy -p {{crate}} --all-targets --all-features --fix --allow-dirty --allow-staged
    fi
    cargo fmt --all

# Run tests via nextest
test crate="":
    #!/usr/bin/env bash
    if [ -z "{{crate}}" ]; then
        cargo nextest run --workspace --all-features
    else
        cargo nextest run -p {{crate}} --all-features
    fi

# Run benchmarks
bench:
    cargo bench --workspace

# Run JSONL append baseline benchmark. Output lands in target/criterion.
bench-baseline:
    cargo bench -p cogito-store-jsonl --bench append_throughput

# Run chaos tests (slow)
chaos:
    cargo test --test resume_chaos -p cogito-core --release -- --nocapture

# CI gate
ci: fmt-check clippy layer-check test

fmt-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Check ADR-0004 layer import rule
layer-check:
    @./scripts/check-layer.sh

# Run the CLI
chat:
    cargo run -p cogito-cli -- chat

# Inspect a session's event log
inspect session_id:
    cargo run -p cogito-cli -- inspect --session {{session_id}}

# Replay a session
replay session_id:
    cargo run -p cogito-cli -- replay --session {{session_id}}

clean:
    cargo clean

# Regenerate JSON Schema for ConversationEvent into docs/schemas/.
gen-schema:
    cargo run -p cogito-gen-schema --release -- \
        --output docs/schemas/conversation-event-v1.json

# Verify committed schema matches the current Rust types (CI gate).
gen-schema-check:
    cargo run -p cogito-gen-schema --release -- \
        --output docs/schemas/conversation-event-v1.json \
        --check
