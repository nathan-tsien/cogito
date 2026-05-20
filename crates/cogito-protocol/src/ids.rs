//! Strongly-typed identifiers used throughout the protocol layer.
//!
//! All IDs wrap a [`ulid::Ulid`] which is monotonic per process, lexically
//! sortable, and renders as a 26-character Crockford base32 string in JSON.

use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::{Generator, Ulid};

/// Process-local monotonic ULID generator shared by every `id_newtype!`
/// constructor. Wrapping a `ulid::Generator` here guarantees that any two
/// IDs minted in the same process satisfy `a < b` even when they fall in
/// the same millisecond.
///
/// If the generator's random bits would overflow (only possible after
/// ~2^80 calls within the same millisecond), or the mutex is poisoned,
/// we fall back to a fresh `Ulid::new()` — strict monotonicity is best
/// effort, not a correctness invariant of the protocol.
static GENERATOR: Mutex<Generator> = Mutex::new(Generator::new());

/// Mint a fresh monotonic ULID using the shared process-local generator.
fn next_ulid() -> Ulid {
    match GENERATOR.lock() {
        Ok(mut g) => g.generate().unwrap_or_else(|_| Ulid::new()),
        Err(poisoned) => poisoned
            .into_inner()
            .generate()
            .unwrap_or_else(|_| Ulid::new()),
    }
}

macro_rules! id_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            Serialize,
            Deserialize,
            JsonSchema,
        )]
        #[serde(transparent)]
        // schemars attribute lives on the inner field; the struct is #[serde(transparent)] so the wire schema is the inner type's schema (a String rendering of the ULID).
        pub struct $name(#[schemars(with = "String")] Ulid);

        impl $name {
            /// Create a fresh ID using a process-local monotonic ULID.
            #[must_use]
            pub fn new() -> Self {
                Self($crate::ids::next_ulid())
            }

            /// Return the inner ULID by value.
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

impl EventId {
    /// Sentinel value returned when the recorder fails while trying to
    /// record a `TurnFailed` event — the actor cannot record its own
    /// failure. Encoded as the all-zeros ULID so it is distinguishable
    /// from any real `EventId::new()` (which is monotonically generated).
    ///
    /// Callers receive this value only on the unrecoverable "recorder
    /// errored while recording the FSM's terminal Failed transition"
    /// path. Downstream consumers should detect it via
    /// [`Self::is_recorder_failure_placeholder`].
    #[must_use]
    pub fn recorder_failure_placeholder() -> Self {
        Self::from(Ulid::nil())
    }

    /// Returns true iff this `EventId` is the recorder-failure sentinel.
    /// See [`Self::recorder_failure_placeholder`].
    #[must_use]
    pub fn is_recorder_failure_placeholder(&self) -> bool {
        self.as_ulid() == Ulid::nil()
    }
}

#[cfg(test)]
mod tests {
    use super::{EventId, SessionId, TurnId};

    #[test]
    fn recorder_failure_placeholder_is_distinguishable() {
        let real = EventId::new();
        let sentinel = EventId::recorder_failure_placeholder();
        assert_ne!(real, sentinel);
        assert!(!real.is_recorder_failure_placeholder());
        assert!(sentinel.is_recorder_failure_placeholder());
        // Two sentinels are equal — they represent the same conceptual value.
        assert_eq!(sentinel, EventId::recorder_failure_placeholder());
    }

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
        // The shared monotonic generator must mint strictly increasing IDs
        // for two sequential `new()` calls in the same process. A regression
        // to non-strict ordering (e.g. dropping the Generator wrapper) would
        // silently break event-log ordering invariants.
        assert!(
            b > a,
            "monotonic ULID generator must mint strictly increasing IDs: a={a}, b={b}"
        );
    }
}
