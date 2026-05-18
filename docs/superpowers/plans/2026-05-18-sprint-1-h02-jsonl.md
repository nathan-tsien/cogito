# Sprint 1 · H02 + JSONL ConversationStore + Cross-Language Event Log

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Sprint 1 of v0.1 per spec `2026-05-18-h02-conversation-store-and-event-log.md` — land `ConversationEvent` + `ConversationStore` + dev-grade JSONL backend + `StepRecorder` + cross-language schema commitment (ADR-0007).

**Architecture:** Pure additive Rust types + dev-grade backend + tooling. No FSM yet (Sprint 2). No real H06/H08 wiring (Sprint 2). Recorder is unit-tested against a mock store. JSONL is the only backend; every choice is "simplest thing that works" because JSONL is dev/debug-only (Postgres v0.4 carries production load).

**Tech Stack:** Rust 2024 (MSRV 1.85), tokio 1.40, tokio-util, serde 1, thiserror 1, ulid, dashmap, parking_lot, async-trait, futures, async-stream, schemars 0.8, criterion (benches), nextest, just, chrono.

**Conventions (read before starting any task):**

- All Rust comments (`//`, `///`, `//!`) in English (per CLAUDE.md §Coding standards). Chinese stays in spec/ADR/commit/chat.
- Errors via `thiserror` (libraries) or `anyhow` (binaries / tools). No `unwrap` / `expect` / `panic` in non-test code.
- `unsafe_code = "forbid"`. `missing_docs = "warn"` — every public item has a doc comment.
- Workspace deps go through `[workspace.dependencies]`; members declare `{ workspace = true }`.
- Commits: imperative, capitalized first word, no trailing period. Match recent style.
- Each task ends with one commit. Branch: `impl/sprint-1-h02-jsonl` off `main` (created in Task 1).
- After plan completes: open PR `nathan-tsien:impl/sprint-1-h02-jsonl -> main`, base = main.
- **Spec is the source of truth.** When this plan and the spec disagree, fix the plan. When the spec is wrong, stop and escalate.

**Test snippet adaptation (workspace lints are strict):**

`unwrap_used = "deny"` + `expect_used = "deny"` + `panic = "deny"` are enforced at workspace root. Test snippets below sometimes use `.unwrap()` for readability; adapt to one of:

```rust
#[test]
fn name() -> Result<(), Box<dyn std::error::Error>> {
    // … propagate with ?
    Ok(())
}
```

or for serde-only tests:

```rust
#[test]
fn name() -> serde_json::Result<()> {
    let json = serde_json::to_string(&value)?;
    let back: T = serde_json::from_str(&json)?;
    assert_eq!(value, back);
    Ok(())
}
```

`#[tokio::test]` async tests follow the same pattern with `Result<(), …>` return.

---

## Task 0: Branch + plan trail

**Files:**
- Modify: (git only)

- [ ] **Step 1: Create implementation branch off main**

```bash
git fetch github
git checkout main
git pull --ff-only
git checkout -b impl/sprint-1-h02-jsonl
```

Expected: clean checkout on new branch, tracking nothing yet.

- [ ] **Step 2: Confirm no in-flight changes**

Run: `git status --short`
Expected: empty output.

- [ ] **Step 3: (No commit) Begin Task 1**

---

## Task 1: Workspace dep additions + `cogito-gen-schema` scaffold

**Files:**
- Modify: `Cargo.toml`
- Create: `tools/cogito-gen-schema/Cargo.toml`
- Create: `tools/cogito-gen-schema/src/main.rs`

- [ ] **Step 1: Add new workspace deps to `Cargo.toml`**

Open `Cargo.toml`. In `[workspace.dependencies]`, add (alphabetical-ish):

```toml
async-stream = "0.3"
schemars = { version = "0.8", features = ["chrono", "uuid1"] }
ulid = { version = "1.1", features = ["serde"] }
```

Verify `criterion`, `dashmap`, `parking_lot`, `chrono`, `tokio-util` are already present from Sprint 0 (they should be).

In `[workspace]`, add `tools/cogito-gen-schema` to the `members` array (keep alphabetical order — after the `crates/...` block, in a new `tools/` block):

```toml
[workspace]
members = [
    "crates/cogito-core",
    "crates/cogito-protocol",
    # … existing members …
    "tools/cogito-gen-schema",
]
```

- [ ] **Step 2: Scaffold tools/cogito-gen-schema crate**

Create `tools/cogito-gen-schema/Cargo.toml`:

```toml
[package]
name = "cogito-gen-schema"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true
description = "Internal tool: generate JSON Schema for cogito-protocol public types"

[[bin]]
name = "cogito-gen-schema"
path = "src/main.rs"

[dependencies]
cogito-protocol.workspace = true
schemars.workspace = true
serde_json.workspace = true
anyhow.workspace = true
clap = { version = "4", features = ["derive"] }

[lints]
workspace = true
```

(Confirm `clap` is in workspace deps from Sprint 0; if not, add `clap = { version = "4", features = ["derive"] }` to `[workspace.dependencies]` first.)

- [ ] **Step 3: Stub main.rs with todo!()**

Create `tools/cogito-gen-schema/src/main.rs`:

```rust
//! Generate JSON Schema for cogito-protocol public types.
//!
//! Invoked via `just gen-schema`. CI runs `just gen-schema-check` (this
//! tool with `--check`) to enforce drift-free committed schema files.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cogito-gen-schema", about)]
struct Args {
    /// Output path for the generated JSON Schema.
    #[arg(long)]
    output: PathBuf,

    /// If set, compare generated schema against the file at `--output`
    /// and exit non-zero if they differ. Does not write.
    #[arg(long, default_value_t = false)]
    check: bool,
}

fn main() -> Result<()> {
    let _args = Args::parse();
    // Real implementation lands in Task 11 after types are defined.
    anyhow::bail!("not yet implemented — see Plan 2 Task 11")
}
```

- [ ] **Step 4: Verify workspace builds**

Run: `cargo check --workspace`
Expected: all crates build cleanly, including `cogito-gen-schema`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml tools/cogito-gen-schema
git commit -m "Add schema-gen tool scaffold + workspace deps for Sprint 1"
```

---

## Task 2: cogito-protocol IDs (`EventId`, `SessionId`, `TurnId`)

**Files:**
- Create: `crates/cogito-protocol/src/ids.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Add ulid to cogito-protocol Cargo.toml**

Open `crates/cogito-protocol/Cargo.toml`. In `[dependencies]`, add:

```toml
ulid.workspace = true
schemars.workspace = true
chrono.workspace = true
```

(Confirm `chrono = { workspace = true, features = ["serde"] }` — if not in workspace deps, add `features = ["serde"]` at the workspace root.)

- [ ] **Step 2: Write the failing test FIRST**

Create `crates/cogito-protocol/src/ids.rs` with **only** the test module (no impl yet):

```rust
//! Strongly-typed identifiers used throughout the protocol layer.
//!
//! All IDs wrap a [`ulid::Ulid`] which is monotonic per process, lexically
//! sortable, and renders as a 26-character Crockford base32 string in JSON.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_id_roundtrips_through_json() -> serde_json::Result<()> {
        let id = EventId::new();
        let json = serde_json::to_string(&id)?;
        // ULID renders as a quoted 26-char string.
        assert_eq!(json.len(), 28); // 26 chars + 2 quotes
        let back: EventId = serde_json::from_str(&json)?;
        assert_eq!(id, back);
        Ok(())
    }

    #[test]
    fn session_id_is_distinct_from_turn_id() -> serde_json::Result<()> {
        let s = SessionId::new();
        let json = serde_json::to_string(&s)?;
        // SessionId and TurnId should not be confusable: deserializing a
        // SessionId JSON into a TurnId works at the JSON level (both are
        // strings) but they remain different Rust types.
        let _t: TurnId = serde_json::from_str(&json)?;
        Ok(())
    }

    #[test]
    fn ids_are_display_and_parse() -> Result<(), Box<dyn std::error::Error>> {
        let id = EventId::new();
        let rendered = id.to_string();
        let parsed: EventId = rendered.parse()?;
        assert_eq!(id, parsed);
        Ok(())
    }

    #[test]
    fn ids_implement_ord() {
        let a = EventId::new();
        let b = EventId::new();
        // ulid::Ulid is monotonic per process, so b should sort >= a.
        assert!(b >= a);
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p cogito-protocol --lib ids::tests`
Expected: FAIL with "cannot find type `EventId` in this scope" (and similar for SessionId/TurnId).

- [ ] **Step 4: Implement the minimal types to make the tests pass**

Replace the contents of `crates/cogito-protocol/src/ids.rs` with the test module ABOVE plus the impl ABOVE the `#[cfg(test)]`:

```rust
//! Strongly-typed identifiers used throughout the protocol layer.
//!
//! All IDs wrap a [`ulid::Ulid`] which is monotonic per process, lexically
//! sortable, and renders as a 26-character Crockford base32 string in JSON.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

macro_rules! id_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
            Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            /// Create a fresh ID using a process-local monotonic ULID.
            #[must_use]
            pub fn new() -> Self {
                Self(Ulid::new())
            }

            /// Borrow the inner ULID.
            #[must_use]
            pub fn as_ulid(self) -> Ulid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_str(s).map(Self)
            }
        }

        impl From<Ulid> for $name {
            fn from(u: Ulid) -> Self {
                Self(u)
            }
        }
    };
}

id_newtype!(EventId, "Globally unique event identifier.");
id_newtype!(SessionId, "Conversation session identifier.");
id_newtype!(TurnId, "Per-session turn identifier.");

#[cfg(test)]
mod tests {
    use super::*;

    // … (same tests as Step 2)
}
```

- [ ] **Step 5: Wire module into lib.rs**

Open `crates/cogito-protocol/src/lib.rs`. Add `pub mod ids;` alphabetically among existing `pub mod` declarations, and add the re-exports at the crate root:

```rust
pub mod ids;
pub use ids::{EventId, SessionId, TurnId};
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p cogito-protocol --lib ids::tests`
Expected: 4/4 PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/cogito-protocol/Cargo.toml crates/cogito-protocol/src/ids.rs crates/cogito-protocol/src/lib.rs Cargo.toml
git commit -m "Add EventId/SessionId/TurnId ULID newtypes to cogito-protocol"
```

---

## Task 3: cogito-protocol `ContentBlock`

**Files:**
- Create: `crates/cogito-protocol/src/content.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/cogito-protocol/src/content.rs`:

```rust
//! `ContentBlock` — the wire-format unit shared between models, tools,
//! and persisted events. v0.1 covers Text / ToolUse / ToolResult. Image
//! and other multimodal variants land in v0.2 (ADR-0007 storage spec).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::tool::ToolResult;

/// One unit of content as defined by the Anthropic / OpenAI wire formats.
///
/// Adjacently-tagged (`tag = "type", content = "data"`) for forward
/// compatibility: newtype-with-sequence bodies are allowed (unlike
/// internal tagging), and new variants can be added without bumping
/// `SCHEMA_VERSION` (see ADR-0007).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentBlock {
    /// Plain assistant or user text.
    Text {
        /// The text content.
        text: String,
    },
    /// Model-issued tool call.
    ToolUse {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Tool arguments as JSON.
        args: serde_json::Value,
    },
    /// Result fed back to the model for a previously-issued tool call.
    ToolResult {
        /// Identifier matching the originating `ToolUse.call_id`.
        call_id: String,
        /// Structured result.
        result: ToolResult,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrips() -> serde_json::Result<()> {
        let cb = ContentBlock::Text { text: "hello".into() };
        let json = serde_json::to_string(&cb)?;
        assert_eq!(json, r#"{"type":"text","data":{"text":"hello"}}"#);
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }

    #[test]
    fn tool_use_roundtrips() -> serde_json::Result<()> {
        let cb = ContentBlock::ToolUse {
            call_id: "toolu_01".into(),
            tool_name: "read_file".into(),
            args: serde_json::json!({"path": "/tmp/x"}),
        };
        let json = serde_json::to_string(&cb)?;
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }

    #[test]
    fn tool_result_carrying_sequence_body_roundtrips() -> serde_json::Result<()> {
        let cb = ContentBlock::ToolResult {
            call_id: "toolu_01".into(),
            result: ToolResult::Output(vec![ContentBlock::Text {
                text: "file contents".into(),
            }]),
        };
        let json = serde_json::to_string(&cb)?;
        let back: ContentBlock = serde_json::from_str(&json)?;
        assert_eq!(cb, back);
        Ok(())
    }
}
```

- [ ] **Step 2: Wire module into lib.rs**

In `crates/cogito-protocol/src/lib.rs`, alphabetically:

```rust
pub mod content;
pub use content::ContentBlock;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cogito-protocol --lib content::tests`
Expected: 3/3 PASS. The third test (tool_result with `Vec<ContentBlock>`) is the regression for the Sprint 0 Task 7 serde-tag bug — it must pass.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/content.rs crates/cogito-protocol/src/lib.rs
git commit -m "Add ContentBlock (Text/ToolUse/ToolResult) to cogito-protocol"
```

---

## Task 4: cogito-protocol `SessionMeta`

**Files:**
- Create: `crates/cogito-protocol/src/session.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Write failing tests + implement together (small surface)**

Create `crates/cogito-protocol/src/session.rs`:

```rust
//! Session-level metadata recorded once per session as the first
//! `ConversationEvent::SessionStarted` payload.
//!
//! Most fields are optional pass-through metadata supplied by the
//! consumer. Cogito performs no validation or auth on these — they
//! are preserved verbatim for the SaaS catalog use case (ADR-0007).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Session-level metadata. All fields except `cogito_version` are
/// optional / consumer-supplied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct SessionMeta {
    /// Cogito library version that created this session.
    pub cogito_version: String,

    /// Strategy name (from `HarnessStrategy::name`) selected for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,

    /// Model identifier intended for this session (e.g. `"claude-sonnet-4-6"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Optional consumer-supplied user identifier. Cogito does no auth
    /// on this field — opaque pass-through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    /// Optional consumer-supplied tenant identifier. Cogito propagates
    /// only; enforcement is the consumer's responsibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,

    /// Opaque consumer-supplied metadata; preserved verbatim.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_meta_roundtrips_with_none_fields_omitted() -> serde_json::Result<()> {
        let meta = SessionMeta {
            cogito_version: "0.1.0".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta)?;
        // Only cogito_version should be serialized; Option fields and empty
        // extra map are skipped.
        assert_eq!(json, r#"{"cogito_version":"0.1.0"}"#);
        let back: SessionMeta = serde_json::from_str(&json)?;
        assert_eq!(meta, back);
        Ok(())
    }

    #[test]
    fn full_meta_roundtrips() -> serde_json::Result<()> {
        let mut extra = serde_json::Map::new();
        extra.insert("source".into(), serde_json::json!("web"));
        let meta = SessionMeta {
            cogito_version: "0.1.0".into(),
            strategy: Some("default".into()),
            model: Some("claude-sonnet-4-6".into()),
            user_id: Some("u_42".into()),
            tenant_id: Some("acme".into()),
            extra,
        };
        let json = serde_json::to_string(&meta)?;
        let back: SessionMeta = serde_json::from_str(&json)?;
        assert_eq!(meta, back);
        Ok(())
    }

    #[test]
    fn unknown_fields_in_json_do_not_panic() -> serde_json::Result<()> {
        // Forward-compat: a v0.2 writer may add a field we don't know.
        // serde defaults to ignoring unknowns (no `deny_unknown_fields`).
        let json = r#"{"cogito_version":"0.2.0","brand_new_field":42}"#;
        let _meta: SessionMeta = serde_json::from_str(json)?;
        Ok(())
    }
}
```

- [ ] **Step 2: Wire into lib.rs**

```rust
pub mod session;
pub use session::SessionMeta;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cogito-protocol --lib session::tests`
Expected: 3/3 PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/session.rs crates/cogito-protocol/src/lib.rs
git commit -m "Add SessionMeta to cogito-protocol"
```

---

## Task 5: cogito-protocol `ConversationEvent` + `EventPayload` + `SCHEMA_VERSION`

**Files:**
- Create: `crates/cogito-protocol/src/event.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/cogito-protocol/src/event.rs`:

```rust
//! Persisted event log shape. See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §2 for the design rationale.
//!
//! ConversationEvent is the persistent counterpart to StreamEvent (the
//! live broadcast). They are intentionally different types — see ADR-0006 §7.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::content::ContentBlock;
use crate::ids::{EventId, SessionId, TurnId};
use crate::job::{JobCompletionEvent, JobId};
use crate::session::SessionMeta;
use crate::tool::ToolResult;
use crate::turn::{TurnFailureReason, TurnOutcome};

/// Schema version emitted by this build of cogito. Bumped together with
/// every breaking change to `ConversationEvent` or `EventPayload`. See
/// ADR-0005 §4 #2 and ADR-0007 for compatibility rules.
pub const SCHEMA_VERSION: u32 = 1;

/// One persisted entry in a conversation's event log.
///
/// Envelope fields are at the JSON top level. The variant-specific payload
/// is adjacently tagged with `tag = "type"` / `content = "data"`, flattened
/// into the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationEvent {
    /// Schema version of the envelope and payload.
    pub schema_version: u32,

    /// Globally unique, monotonic-per-process event identifier.
    pub event_id: EventId,

    /// Session this event belongs to.
    pub session_id: SessionId,

    /// Turn this event belongs to. `None` for session-level events.
    pub turn_id: Option<TurnId>,

    /// Monotonic per-session sequence number. First event has `seq = 0`.
    pub seq: u64,

    /// Wall-clock timestamp at recorder serialization time.
    pub ts: DateTime<Utc>,

    /// Variant-specific payload.
    #[serde(flatten)]
    pub payload: EventPayload,
}

/// The variant-specific payload of a `ConversationEvent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventPayload {
    /// First event of every session.
    SessionStarted {
        /// Session-level metadata.
        meta: SessionMeta,
    },

    /// A new turn has begun.
    TurnStarted {
        /// User input that triggered this turn.
        user_input: Vec<ContentBlock>,
    },

    /// One content block of assistant text has been fully emitted.
    AssistantMessageAppended {
        /// Full text of the completed content block.
        text: String,
    },

    /// The model emitted a tool_use content block.
    ToolUseRecorded {
        /// Opaque identifier for the tool call.
        call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Tool arguments as JSON.
        args: serde_json::Value,
    },

    /// H08 returned a `ToolResult` for a previously recorded call.
    ToolResultRecorded {
        /// Identifier matching the originating `ToolUseRecorded.call_id`.
        call_id: String,
        /// The tool result.
        result: ToolResult,
    },

    /// The turn paused on an async tool call.
    TurnPaused {
        /// Identifier of the async job being awaited.
        job_id: JobId,
    },

    /// An async job that previously paused this turn has finished.
    JobCompletedRecorded {
        /// Identifier of the completed job.
        job_id: JobId,
        /// The completion event payload (success or failure).
        outcome: JobCompletionEvent,
    },

    /// The turn reached terminal Completed state.
    TurnCompleted {
        /// Outcome detail.
        outcome: TurnOutcome,
    },

    /// The turn ended in failure.
    TurnFailed {
        /// Structured failure reason.
        reason: TurnFailureReason,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope(payload: EventPayload) -> ConversationEvent {
        ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: Some(TurnId::new()),
            seq: 0,
            ts: Utc::now(),
            payload,
        }
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn envelope_uses_adjacent_tagging_in_json() -> serde_json::Result<()> {
        let event = sample_envelope(EventPayload::AssistantMessageAppended {
            text: "hi".into(),
        });
        let json = serde_json::to_string(&event)?;
        // Envelope keys appear at top level; payload is `type` + `data`.
        assert!(json.contains(r#""schema_version":1"#));
        assert!(json.contains(r#""type":"assistant_message_appended""#));
        assert!(json.contains(r#""data":{"text":"hi"}"#));
        Ok(())
    }

    #[test]
    fn all_nine_variants_roundtrip() -> serde_json::Result<()> {
        let variants = vec![
            EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "0.1.0".into(),
                    ..Default::default()
                },
            },
            EventPayload::TurnStarted {
                user_input: vec![ContentBlock::Text { text: "go".into() }],
            },
            EventPayload::AssistantMessageAppended { text: "ok".into() },
            EventPayload::ToolUseRecorded {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({"p": 1}),
            },
            EventPayload::ToolResultRecorded {
                call_id: "c1".into(),
                result: ToolResult::Output(vec![ContentBlock::Text {
                    text: "out".into(),
                }]),
            },
            EventPayload::TurnPaused {
                job_id: JobId::new(),
            },
            EventPayload::JobCompletedRecorded {
                job_id: JobId::new(),
                outcome: JobCompletionEvent::default(),
            },
            EventPayload::TurnCompleted {
                outcome: TurnOutcome::Completed,
            },
            EventPayload::TurnFailed {
                reason: TurnFailureReason::Cancelled,
            },
        ];
        for v in variants {
            let event = sample_envelope(v.clone());
            let json = serde_json::to_string(&event)?;
            let back: ConversationEvent = serde_json::from_str(&json)?;
            assert_eq!(event, back, "variant {v:?} did not roundtrip");
        }
        Ok(())
    }

    #[test]
    fn session_started_carries_no_turn_id() -> serde_json::Result<()> {
        // SessionStarted is session-level; turn_id should be None in idiomatic
        // usage. Serde permits any value here, but assert the canonical shape.
        let event = ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: SessionId::new(),
            turn_id: None,
            seq: 0,
            ts: Utc::now(),
            payload: EventPayload::SessionStarted {
                meta: SessionMeta {
                    cogito_version: "0.1.0".into(),
                    ..Default::default()
                },
            },
        };
        let json = serde_json::to_string(&event)?;
        // `turn_id` is `Option<TurnId>` — when None, serde-json emits `null`
        // (no `skip_serializing_if`). Assert that to lock the wire shape.
        assert!(json.contains(r#""turn_id":null"#));
        let back: ConversationEvent = serde_json::from_str(&json)?;
        assert_eq!(event, back);
        Ok(())
    }

    #[test]
    fn non_exhaustive_keeps_match_arms_safe() {
        // Compile-time check: external code cannot exhaustively match
        // EventPayload. We can match here because we're inside the crate.
        let p = EventPayload::TurnCompleted {
            outcome: TurnOutcome::Completed,
        };
        match p {
            EventPayload::TurnCompleted { .. } => {}
            _ => panic!("wrong variant"),
        }
    }
}
```

- [ ] **Step 2: Wire into lib.rs**

```rust
pub mod event;
pub use event::{ConversationEvent, EventPayload, SCHEMA_VERSION};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cogito-protocol --lib event::tests`
Expected: 5/5 PASS.

If `JobCompletionEvent::default()` doesn't compile, check the actual constructor in `crates/cogito-protocol/src/job.rs` and adapt the test fixture accordingly (Sprint 0 added these types; the exact `Default` impl may differ from what's shown here).

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-protocol/src/event.rs crates/cogito-protocol/src/lib.rs
git commit -m "Add ConversationEvent + EventPayload (9 variants) + SCHEMA_VERSION"
```

---

## Task 6: cogito-protocol `ConversationStore` trait + `StoreError`

**Files:**
- Create: `crates/cogito-protocol/src/store.rs`
- Modify: `crates/cogito-protocol/src/lib.rs`

- [ ] **Step 1: Add `futures` to cogito-protocol if not already present**

Check `crates/cogito-protocol/Cargo.toml`. Ensure `futures.workspace = true` is in `[dependencies]`. If missing, add it.

- [ ] **Step 2: Create the trait + error**

Create `crates/cogito-protocol/src/store.rs`:

```rust
//! `ConversationStore` — Brain-facing persistence trait.
//!
//! See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §3 for the full method semantics. See ADR-0007 for why cross-session /
//! cross-tenant query methods do **not** belong on this trait.

use async_trait::async_trait;
use futures::stream::BoxStream;
use thiserror::Error;

use crate::event::ConversationEvent;
use crate::ids::SessionId;

/// Persistent backend for a session's `ConversationEvent` stream.
///
/// Implementations live in separate crates (`cogito-store-jsonl`,
/// `cogito-store-postgres` v0.4). The Runtime holds **one**
/// `Arc<dyn ConversationStore>` shared by all `SessionActor`s; every
/// method takes the session identifier explicitly.
///
/// Durability semantics are backend-defined; see each backend's crate
/// docs.
#[async_trait]
pub trait ConversationStore: Send + Sync + 'static {
    /// Append a single event. Backends MUST honor `event.seq` and MUST
    /// NOT reorder events. On `Err`, the backend's per-session state is
    /// considered tainted: callers SHOULD `close(session_id)` before
    /// further appends.
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError>;

    /// Flush backend-internal buffers for `session_id`. No-op for backends
    /// without buffering. JSONL flushes its `tokio::fs::File`.
    async fn flush(&self, session_id: SessionId) -> Result<(), StoreError>;

    /// Release per-session resources (file handles, connection slot).
    /// After `close`, subsequent `append` for the same session is valid
    /// and re-acquires resources.
    async fn close(&self, session_id: SessionId) -> Result<(), StoreError>;

    /// Largest `seq` ever appended for `session_id`, or `None` if no
    /// events exist. Used by Sprint 3's H03 Resume Coordinator.
    async fn latest_seq(
        &self,
        session_id: SessionId,
    ) -> Result<Option<u64>, StoreError>;

    /// Stream events where `event.seq > from_seq`, in strict ascending
    /// `seq` order. Use `from_seq = 0` to read from the second event
    /// onward; use the result of `latest_seq` + 1 to read net-new events
    /// after a resume.
    fn replay(
        &self,
        session_id: SessionId,
        from_seq: u64,
    ) -> BoxStream<'_, Result<ConversationEvent, StoreError>>;
}

/// Errors returned by `ConversationStore` methods.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The requested session has no recorded events.
    #[error("session not found: {session_id}")]
    SessionNotFound {
        /// Identifier of the missing session.
        session_id: SessionId,
    },

    /// Underlying I/O failure.
    #[error("backend io error: {source}")]
    Io {
        /// The wrapped I/O error.
        #[from]
        source: std::io::Error,
    },

    /// JSON serialization or parsing failure.
    #[error("serde error: {source}")]
    Serde {
        /// The wrapped serde error.
        #[from]
        source: serde_json::Error,
    },

    /// Schema version of the persisted event is higher than this build
    /// understands. Reader cannot safely process the event.
    #[error("schema version {found} not supported; this build understands <= {supported}")]
    UnsupportedSchemaVersion {
        /// The version found on disk.
        found: u32,
        /// The maximum version this build supports.
        supported: u32,
    },

    /// Backend-specific error with a human-readable message.
    #[error("backend error: {message}")]
    Backend {
        /// Human-readable detail.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check that `ConversationStore` is dyn-safe.
    #[test]
    fn trait_is_dyn_safe() {
        fn assert_dyn_safe(_: &dyn ConversationStore) {}
        // No instance needed; this only checks the trait constructs
        // a valid `dyn` type. The body executes only if called.
        let _ = assert_dyn_safe;
    }

    #[test]
    fn store_error_displays_session_not_found() {
        let sid = SessionId::new();
        let err = StoreError::SessionNotFound { session_id: sid };
        let text = err.to_string();
        assert!(text.starts_with("session not found:"));
    }
}
```

- [ ] **Step 3: Wire into lib.rs**

```rust
pub mod store;
pub use store::{ConversationStore, StoreError};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p cogito-protocol --lib store::tests`
Expected: 2/2 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-protocol/Cargo.toml crates/cogito-protocol/src/store.rs crates/cogito-protocol/src/lib.rs
git commit -m "Add ConversationStore trait + StoreError to cogito-protocol"
```

---

## Task 7: Contract test infrastructure in `cogito-test-fixtures`

**Files:**
- Modify: `testing/cogito-test-fixtures/Cargo.toml`
- Create: `testing/cogito-test-fixtures/src/store_contract.rs`
- Modify: `testing/cogito-test-fixtures/src/lib.rs`

- [ ] **Step 1: Add deps to fixtures crate**

In `testing/cogito-test-fixtures/Cargo.toml`, ensure:

```toml
[dependencies]
cogito-protocol.workspace = true
tokio.workspace = true
futures.workspace = true
chrono.workspace = true
serde_json.workspace = true
```

- [ ] **Step 2: Create the contract test module**

Create `testing/cogito-test-fixtures/src/store_contract.rs`:

```rust
//! Shared `ConversationStore` contract test suite.
//!
//! Every `ConversationStore` implementation MUST pass `run_store_contract`.
//! Backend integration tests look like:
//!
//! ```ignore
//! #[tokio::test]
//! async fn jsonl_passes_store_contract() {
//!     let tmp = tempfile::tempdir().unwrap();
//!     let root = tmp.path().to_path_buf();
//!     cogito_test_fixtures::store_contract::run_store_contract(move || {
//!         let root = root.clone();
//!         async move { Arc::new(JsonlStore::new(root)) as Arc<dyn ConversationStore> }
//!     }).await;
//! }
//! ```

use std::sync::Arc;

use chrono::Utc;
use cogito_protocol::{
    ContentBlock, ConversationEvent, ConversationStore, EventId, EventPayload, SessionId,
    SessionMeta, TurnId, SCHEMA_VERSION,
};
use futures::StreamExt;

/// Build a `SessionStarted` event for `session_id`.
pub fn session_started_event(session_id: SessionId, seq: u64) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: None,
        seq,
        ts: Utc::now(),
        payload: EventPayload::SessionStarted {
            meta: SessionMeta {
                cogito_version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
        },
    }
}

/// Build a `TurnStarted` event with one text user input.
pub fn turn_started_event(
    session_id: SessionId,
    turn_id: TurnId,
    seq: u64,
    text: &str,
) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload: EventPayload::TurnStarted {
            user_input: vec![ContentBlock::Text { text: text.into() }],
        },
    }
}

/// Run the full contract suite. `make_store` is called once at start;
/// the returned `Arc<dyn ConversationStore>` is reused across all
/// sub-tests, so the backend MUST tolerate state from earlier sub-tests
/// (sub-tests use disjoint session_ids to avoid interference).
pub async fn run_store_contract<F>(make_store: F)
where
    F: AsyncFn() -> Arc<dyn ConversationStore>,
{
    let store = make_store().await;
    test_append_then_latest_seq(&*store).await;
    test_append_then_replay_full(&*store).await;
    test_append_then_replay_from_offset(&*store).await;
    test_replay_empty_session_returns_empty_stream(&*store).await;
    test_latest_seq_empty_session_returns_none(&*store).await;
    test_multiple_sessions_isolated(&*store).await;
    test_close_then_reappend(&*store).await;
    test_concurrent_append_two_sessions(&*store).await;
}

async fn test_append_then_latest_seq(store: &dyn ConversationStore) {
    let sid = SessionId::new();
    store
        .append(&session_started_event(sid, 0))
        .await
        .expect("append should succeed");
    store
        .append(&turn_started_event(sid, TurnId::new(), 1, "hi"))
        .await
        .expect("append should succeed");
    let last = store
        .latest_seq(sid)
        .await
        .expect("latest_seq should succeed");
    assert_eq!(last, Some(1), "latest_seq after two appends");
}

async fn test_append_then_replay_full(store: &dyn ConversationStore) {
    let sid = SessionId::new();
    let n = 5;
    for seq in 0..n {
        let event = if seq == 0 {
            session_started_event(sid, seq)
        } else {
            turn_started_event(sid, TurnId::new(), seq, "x")
        };
        store.append(&event).await.expect("append");
    }
    let stream = store.replay(sid, 0);
    let collected: Vec<_> = stream.collect().await;
    // from_seq = 0 means events with seq > 0; one fewer than appended.
    assert_eq!(
        collected.len(),
        (n as usize) - 1,
        "replay(from=0) should return events where seq > 0",
    );
    for r in collected {
        let _ = r.expect("replay item");
    }
}

async fn test_append_then_replay_from_offset(store: &dyn ConversationStore) {
    let sid = SessionId::new();
    for seq in 0..5 {
        store
            .append(&turn_started_event(sid, TurnId::new(), seq, "x"))
            .await
            .expect("append");
    }
    let stream = store.replay(sid, 2);
    let collected: Vec<_> = stream.collect().await;
    assert_eq!(
        collected.len(),
        2,
        "replay(from=2) should return events where seq > 2 (i.e. 3, 4)",
    );
}

async fn test_replay_empty_session_returns_empty_stream(store: &dyn ConversationStore) {
    let sid = SessionId::new();
    let stream = store.replay(sid, 0);
    let collected: Vec<_> = stream.collect().await;
    assert!(
        collected.is_empty(),
        "replay of unknown session should be empty",
    );
}

async fn test_latest_seq_empty_session_returns_none(store: &dyn ConversationStore) {
    let sid = SessionId::new();
    let last = store.latest_seq(sid).await.expect("latest_seq");
    assert_eq!(last, None);
}

async fn test_multiple_sessions_isolated(store: &dyn ConversationStore) {
    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    store
        .append(&session_started_event(sid_a, 0))
        .await
        .expect("a");
    store
        .append(&turn_started_event(sid_a, TurnId::new(), 1, "a1"))
        .await
        .expect("a");
    store
        .append(&session_started_event(sid_b, 0))
        .await
        .expect("b");
    let a_last = store.latest_seq(sid_a).await.expect("a");
    let b_last = store.latest_seq(sid_b).await.expect("b");
    assert_eq!(a_last, Some(1));
    assert_eq!(b_last, Some(0));
}

async fn test_close_then_reappend(store: &dyn ConversationStore) {
    let sid = SessionId::new();
    store
        .append(&session_started_event(sid, 0))
        .await
        .expect("append");
    store.close(sid).await.expect("close");
    store
        .append(&turn_started_event(sid, TurnId::new(), 1, "after-close"))
        .await
        .expect("append after close");
    let last = store.latest_seq(sid).await.expect("latest_seq");
    assert_eq!(last, Some(1));
}

async fn test_concurrent_append_two_sessions(store: &dyn ConversationStore) {
    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    let n = 50;
    let store_a = store;
    let store_b = store;
    let task_a = async {
        for seq in 0..n {
            store_a
                .append(&turn_started_event(sid_a, TurnId::new(), seq, "a"))
                .await
                .expect("a");
        }
    };
    let task_b = async {
        for seq in 0..n {
            store_b
                .append(&turn_started_event(sid_b, TurnId::new(), seq, "b"))
                .await
                .expect("b");
        }
    };
    let (_, _) = tokio::join!(task_a, task_b);
    assert_eq!(store.latest_seq(sid_a).await.expect("a"), Some(n - 1));
    assert_eq!(store.latest_seq(sid_b).await.expect("b"), Some(n - 1));
}
```

NOTE: `AsyncFn` is stable since Rust 1.85; this plan targets edition 2024 / MSRV 1.85. If the implementer hits a stability issue, fall back to:

```rust
pub async fn run_store_contract<F, Fut>(make_store: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Arc<dyn ConversationStore>>,
```

- [ ] **Step 3: Wire into lib.rs**

In `testing/cogito-test-fixtures/src/lib.rs`:

```rust
pub mod store_contract;
```

- [ ] **Step 4: Build (no execution yet — no backend exists)**

Run: `cargo check -p cogito-test-fixtures`
Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add testing/cogito-test-fixtures
git commit -m "Add ConversationStore contract test suite to test-fixtures"
```

---

## Task 8: `cogito-store-jsonl` minimal implementation

**Files:**
- Modify: `crates/cogito-store-jsonl/Cargo.toml`
- Replace: `crates/cogito-store-jsonl/src/lib.rs`

- [ ] **Step 1: Add deps**

In `crates/cogito-store-jsonl/Cargo.toml`, ensure `[dependencies]` includes:

```toml
cogito-protocol.workspace = true
async-trait.workspace = true
async-stream.workspace = true
dashmap.workspace = true
futures.workspace = true
tokio = { workspace = true, features = ["fs", "io-util", "sync"] }
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
```

- [ ] **Step 2: Replace lib.rs with the real implementation**

Open `crates/cogito-store-jsonl/src/lib.rs`. Replace the entire file with:

```rust
//! `cogito-store-jsonl` — JSONL-file-backed `ConversationStore`.
//!
//! **Scope: dev/debug only.** This backend is the v0.1 default while
//! `cogito-store-postgres` is being built. It is intentionally simple:
//!
//! - One file per session at `<root>/<session_id>.jsonl`.
//! - Per-event userspace flush via `tokio::fs::File::flush`.
//! - **No `sync_data` / `fsync`**: process crash is OK; power loss may
//!   lose recent events. Use Postgres (v0.4) for production durability.
//! - No rotation, no path sharding, no internal index.
//!
//! See spec
//! `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
//! §5 for rationale.

#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use cogito_protocol::{
    ConversationEvent, ConversationStore, SessionId, StoreError, SCHEMA_VERSION,
};
use dashmap::DashMap;
use futures::stream::BoxStream;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// JSONL backend for `ConversationStore`. Dev/debug only — see crate docs.
pub struct JsonlStore {
    root: PathBuf,
    handles: DashMap<SessionId, Arc<Mutex<File>>>,
}

impl JsonlStore {
    /// Create a new store rooted at `root`. No I/O is performed; the
    /// directory is created lazily on first append.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            handles: DashMap::new(),
        }
    }

    fn path_for(&self, session_id: &SessionId) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }

    async fn handle_for(&self, session_id: &SessionId) -> Result<Arc<Mutex<File>>, StoreError> {
        if let Some(existing) = self.handles.get(session_id) {
            return Ok(Arc::clone(&existing));
        }
        tokio::fs::create_dir_all(&self.root).await?;
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.path_for(session_id))
            .await?;
        let arc = Arc::new(Mutex::new(file));
        // Race-tolerant insert: if another task raced us, prefer the
        // existing entry to ensure all writers share one handle.
        let entry = self
            .handles
            .entry(*session_id)
            .or_insert_with(|| Arc::clone(&arc));
        Ok(Arc::clone(&entry))
    }
}

#[async_trait]
impl ConversationStore for JsonlStore {
    async fn append(&self, event: &ConversationEvent) -> Result<(), StoreError> {
        let handle = self.handle_for(&event.session_id).await?;
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        let mut file = handle.lock().await;
        file.write_all(&line).await?;
        file.flush().await?;
        Ok(())
    }

    async fn flush(&self, session_id: SessionId) -> Result<(), StoreError> {
        if let Some(handle) = self.handles.get(&session_id) {
            let mut file = handle.lock().await;
            file.flush().await?;
        }
        Ok(())
    }

    async fn close(&self, session_id: SessionId) -> Result<(), StoreError> {
        if let Some((_, handle)) = self.handles.remove(&session_id) {
            let mut file = handle.lock().await;
            file.flush().await?;
            // File handle drops with the Arc.
        }
        Ok(())
    }

    async fn latest_seq(&self, session_id: SessionId) -> Result<Option<u64>, StoreError> {
        let path = self.path_for(&session_id);
        if !path.exists() {
            return Ok(None);
        }
        let text = tokio::fs::read_to_string(&path).await?;
        let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
            return Ok(None);
        };
        let event: ConversationEvent = serde_json::from_str(last)?;
        Ok(Some(event.seq))
    }

    fn replay(
        &self,
        session_id: SessionId,
        from_seq: u64,
    ) -> BoxStream<'_, Result<ConversationEvent, StoreError>> {
        let path = self.path_for(&session_id);
        Box::pin(async_stream::try_stream! {
            let file = match File::open(&path).await {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => Err(StoreError::from(e))?,
            };
            let mut lines = BufReader::new(file).lines();
            while let Some(line) = lines.next_line().await.map_err(StoreError::from)? {
                if line.trim().is_empty() {
                    continue;
                }
                let event: ConversationEvent = serde_json::from_str(&line)?;
                if event.schema_version > SCHEMA_VERSION {
                    Err(StoreError::UnsupportedSchemaVersion {
                        found: event.schema_version,
                        supported: SCHEMA_VERSION,
                    })?;
                }
                if event.seq > from_seq {
                    yield event;
                }
            }
        })
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo check -p cogito-store-jsonl`
Expected: builds cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-store-jsonl
git commit -m "Implement cogito-store-jsonl dev-grade backend"
```

---

## Task 9: JSONL contract test integration

**Files:**
- Modify: `crates/cogito-store-jsonl/Cargo.toml` (dev-deps)
- Create: `crates/cogito-store-jsonl/tests/contract.rs`

- [ ] **Step 1: Add dev-deps**

In `crates/cogito-store-jsonl/Cargo.toml`:

```toml
[dev-dependencies]
cogito-test-fixtures.workspace = true
tempfile.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

(Confirm `tempfile` is in workspace deps; if not, add `tempfile = "3"` at workspace root.)

- [ ] **Step 2: Write the contract test**

Create `crates/cogito-store-jsonl/tests/contract.rs`:

```rust
//! Integration test asserting `JsonlStore` satisfies the
//! `ConversationStore` contract.

use std::sync::Arc;

use cogito_protocol::ConversationStore;
use cogito_store_jsonl::JsonlStore;
use cogito_test_fixtures::store_contract::run_store_contract;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn jsonl_passes_store_contract() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().to_path_buf();
    run_store_contract(move || {
        let store: Arc<dyn ConversationStore> = Arc::new(JsonlStore::new(root.clone()));
        async move { store }
    })
    .await;
    Ok(())
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p cogito-store-jsonl --test contract`
Expected: PASS. The single test invokes all 8 contract sub-tests inside `run_store_contract`.

- [ ] **Step 4: Commit**

```bash
git add crates/cogito-store-jsonl/Cargo.toml crates/cogito-store-jsonl/tests/contract.rs Cargo.toml
git commit -m "Verify JsonlStore satisfies ConversationStore contract"
```

---

## Task 10: H02 `StepRecorder` implementation

**Files:**
- Create: `crates/cogito-core/src/harness/step_recorder.rs`
- Modify: `crates/cogito-core/src/harness/mod.rs`

- [ ] **Step 1: Locate the harness module**

Sprint 0 created `crates/cogito-core/src/harness/` as a stubbed module.
Confirm with: `ls crates/cogito-core/src/harness/`

If the directory does not exist, create `crates/cogito-core/src/harness/mod.rs` with:

```rust
//! H01-H10: the Brain. Components are coordinated by H01 only; see
//! ADR-0004 and AGENTS.md §"Inviolable design principles".

pub mod step_recorder;
pub use step_recorder::StepRecorder;
```

If `mod.rs` already exists, append `pub mod step_recorder;` and `pub use step_recorder::StepRecorder;`.

- [ ] **Step 2: Write the recorder + unit tests in the same file**

Create `crates/cogito-core/src/harness/step_recorder.rs`:

```rust
//! H02 Step Recorder.
//!
//! Owns the live mapping from H01 / H06 events into the two streams:
//!
//! - **Persisted**: `ConversationEvent` written to `ConversationStore`.
//! - **Live broadcast**: `StreamEvent` sent to subscribers.
//!
//! See spec §6 and ADR-0006 §7 for the dual-stream rationale. Text-block
//! batching: per Codex / Claude Code precedent, text deltas are NOT
//! persisted individually. They are accumulated until the wire-protocol
//! `content_block_stop` (text block) boundary, then written as one
//! `AssistantMessageAppended`.

use std::sync::Arc;

use chrono::Utc;
use cogito_protocol::{
    ConversationEvent, ConversationStore, EventId, EventPayload, JobCompletionEvent, JobId,
    SessionId, SessionMeta, StoreError, StreamEvent, TurnFailureReason, TurnId, TurnOutcome,
    SCHEMA_VERSION,
};
use cogito_protocol::content::ContentBlock;
use cogito_protocol::tool::ToolResult;
use tokio::sync::broadcast;

/// H02 Step Recorder.
pub struct StepRecorder {
    store: Arc<dyn ConversationStore>,
    events_tx: broadcast::Sender<StreamEvent>,
    session_id: SessionId,
    seq_counter: u64,
    current_text_block: Option<TextBlockBuf>,
}

struct TextBlockBuf {
    turn_id: TurnId,
    text: String,
}

impl StepRecorder {
    /// Create a recorder bound to `session_id`. `seq_start` is the seq
    /// number to assign to the next appended event; 0 for a fresh session,
    /// `latest_seq + 1` when resuming.
    pub fn new(
        store: Arc<dyn ConversationStore>,
        events_tx: broadcast::Sender<StreamEvent>,
        session_id: SessionId,
        seq_start: u64,
    ) -> Self {
        Self {
            store,
            events_tx,
            session_id,
            seq_counter: seq_start,
            current_text_block: None,
        }
    }

    /// Record the session-open event. Called once per session, before any
    /// turn starts.
    pub async fn record_session_started(
        &mut self,
        meta: SessionMeta,
    ) -> Result<(), StoreError> {
        self.append(None, EventPayload::SessionStarted { meta }).await
    }

    /// Record the start of a new turn and broadcast a live event.
    pub async fn record_turn_started(
        &mut self,
        turn_id: TurnId,
        user_input: Vec<ContentBlock>,
    ) -> Result<(), StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnStarted);
        self.append(
            Some(turn_id),
            EventPayload::TurnStarted { user_input },
        )
        .await
    }

    /// Buffer a streaming text chunk and broadcast it live. Does NOT
    /// persist. Call `on_text_block_complete` when the wire protocol
    /// signals the block is finished.
    pub fn on_text_delta(&mut self, turn_id: TurnId, chunk: String) {
        let _ = self
            .events_tx
            .send(StreamEvent::TextDelta { chunk: chunk.clone() });
        self.current_text_block
            .get_or_insert_with(|| TextBlockBuf {
                turn_id,
                text: String::new(),
            })
            .text
            .push_str(&chunk);
    }

    /// Persist the accumulated text block, if any.
    pub async fn on_text_block_complete(&mut self) -> Result<(), StoreError> {
        let Some(buf) = self.current_text_block.take() else {
            return Ok(());
        };
        self.append(
            Some(buf.turn_id),
            EventPayload::AssistantMessageAppended { text: buf.text },
        )
        .await
    }

    /// Record a tool_use content block and broadcast a live event.
    pub async fn record_tool_use(
        &mut self,
        turn_id: TurnId,
        call_id: String,
        tool_name: String,
        args: serde_json::Value,
    ) -> Result<(), StoreError> {
        let _ = self.events_tx.send(StreamEvent::ToolDispatchStarted {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
        });
        self.append(
            Some(turn_id),
            EventPayload::ToolUseRecorded {
                call_id,
                tool_name,
                args,
            },
        )
        .await
    }

    /// Record a tool_result and broadcast a live event with success flag.
    pub async fn record_tool_result(
        &mut self,
        turn_id: TurnId,
        call_id: String,
        result: ToolResult,
    ) -> Result<(), StoreError> {
        let ok = matches!(result, ToolResult::Output(_));
        let _ = self.events_tx.send(StreamEvent::ToolDispatchEnded {
            call_id: call_id.clone(),
            ok,
        });
        self.append(
            Some(turn_id),
            EventPayload::ToolResultRecorded { call_id, result },
        )
        .await
    }

    /// Record that the turn paused on an async job.
    pub async fn record_turn_paused(
        &mut self,
        turn_id: TurnId,
        job_id: JobId,
    ) -> Result<(), StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnPaused);
        self.append(Some(turn_id), EventPayload::TurnPaused { job_id })
            .await
    }

    /// Record that a previously-awaited job completed.
    pub async fn record_job_completed(
        &mut self,
        turn_id: TurnId,
        job_id: JobId,
        outcome: JobCompletionEvent,
    ) -> Result<(), StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnResumed);
        self.append(
            Some(turn_id),
            EventPayload::JobCompletedRecorded { job_id, outcome },
        )
        .await
    }

    /// Record successful turn completion.
    pub async fn record_turn_completed(
        &mut self,
        turn_id: TurnId,
        outcome: TurnOutcome,
    ) -> Result<(), StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnCompleted);
        self.append(Some(turn_id), EventPayload::TurnCompleted { outcome })
            .await
    }

    /// Record turn failure with a human-readable reason for subscribers.
    pub async fn record_turn_failed(
        &mut self,
        turn_id: TurnId,
        reason: TurnFailureReason,
    ) -> Result<(), StoreError> {
        let _ = self.events_tx.send(StreamEvent::TurnFailed {
            reason: reason.to_string(),
        });
        self.append(Some(turn_id), EventPayload::TurnFailed { reason })
            .await
    }

    async fn append(
        &mut self,
        turn_id: Option<TurnId>,
        payload: EventPayload,
    ) -> Result<(), StoreError> {
        let event = ConversationEvent {
            schema_version: SCHEMA_VERSION,
            event_id: EventId::new(),
            session_id: self.session_id,
            turn_id,
            seq: self.seq_counter,
            ts: Utc::now(),
            payload,
        };
        self.store.append(&event).await?;
        self.seq_counter = self.seq_counter.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use cogito_store_jsonl::JsonlStore;

    fn fresh_store_in(dir: &std::path::Path) -> Arc<dyn ConversationStore> {
        Arc::new(JsonlStore::new(dir.to_path_buf()))
    }

    #[tokio::test]
    async fn text_block_lifecycle_persists_one_event() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(store.clone(), tx, sid, 0);

        let turn = TurnId::new();
        rec.on_text_delta(turn, "hello ".into());
        rec.on_text_delta(turn, "world".into());
        // No store write yet.
        assert_eq!(store.latest_seq(sid).await?, None);

        rec.on_text_block_complete().await?;
        // Exactly one event persisted.
        assert_eq!(store.latest_seq(sid).await?, Some(0));
        Ok(())
    }

    #[tokio::test]
    async fn text_block_lifecycle_combines_full_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(store.clone(), tx, sid, 0);
        let turn = TurnId::new();
        rec.on_text_delta(turn, "foo".into());
        rec.on_text_delta(turn, "bar".into());
        rec.on_text_block_complete().await?;

        use futures::StreamExt;
        let mut stream = store.replay(sid, u64::MAX); // read no offsets
        drop(stream);
        let mut stream = Box::pin(store.replay(sid, 0));
        // seq 0 emitted, so replay(0) returns events where seq > 0 → nothing.
        // Use replay with a virtual offset of -1 by using 0 and checking seq=0 separately.
        // Instead, just count via latest_seq:
        assert_eq!(store.latest_seq(sid).await?, Some(0));
        // And verify the content by reading the file directly.
        let entries: Vec<_> = tokio::fs::read_dir(tmp.path()).await?.next_entry().await?.into_iter().collect();
        assert_eq!(entries.len(), 1);
        let path = entries[0].path();
        let text = tokio::fs::read_to_string(&path).await?;
        assert!(text.contains("\"text\":\"foobar\""), "got: {text}");
        let _ = stream.next();
        Ok(())
    }

    #[tokio::test]
    async fn text_block_complete_without_deltas_is_noop()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(store.clone(), tx, sid, 0);
        rec.on_text_block_complete().await?;
        assert_eq!(store.latest_seq(sid).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn seq_counter_is_monotonic() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = fresh_store_in(tmp.path());
        let (tx, _rx) = broadcast::channel(64);
        let sid = SessionId::new();
        let mut rec = StepRecorder::new(store.clone(), tx, sid, 0);

        rec.record_session_started(SessionMeta {
            cogito_version: "0.1.0".into(),
            ..Default::default()
        })
        .await?;
        let turn = TurnId::new();
        rec.record_turn_started(turn, vec![ContentBlock::Text { text: "hi".into() }])
            .await?;
        rec.record_turn_completed(turn, TurnOutcome::Completed).await?;

        assert_eq!(store.latest_seq(sid).await?, Some(2));
        Ok(())
    }
}
```

NOTE on the second test (`text_block_lifecycle_combines_full_text`): the
implementer should clean up the awkward stream dance. Replace it with a
straight file-read assertion if simpler. The intent is to verify the
combined text is `"foobar"`.

- [ ] **Step 3: Add dev-deps to cogito-core**

In `crates/cogito-core/Cargo.toml` `[dev-dependencies]`, ensure:

```toml
cogito-store-jsonl.workspace = true
tempfile.workspace = true
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p cogito-core --lib harness::step_recorder::tests`
Expected: 4/4 PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cogito-core
git commit -m "Implement H02 StepRecorder with content_block-boundary text batching"
```

---

## Task 11: `cogito-gen-schema` real implementation

**Files:**
- Modify: `tools/cogito-gen-schema/src/main.rs`
- Modify: `justfile`

- [ ] **Step 1: Implement schema generation**

Replace `tools/cogito-gen-schema/src/main.rs`:

```rust
//! Generate JSON Schema for cogito-protocol public types.
//!
//! Invoked via `just gen-schema`. CI runs `just gen-schema-check` to
//! enforce drift-free committed schema files.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use cogito_protocol::ConversationEvent;
use schemars::schema_for;

#[derive(Parser)]
#[command(name = "cogito-gen-schema", about)]
struct Args {
    /// Output path for the generated JSON Schema.
    #[arg(long)]
    output: PathBuf,

    /// If set, compare generated schema against the file at `--output`
    /// and exit non-zero if they differ. Does not write.
    #[arg(long, default_value_t = false)]
    check: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let schema = schema_for!(ConversationEvent);
    let generated = serde_json::to_string_pretty(&schema)
        .context("serializing schema to JSON")?;
    // Trailing newline so the file plays well with text-editing tools.
    let generated = format!("{generated}\n");

    if args.check {
        let existing = std::fs::read_to_string(&args.output)
            .with_context(|| format!("reading {}", args.output.display()))?;
        if existing != generated {
            bail!(
                "schema drift detected: {} differs from generated output. \
                 Run `just gen-schema` and commit.",
                args.output.display()
            );
        }
        Ok(())
    } else {
        if let Some(parent) = args.output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir -p {}", parent.display()))?;
        }
        std::fs::write(&args.output, generated.as_bytes())
            .with_context(|| format!("writing {}", args.output.display()))?;
        eprintln!("wrote {}", args.output.display());
        Ok(())
    }
}
```

- [ ] **Step 2: Add justfile recipes**

Open `justfile`. Add recipes at the bottom (preserving existing recipe style):

```just
# Regenerate JSON Schema for ConversationEvent into docs/schemas/.
gen-schema:
    cargo run -p cogito-gen-schema --release -- \
        --output docs/schemas/conversation-event-v1.json

# Verify committed schema matches the current Rust types (CI gate).
gen-schema-check:
    cargo run -p cogito-gen-schema --release -- \
        --output docs/schemas/conversation-event-v1.json \
        --check
```

- [ ] **Step 3: Generate the schema file**

```bash
just gen-schema
```

Expected: writes `docs/schemas/conversation-event-v1.json` (a valid JSON Schema doc, probably ~300-600 lines).

- [ ] **Step 4: Verify check mode is silent on a fresh generation**

```bash
just gen-schema-check
```

Expected: exits 0 with no diff message.

- [ ] **Step 5: Commit**

```bash
git add tools/cogito-gen-schema/src/main.rs justfile docs/schemas/conversation-event-v1.json
git commit -m "Implement cogito-gen-schema + generate conversation-event-v1.json"
```

---

## Task 12: Sample fixture file

**Files:**
- Create: `testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
- Create: `testing/cogito-test-fixtures/src/fixtures.rs` (helper to build the sample)
- Modify: `testing/cogito-test-fixtures/src/lib.rs`
- Create: `testing/cogito-test-fixtures/tests/fixture_roundtrip.rs`

- [ ] **Step 1: Write a fixture builder**

Create `testing/cogito-test-fixtures/src/fixtures.rs`:

```rust
//! Canonical sample fixtures used by both contract tests and external
//! readers as a worked example of the v1 JSONL schema.

use chrono::{TimeZone, Utc};
use cogito_protocol::{
    ContentBlock, ConversationEvent, EventId, EventPayload, JobCompletionEvent, JobId,
    SessionId, SessionMeta, TurnId, TurnOutcome, SCHEMA_VERSION,
};
use cogito_protocol::tool::ToolResult;
use ulid::Ulid;

/// Build the canonical sample session: one session covering all 9 event
/// variants in their natural turn order, with deterministic identifiers
/// and timestamps so the JSONL file is byte-reproducible.
pub fn canonical_sample_session() -> Vec<ConversationEvent> {
    // Deterministic IDs: Ulid::from_string from fixed inputs.
    let sid = SessionId::from(
        Ulid::from_string("01J9C0R0K0SESSION0SESSION0").expect("fixed ulid"),
    );
    let turn = TurnId::from(
        Ulid::from_string("01J9C0R0K0TURN0TURN0TURN00").expect("fixed ulid"),
    );
    let job = JobId::from_ulid(
        Ulid::from_string("01J9C0R0K0JOB0JOB0JOB0JOB0").expect("fixed ulid"),
    );

    // Deterministic event_id: increment on each event using ulid::Ulid
    // from sortable timestamps.
    let mut counter: u64 = 0;
    let mut next_event_id = || {
        counter += 1;
        EventId::from(
            Ulid::from_parts(counter, 0),
        )
    };

    let ts0 = Utc.with_ymd_and_hms(2026, 5, 18, 10, 0, 0).unwrap();

    let envelope = |seq: u64, turn_id: Option<TurnId>, payload: EventPayload| ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: next_event_id(),
        session_id: sid,
        turn_id,
        seq,
        ts: ts0 + chrono::Duration::milliseconds((seq * 100) as i64),
        payload,
    };

    let mut events = Vec::new();
    events.push(envelope(0, None, EventPayload::SessionStarted {
        meta: SessionMeta {
            cogito_version: "0.1.0".into(),
            strategy: Some("default".into()),
            model: Some("claude-sonnet-4-6".into()),
            user_id: Some("u_42".into()),
            ..Default::default()
        },
    }));
    events.push(envelope(1, Some(turn), EventPayload::TurnStarted {
        user_input: vec![ContentBlock::Text { text: "read /tmp/x".into() }],
    }));
    events.push(envelope(2, Some(turn), EventPayload::AssistantMessageAppended {
        text: "Reading /tmp/x now.".into(),
    }));
    events.push(envelope(3, Some(turn), EventPayload::ToolUseRecorded {
        call_id: "toolu_01".into(),
        tool_name: "read_file".into(),
        args: serde_json::json!({"path": "/tmp/x"}),
    }));
    events.push(envelope(4, Some(turn), EventPayload::ToolResultRecorded {
        call_id: "toolu_01".into(),
        result: ToolResult::Output(vec![ContentBlock::Text {
            text: "file contents".into(),
        }]),
    }));
    events.push(envelope(5, Some(turn), EventPayload::TurnPaused { job_id: job }));
    events.push(envelope(6, Some(turn), EventPayload::JobCompletedRecorded {
        job_id: job,
        outcome: JobCompletionEvent::default(),
    }));
    events.push(envelope(7, Some(turn), EventPayload::TurnCompleted {
        outcome: TurnOutcome::Completed,
    }));
    events.push(envelope(8, Some(turn), EventPayload::TurnFailed {
        reason: cogito_protocol::TurnFailureReason::Cancelled,
    }));

    events
}

/// Serialize the canonical sample to JSONL bytes.
pub fn canonical_sample_jsonl() -> Vec<u8> {
    let mut buf = Vec::new();
    for event in canonical_sample_session() {
        let mut line = serde_json::to_vec(&event).expect("event serializes");
        line.push(b'\n');
        buf.extend_from_slice(&line);
    }
    buf
}
```

Adapt `JobId::from_ulid` / `EventId::from(Ulid)` to whatever the actual constructors accept (Task 2 used `From<Ulid>` for the ID types, so `EventId::from(ulid)` works; `JobId` was added in Sprint 0 — check `crates/cogito-protocol/src/job.rs` for its constructor surface). If `JobId` does not accept a `Ulid`, use whatever `JobId::new()` produces and accept non-reproducibility for the job_id field only.

- [ ] **Step 2: Wire into lib.rs**

In `testing/cogito-test-fixtures/src/lib.rs`:

```rust
pub mod fixtures;
```

- [ ] **Step 3: Generate the fixture file via a small dev-time script**

The cleanest way: write a one-off `cargo run -p cogito-test-fixtures --bin write-sample` binary. Add to `testing/cogito-test-fixtures/Cargo.toml`:

```toml
[[bin]]
name = "write-sample"
path = "src/bin/write_sample.rs"
required-features = []
```

Create `testing/cogito-test-fixtures/src/bin/write_sample.rs`:

```rust
//! Write the canonical sample JSONL fixture to its checked-in path.

use anyhow::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
    let bytes = cogito_test_fixtures::fixtures::canonical_sample_jsonl();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions/sample-v1.jsonl");
    std::fs::create_dir_all(out.parent().unwrap())?;
    std::fs::write(&out, &bytes)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}
```

Run: `cargo run -p cogito-test-fixtures --bin write-sample`

Expected: writes `testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`.

- [ ] **Step 4: Write a roundtrip test that locks the fixture**

Create `testing/cogito-test-fixtures/tests/fixture_roundtrip.rs`:

```rust
//! Asserts the checked-in sample fixture parses back into the in-code
//! canonical session. Fails if either drifts.

use cogito_protocol::ConversationEvent;
use cogito_test_fixtures::fixtures::canonical_sample_session;

#[test]
fn sample_fixture_parses_to_canonical_session() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/sessions/sample-v1.jsonl");
    let text = std::fs::read_to_string(&path)?;
    let parsed: Vec<ConversationEvent> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()?;
    let expected = canonical_sample_session();
    assert_eq!(parsed, expected, "fixture file drifted from canonical session");
    Ok(())
}
```

- [ ] **Step 5: Run the roundtrip test**

```bash
cargo test -p cogito-test-fixtures --test fixture_roundtrip
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add testing/cogito-test-fixtures
git commit -m "Add canonical v1 JSONL fixture + roundtrip lock"
```

---

## Task 13: `append_throughput` benchmark + baseline doc

**Files:**
- Modify: `crates/cogito-store-jsonl/Cargo.toml`
- Create: `crates/cogito-store-jsonl/benches/append_throughput.rs`
- Modify: `justfile`
- Create: `docs/quality/v0.1-jsonl-baseline.md` (initial template, filled by bench run)

- [ ] **Step 1: Add criterion dev-dep + bench config**

In `crates/cogito-store-jsonl/Cargo.toml`:

```toml
[dev-dependencies]
# … existing …
criterion = { workspace = true, features = ["async_tokio"] }

[[bench]]
name = "append_throughput"
harness = false
```

- [ ] **Step 2: Write the benchmark**

Create `crates/cogito-store-jsonl/benches/append_throughput.rs`:

```rust
//! `append_throughput` — measures `JsonlStore::append` latency and
//! throughput for the v0.1 dev-grade baseline. Results inform
//! `docs/quality/v0.1-jsonl-baseline.md`. **Not** a production SLO
//! lock — see ADR-0005 §3 footnote and ADR-0007.

use std::sync::Arc;

use chrono::Utc;
use cogito_protocol::{
    ConversationEvent, ConversationStore, EventId, EventPayload, SessionId, TurnId,
    SCHEMA_VERSION,
};
use cogito_store_jsonl::JsonlStore;
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

fn build_event(session_id: SessionId, turn_id: TurnId, seq: u64) -> ConversationEvent {
    ConversationEvent {
        schema_version: SCHEMA_VERSION,
        event_id: EventId::new(),
        session_id,
        turn_id: Some(turn_id),
        seq,
        ts: Utc::now(),
        payload: EventPayload::ToolUseRecorded {
            call_id: "toolu_bench".into(),
            tool_name: "noop_bench".into(),
            args: serde_json::json!({
                "param_a": "value-with-some-bytes",
                "param_b": 42,
                "param_c": [1, 2, 3, 4, 5, 6, 7, 8],
            }),
        },
    }
}

fn bench_append(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    c.bench_function("jsonl_append_single_event", |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let tmp = tempfile::tempdir().expect("tmp dir");
            let store: Arc<dyn ConversationStore> =
                Arc::new(JsonlStore::new(tmp.path()));
            let sid = SessionId::new();
            let tid = TurnId::new();
            let start = std::time::Instant::now();
            for seq in 0..iters {
                let event = build_event(sid, tid, seq);
                store.append(&event).await.expect("append");
            }
            start.elapsed()
        })
    });
}

criterion_group!(benches, bench_append);
criterion_main!(benches);
```

- [ ] **Step 3: Add `bench-baseline` justfile recipe**

```just
# Run JSONL append baseline benchmark. Output lands in target/criterion.
bench-baseline:
    cargo bench -p cogito-store-jsonl --bench append_throughput
```

- [ ] **Step 4: Run the benchmark**

```bash
just bench-baseline 2>&1 | tee /tmp/cogito-bench.log
```

Expected: criterion writes `target/criterion/jsonl_append_single_event/`. Log shows P50 / P99 figures.

- [ ] **Step 5: Capture baseline doc**

Create `docs/quality/v0.1-jsonl-baseline.md` (replace the metric values with actual readings from Step 4):

```markdown
# v0.1 JSONL Append Baseline (informational)

> Measured against `cogito-store-jsonl` via
> `crates/cogito-store-jsonl/benches/append_throughput.rs`.
>
> **This is a dev-grade backend baseline, not a production SLO.** The
> ADR-0005 §3 P99 < 5 ms target is locked at v0.4 against
> `cogito-store-postgres` under production-realistic load.

## Environment

- Host: `$(uname -a)`
- Date: 2026-05-18
- Rust: `$(rustc --version)`
- JSONL durability: userspace flush only (`tokio::fs::File::flush`); no `sync_data` / `fsync`.

## Results — `jsonl_append_single_event`

| Metric | Value |
|---|---|
| Mean per-append latency | _fill from criterion output_ |
| P50 latency | _fill_ |
| P99 latency | _fill_ |
| Throughput | _fill_ events/sec |
| Sample event payload size | ~200 bytes |

## Reproducing

```bash
just bench-baseline
```

Criterion HTML reports under `target/criterion/jsonl_append_single_event/`.
```

Fill the table with actual numbers from `/tmp/cogito-bench.log`.

- [ ] **Step 6: Commit**

```bash
git add crates/cogito-store-jsonl/Cargo.toml crates/cogito-store-jsonl/benches/append_throughput.rs justfile docs/quality/v0.1-jsonl-baseline.md
git commit -m "Add JSONL append benchmark + dev-grade baseline doc"
```

---

## Task 14: ADR-0007 (Event log as cross-language contract)

**Files:**
- Create: `docs/adr/0007-event-log-as-cross-language-contract.md`
- Modify: `docs/adr/README.md`
- Modify: `docs/adr/0005-production-scope-and-quality-gates.md` (§3 footnote)

- [ ] **Step 1: Write ADR-0007**

Create `docs/adr/0007-event-log-as-cross-language-contract.md`:

```markdown
# ADR-0007: Event log as cross-language storage contract

## Status

Accepted

## Context

cogito ships as an embeddable Rust library. The first SaaS deployment
profile (ADR-0005 §2) co-locates a Rust process running cogito with one
or more **non-Rust services** (Go HTTP API, Python analytics, Node BFF)
that need to consume the conversation event log for user-facing query,
audit, billing, and dashboards.

These external readers cannot consume a Rust trait. They consume the
**storage itself** — JSONL files in v0.1 dev/debug deployments, the
Postgres schema in v0.4+ production deployments, and any future
backend (S3, Kafka, …).

Earlier brainstorming (2026-05-18 Q2) proposed two Rust traits
(`ConversationStore` + `ConversationCatalog`) to serve both Brain-side
writes and external-side reads. That framing was wrong: only the
Brain-side path can be served by a Rust trait. The external-side path
is necessarily storage-level.

## Decision

The `ConversationStore` Rust trait (`cogito-protocol::store`) serves
**Brain's command path + single-session replay only**. Methods on this
trait MUST be scoped to:

1. Writing one `ConversationEvent`.
2. Reading events for one explicitly-named `SessionId`.

Any cross-session, cross-tenant, or user-facing query capability —
"list conversations for user U", "search across tenants", "aggregate
billing per day" — is exposed via the **storage-level contract**, not
via Rust traits.

### Storage-level contracts cogito commits to

| Backend | Public contract | First shipped |
|---|---|---|
| `cogito-store-jsonl` (dev/debug) | JSONL line format documented at `docs/data-model/jsonl-v1.md` | v0.1 |
| `cogito-store-postgres` (production) | SQL DDL at `crates/cogito-store-postgres/migrations/0001_init.sql` | v0.4 |
| Future backends (S3, Kafka) | Backend-specific format docs | TBD |

Each storage contract is governed by `ConversationEvent::schema_version`
(ADR-0005 §4 #2). The same versioning and migration rules apply
regardless of which storage backend a reader is using.

### What this means for cogito's deliverables

- `ConversationEvent` Rust types live in `cogito-protocol::event`.
- A JSON Schema artifact (`docs/schemas/conversation-event-v1.json`)
  is generated from those types via `cogito-gen-schema` and committed
  to the repo. CI enforces no drift. External Go/Python/Node services
  can use this schema for typed deserialization or code generation.
- A canonical fixture (`testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`)
  covers all 9 `EventPayload` variants and serves as a worked example
  for both internal contract tests and external readers.
- The JSONL line format spec at `docs/data-model/jsonl-v1.md` is a
  human-readable companion to the JSON Schema.

### Inviolable design rule added to `AGENTS.md`

> `ConversationStore` is Brain's command + single-session replay trait.
> Adding any cross-session, cross-tenant, or user-history query method
> to this trait is a design error. Cross-session / catalog access for
> external services is served by reading the underlying storage
> directly (per this ADR).

## Consequences

- **Easier**: external readers do not depend on the Rust compilation
  unit; they consume a stable, language-neutral artifact (JSONL bytes /
  Postgres rows / JSON Schema). Cogito's release cadence does not
  block their development.
- **Easier**: `ConversationStore`'s surface stays minimal and
  evolvable independently from query/catalog concerns.
- **Harder**: the JSONL line format and Postgres DDL become public API
  with the same SemVer obligations as Rust public symbols. Changes
  require the migration tooling outlined in ADR-0005 §4 #2.
- **Harder**: new read access patterns cannot be added by extending
  the Rust trait — they require schema design that works for SQL +
  file-scan + future backends.

## Follow-on work

- v0.1 Sprint 1: deliver the JSON Schema artifact + fixture + JSONL
  spec doc; commit the inviolable rule.
- v0.4: deliver the Postgres DDL as the second canonical storage
  contract; lock its forward-compatibility with the same
  `schema_version` mechanism.
- v0.4 onward: any new storage backend ships with its own contract doc
  alongside its implementation.

## References

- ADR-0001 (workspace layout)
- ADR-0002 (event sourcing)
- ADR-0005 (production scope + quality gates §4 #2 schema_version)
- Spec `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md` §4
- `AGENTS.md` (new inviolable rule under §"Inviolable design principles")
```

- [ ] **Step 2: Update ADR-0005 §3 footnote**

Open `docs/adr/0005-production-scope-and-quality-gates.md`. In §3 SLO table, append a footnote line below the table (preserving the existing table):

```markdown
> _† JSONL backend baseline (see `docs/quality/v0.1-jsonl-baseline.md`)
> is informational only; the production SLO is locked at v0.4 against
> `cogito-store-postgres`._
```

And in the row "P99 step record write latency", append `†` to the metric name:

```diff
-| P99 step record write latency | < 5 ms | H02 + `ConversationStore` impl |
+| P99 step record write latency † | < 5 ms | H02 + `ConversationStore` impl |
```

- [ ] **Step 3: Update ADR index**

Open `docs/adr/README.md`. Add to the index table/list:

```markdown
| 0007 | Event log as cross-language storage contract | Accepted |
```

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0007-event-log-as-cross-language-contract.md docs/adr/0005-production-scope-and-quality-gates.md docs/adr/README.md
git commit -m "Ratify ADR-0007: event log as cross-language storage contract"
```

---

## Task 15: AGENTS.md amendments (two rules)

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Replace the text-delta batching exception**

Open `AGENTS.md`. Locate the §"Inviolable design principles" → "### 2. H02 Step Recorder writes events immediately" subsection. Replace the body:

```diff
-### 2. H02 Step Recorder writes events immediately
-
-No batching. No buffering across components. The only exception is
-`text_delta` events, which may be batched for ≤200ms or ≤500 chars,
-then flushed.
+### 2. H02 Step Recorder writes events immediately
+
+No batching. No buffering across components. `StreamEvent::TextDelta`
+is live-only (never persisted by H02). Persistence happens at the
+wire-protocol content_block boundary: when the demultiplexer signals
+`text_block_complete`, H02 writes one `AssistantMessageAppended`
+carrying the full block text. This matches Codex and Claude Code,
+both of which align persistence with content_block boundaries. No
+timer-based or size-based batching exists.
```

- [ ] **Step 2: Add the new catalog-scope rule**

Locate the last numbered inviolable rule (currently "### 6"). Add a new "### 7" after it:

```markdown
### 7. `ConversationStore` is Brain's command + single-session replay trait

Methods on `ConversationStore` (`cogito-protocol::store`) MUST be
scoped to: (a) writing one event, (b) reading events for one
explicitly-named session. Adding any cross-session, cross-tenant, or
user-history query method to this trait is a design error.

Cross-session / catalog access for external (Go/Python/Node) services
is served by reading the underlying storage directly (JSONL files in
v0.1 dev/debug; Postgres tables in v0.4 production). See ADR-0007 for
the principle and ADR-0012 (v0.4) for the `TenantContext` model.
```

- [ ] **Step 3: Verify file still parses cleanly**

Run: `head -120 AGENTS.md`
Expected: visible rules 1-7, no stray markup.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "Amend AGENTS.md: text-delta lifecycle (§2) + ConversationStore scope (§7)"
```

---

## Task 16: `docs/data-model/jsonl-v1.md` + H02 component doc update

**Files:**
- Create: `docs/data-model/jsonl-v1.md`
- Modify: `docs/components/H02-step-recorder.md`

- [ ] **Step 1: Write the JSONL data-model doc**

Create `docs/data-model/jsonl-v1.md`:

```markdown
# JSONL Event Log — schema v1

> **Status**: stable for the cogito 0.x line. Governed by
> `ConversationEvent::schema_version` per ADR-0005 §4 #2 and ADR-0007.
>
> **Audience**: external (Go / Python / Node) services reading the
> conversation event log; cogito library consumers writing custom
> backends.

## File layout

- One file per session at `<root>/<session_id>.jsonl`.
- `<session_id>` is a [ULID](https://github.com/ulid/spec) rendered as
  the canonical 26-character Crockford base32 string.
- Lines are UTF-8 JSON objects terminated by `\n`. No leading whitespace.
- The file is **append-only** during writer activity. Readers MAY tail
  the file but MUST handle truncated final lines gracefully (the writer
  may be mid-write).
- The first non-empty line of every file is the `SessionStarted` event
  (`type = "session_started"`).
- Lines are in strict ascending `seq` order. Gaps in `seq` are
  forbidden; readers encountering a gap SHOULD treat the file as
  corrupt.

## Line schema (envelope)

```json
{
  "schema_version": 1,
  "event_id": "01J9C0R0K3T0X8K3T0X8K3T0X8",
  "session_id": "01J9C0R0K0SESSION0SESSION0",
  "turn_id": "01J9C0R0K0TURN0TURN0TURN00",
  "seq": 42,
  "ts": "2026-05-18T10:00:00.123Z",
  "type": "tool_use_recorded",
  "data": {"call_id": "toolu_01", "tool_name": "read_file", "args": {"path": "/tmp/x"}}
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `schema_version` | int | ✓ | `1` for this version. Bumped together with the envelope or any payload shape on breaking change. |
| `event_id` | ULID string | ✓ | Globally unique, monotonic per writing process. |
| `session_id` | ULID string | ✓ | Identifies the session. Matches the filename. |
| `turn_id` | ULID string \| `null` | ✓ | `null` for session-level events (e.g. `session_started`). |
| `seq` | uint64 | ✓ | Monotonic per session, starts at 0. Used by Resume Coordinator. |
| `ts` | RFC 3339 timestamp (UTC) | ✓ | Wall-clock at write time. Use for display only; causality is `seq`. |
| `type` | string | ✓ | One of the 9 payload variants below (snake_case). |
| `data` | object | ✓ | Variant-specific payload; see "Payload variants" below. |

## Payload variants (9)

### `session_started`

```json
{"type": "session_started", "data": {"meta": {"cogito_version": "0.1.0", "strategy": "default", "model": "claude-sonnet-4-6", "user_id": "u_42"}}}
```

Always the first line of a file. `meta` carries optional consumer-supplied
metadata (see `SessionMeta` schema in `docs/schemas/conversation-event-v1.json`).

### `turn_started`

```json
{"type": "turn_started", "data": {"user_input": [{"type": "text", "data": {"text": "read /tmp/x"}}]}}
```

`user_input` is a `Vec<ContentBlock>`. v1 supports `text`, `tool_use`,
`tool_result` content block types.

### `assistant_message_appended`

```json
{"type": "assistant_message_appended", "data": {"text": "Reading /tmp/x now."}}
```

One per wire-protocol content_block_stop for an assistant text block.
The recorder does NOT persist individual streaming deltas — they appear
only on the live `StreamEvent` channel.

### `tool_use_recorded`

```json
{"type": "tool_use_recorded", "data": {"call_id": "toolu_01", "tool_name": "read_file", "args": {"path": "/tmp/x"}}}
```

### `tool_result_recorded`

```json
{"type": "tool_result_recorded", "data": {"call_id": "toolu_01", "result": {"Output": [{"type": "text", "data": {"text": "file contents"}}]}}}
```

### `turn_paused`

```json
{"type": "turn_paused", "data": {"job_id": "01J9C0R0K0JOB0JOB0JOB0JOB0"}}
```

### `job_completed_recorded`

```json
{"type": "job_completed_recorded", "data": {"job_id": "01J9C0R0K0JOB0JOB0JOB0JOB0", "outcome": { /* JobCompletionEvent */ }}}
```

### `turn_completed`

```json
{"type": "turn_completed", "data": {"outcome": "Completed"}}
```

### `turn_failed`

```json
{"type": "turn_failed", "data": {"reason": "Cancelled"}}
```

## Forward compatibility

- **Additive changes** (new `EventPayload` variant, new optional field
  on `SessionMeta`) do NOT bump `schema_version`. Readers MUST tolerate
  unknown `type` values (skip the line or log) and MUST ignore unknown
  object keys.
- **Breaking changes** (rename a field, change a field's type, remove a
  variant) bump `schema_version`. cogito ships a migration tool for
  every breaking 0.x change; readers SHOULD pin their understanding to
  a known `schema_version` window.

## Validation

A JSON Schema artifact is generated from the Rust source at
`docs/schemas/conversation-event-v1.json`. CI ensures it does not drift
from the implementation. External services SHOULD use it for typed
deserialization.

## Canonical example

A worked sample of all 9 variants in one session is at
`testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`.
```

- [ ] **Step 2: Update H02 component doc**

Open `docs/components/H02-step-recorder.md`. Append a new section before the existing "Implementation note" (or at the end if no such section yet):

```markdown
## Text block lifecycle

Per ADR-0007 + spec §6.1, H02 batches text content by **wire-protocol
content_block boundary**, not by timer or character threshold. The
lifecycle is:

1. H06 emits `text_delta` chunks for the current content_block.
   `StepRecorder::on_text_delta` accumulates them into
   `current_text_block.text` AND broadcasts each chunk as
   `StreamEvent::TextDelta` for live subscribers. **Nothing is
   persisted yet.**
2. H06 emits `text_block_complete` when the model signals
   `content_block_stop` for a text block.
   `StepRecorder::on_text_block_complete` writes one
   `AssistantMessageAppended` carrying the full accumulated text and
   clears the buffer.

On crash mid-block: the recorder dies with the SessionActor (no
cross-turn state per ADR-0006 §3). The accumulated text is lost.
Resume restarts the turn from `ModelCalling`, the model re-streams,
and no partial assistant message ends up in the persisted log.

This matches Codex's `should_persist_event_msg` (filters out
`AgentMessageDelta`, persists only `AgentMessage`) and Claude Code's
behavior.
```

- [ ] **Step 3: Commit**

```bash
git add docs/data-model/jsonl-v1.md docs/components/H02-step-recorder.md
git commit -m "Add JSONL v1 data-model doc + H02 text-block lifecycle"
```

---

## Task 17: CI integration + ROADMAP + CHANGELOG

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `ROADMAP.md`
- Create or modify: `CHANGELOG.md`

- [ ] **Step 1: Add schema-check job to CI**

Open `.github/workflows/ci.yml`. Add a new job alongside the existing format/clippy/test/deny/layer-check jobs:

```yaml
  schema-check:
    name: gen-schema --check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Verify docs/schemas/conversation-event-v1.json is up-to-date
        run: cargo run -p cogito-gen-schema --release -- \
             --output docs/schemas/conversation-event-v1.json --check
```

- [ ] **Step 2: Update ROADMAP Sprint 1 checklist**

Open `ROADMAP.md`. Locate "Sprint 1 · H02 Step Recorder + JSONL store". Mark items completed (`- [x]`) according to what this plan delivers:

```markdown
#### Sprint 1 · H02 Step Recorder + JSONL store (1.5 day)
- [x] `cogito-protocol` defines `ConversationEvent` with `schema_version: u32` + `Vec<ContentBlock>` payload (Text + ToolUse + ToolResult variants)
- [x] `cogito-protocol` defines `ConversationStore` trait
- [x] `cogito-store-jsonl` implementation: per-session file, `flush` per event, append-only (durability scope: dev/debug — see ADR-0007)
- [x] Contract test infrastructure (shared test consumed by every backend crate)
- [x] `cogito-core::harness::step_recorder` writes events
- [x] Text-block batching: per content_block boundary (matches Codex / Claude Code; see ADR-0007 + H02 doc)
- [x] Benchmark: `append_throughput` against JSONL; baseline at `docs/quality/v0.1-jsonl-baseline.md` (informational only, ADR-0005 §3 footnote)
- [x] ADR-0007 ratified (event log as cross-language contract)
- [x] JSON Schema artifact at `docs/schemas/conversation-event-v1.json` + CI drift gate
- [x] Canonical fixture at `testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
- [x] `AGENTS.md` §2 + §7 inviolable rules amended
```

If the existing checklist text differs, adapt to it — preserve unmodified items and only mark v0.1 Sprint 1 deliverables as done.

- [ ] **Step 3: Initialize/update CHANGELOG**

If `CHANGELOG.md` does not exist, create it with:

```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — Sprint 1

- `cogito-protocol::event::ConversationEvent` with `schema_version: u32` and
  9-variant `EventPayload`. Adjacent-tag flattened envelope. `SCHEMA_VERSION = 1`.
- `cogito-protocol::store::ConversationStore` trait (`append`, `flush`, `close`,
  `latest_seq`, `replay`) + `StoreError`.
- `cogito-protocol::ids::{EventId, SessionId, TurnId}` ULID newtypes.
- `cogito-protocol::content::ContentBlock` (Text / ToolUse / ToolResult).
- `cogito-protocol::session::SessionMeta`.
- `cogito-store-jsonl` dev/debug-grade backend (one file per session,
  userspace flush only).
- `cogito-core::harness::step_recorder::StepRecorder` with content_block-
  boundary text batching.
- `cogito-test-fixtures::store_contract::run_store_contract` shared
  contract test suite.
- `cogito-test-fixtures::fixtures::canonical_sample_session` + checked-in
  `sample-v1.jsonl` fixture covering all 9 event variants.
- `cogito-gen-schema` internal tool + `docs/schemas/conversation-event-v1.json`
  artifact + CI drift gate.
- ADR-0007 (Event log as cross-language storage contract).
- `AGENTS.md` §2 text-delta lifecycle rewrite; new §7 `ConversationStore`
  scope rule.
- JSONL v1 spec at `docs/data-model/jsonl-v1.md`.
- H02 component doc: "Text block lifecycle" section.
- `append_throughput` criterion benchmark + `docs/quality/v0.1-jsonl-baseline.md`
  informational baseline.

### Compatibility

- `ConversationEvent` schema_version = 1; stable for the 0.x line.
  Future breaking changes will bump the version and ship a migration tool
  per ADR-0005 §4 #2.
- `ConversationStore` trait shape is stable for v0.1. 0.x breaking changes
  permitted with `CHANGELOG.md` entry; v1.0 freezes per ADR-0005 §5.
```

If `CHANGELOG.md` already exists, prepend the new Sprint 1 entries under `## [Unreleased]`.

- [ ] **Step 4: Run full CI gate locally**

```bash
just ci && just gen-schema-check
```

Expected: all checks pass.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml ROADMAP.md CHANGELOG.md
git commit -m "Wire schema-check CI gate + ROADMAP/CHANGELOG for Sprint 1"
```

---

## Task 18: Final sanity sweep + PR

**Files:**
- (none — verification + PR open)

- [ ] **Step 1: Full clean build + tests**

```bash
just fmt
just fix
just test
just gen-schema-check
```

Expected: all green. If anything fails, fix and amend the relevant prior task's commit (or add a small follow-up commit).

- [ ] **Step 2: Confirm cargo-deny still passes**

```bash
cargo deny check
```

Expected: `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 3: Verify file inventory**

Run: `git diff --stat main..HEAD`

Cross-check that the following new files exist:

- `crates/cogito-protocol/src/ids.rs`
- `crates/cogito-protocol/src/content.rs`
- `crates/cogito-protocol/src/session.rs`
- `crates/cogito-protocol/src/event.rs`
- `crates/cogito-protocol/src/store.rs`
- `crates/cogito-store-jsonl/src/lib.rs` (rewritten)
- `crates/cogito-store-jsonl/tests/contract.rs`
- `crates/cogito-store-jsonl/benches/append_throughput.rs`
- `crates/cogito-core/src/harness/step_recorder.rs`
- `testing/cogito-test-fixtures/src/store_contract.rs`
- `testing/cogito-test-fixtures/src/fixtures.rs`
- `testing/cogito-test-fixtures/src/bin/write_sample.rs`
- `testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
- `testing/cogito-test-fixtures/tests/fixture_roundtrip.rs`
- `tools/cogito-gen-schema/Cargo.toml`
- `tools/cogito-gen-schema/src/main.rs`
- `docs/schemas/conversation-event-v1.json`
- `docs/adr/0007-event-log-as-cross-language-contract.md`
- `docs/data-model/jsonl-v1.md`
- `docs/quality/v0.1-jsonl-baseline.md`
- `CHANGELOG.md`

And modifications:

- `Cargo.toml` (deps + tools/ member)
- `crates/cogito-protocol/Cargo.toml`
- `crates/cogito-protocol/src/lib.rs`
- `crates/cogito-store-jsonl/Cargo.toml`
- `crates/cogito-core/Cargo.toml`
- `crates/cogito-core/src/harness/mod.rs`
- `testing/cogito-test-fixtures/Cargo.toml`
- `testing/cogito-test-fixtures/src/lib.rs`
- `justfile`
- `.github/workflows/ci.yml`
- `AGENTS.md`
- `ROADMAP.md`
- `docs/adr/0005-production-scope-and-quality-gates.md`
- `docs/adr/README.md`
- `docs/components/H02-step-recorder.md`

- [ ] **Step 4: Push + open PR**

```bash
git push -u github HEAD

gh pr create --title "Sprint 1: H02 + ConversationStore + JSONL backend + ADR-0007" \
  --body "$(cat <<'EOF'
## Summary

- Implements v0.1 Sprint 1 per spec
  `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`.
- New cogito-protocol types: `ConversationEvent`, `EventPayload` (9 variants),
  `ConversationStore` trait, `EventId/SessionId/TurnId`, `ContentBlock`,
  `SessionMeta`, `StoreError`.
- Dev/debug-grade `cogito-store-jsonl` implementation + contract test.
- H02 `StepRecorder` with content_block-boundary text batching (matches
  Codex / Claude Code).
- ADR-0007: event log is the cross-language storage contract; SaaS catalog
  reads do not go through a Rust trait.
- Tooling: `cogito-gen-schema` produces `docs/schemas/conversation-event-v1.json`
  with CI drift gate. Canonical `sample-v1.jsonl` fixture.
- Informational `append_throughput` baseline at `docs/quality/v0.1-jsonl-baseline.md`.
- `AGENTS.md` amendments: §2 (text-delta lifecycle), §7 (ConversationStore scope).

## Test plan

- [ ] `just ci` green (fmt + clippy + layer-check + test)
- [ ] `cargo deny check` green
- [ ] `just gen-schema-check` green
- [ ] `cargo test -p cogito-store-jsonl --test contract` runs all 8 contract sub-tests
- [ ] `cargo test -p cogito-test-fixtures --test fixture_roundtrip` locks the fixture
- [ ] `cargo test -p cogito-core --lib harness::step_recorder::tests` covers
      text-block lifecycle + monotonic seq
- [ ] Manual sanity: `head -1 testing/cogito-test-fixtures/fixtures/sessions/sample-v1.jsonl`
      starts with `{"schema_version":1,...,"type":"session_started",...}`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL printed. Capture it for the conversation log.

- [ ] **Step 5: No commit (PR creation only)**

---

## Out of scope (locked for clarity)

This plan delivers Sprint 1 only. The following items are deliberately
excluded; do not slip them in:

- Real H01 Turn Driver FSM body — Sprint 2.
- Real H06 Stream Demultiplexer — Sprint 2.
- Real H08 Tool Dispatcher — Sprint 2.
- Anthropic / OpenAI model adapters — Sprint 2 / 5.
- H03 Resume Coordinator — Sprint 3 (consumes the trait, no extension needed).
- Async jobs — Sprint 4.
- `cogito-store-postgres` — v0.4.
- `ConversationCatalog` Rust trait — deferred per ADR-0007.
- `TenantContext` enforcement — v0.4 (ADR-0012).

## Cross-references

- Spec: `docs/superpowers/specs/2026-05-18-h02-conversation-store-and-event-log.md`
- ADR-0005 §3 (SLO), §4 #2 (schema_version), §5 (compatibility)
- ADR-0006 (Runtime + H01 execution model)
- ADR-0007 (Event log as cross-language storage contract — written under this plan)
- AGENTS.md §"Inviolable design principles"
- ROADMAP §"Sprint 1 · H02 Step Recorder + JSONL store"
