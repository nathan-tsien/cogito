//! Sigil scanner — moved to `cogito_protocol::sigil` so the Brain layer
//! (H06) can use it without crossing ADR-0004. Re-exported here for
//! backward compatibility with existing call sites and tests.

pub use cogito_protocol::sigil::{FenceState, SigilHit, find_sigils_outside_code};
