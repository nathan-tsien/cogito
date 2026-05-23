//! Sigil regex + streaming code-fence-aware scanner. See ADR-0020 §1
//! and spec §6.3.

use std::sync::OnceLock;

use regex::Regex;

/// Match anchored on letter; allow kebab + underscore + colon (plugin ns).
const SIGIL_PATTERN: &str = r"\$([A-Za-z][A-Za-z0-9_:-]{0,63})";

fn sigil_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // The pattern is a compile-time constant; if it fails to compile we want
    // to know about it immediately during the first call. Using `expect` here
    // is the documented pattern for `OnceLock::get_or_init` initialisation.
    #[allow(clippy::expect_used)]
    R.get_or_init(|| Regex::new(SIGIL_PATTERN).expect("sigil regex compiles"))
}

/// A sigil match in a text chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigilHit {
    /// The captured name (regex group 1).
    pub name: String,
    /// Byte offset within the supplied chunk.
    pub byte_offset: usize,
}

/// Streaming code-fence parser state. Default = `Normal`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FenceState {
    /// Outside any code construct; sigils match.
    #[default]
    Normal,
    /// Inside a triple-backtick fenced block; sigils ignored until the
    /// closing fence.
    InFenced,
    /// Inside an inline single-backtick span on the current line; sigils
    /// ignored until the closing backtick or end of line.
    InInline,
}

/// Find sigils in `chunk` while honoring `state`. State persists across calls
/// (so multi-chunk streaming works).
pub fn find_sigils_outside_code(state: &mut FenceState, chunk: &str) -> Vec<SigilHit> {
    let re = sigil_regex();
    let mut hits = Vec::new();
    let mut idx = 0usize;
    let bytes = chunk.as_bytes();

    while idx < bytes.len() {
        match *state {
            FenceState::Normal => {
                // Look for the next fence opener or sigil; whichever comes first.
                let triple = find_at_line_start(bytes, idx, b"```");
                let backtick = find_byte(bytes, idx, b'`');
                let newline = find_byte(bytes, idx, b'\n');

                // Pick the smallest of triple_open / backtick that is BEFORE the
                // next newline (inline scope is per-line).
                let next_special = pick_min([
                    triple,
                    if let (Some(bt), Some(nl)) = (backtick, newline) {
                        if bt < nl { Some(bt) } else { None }
                    } else {
                        backtick
                    },
                ]);

                let end = next_special.unwrap_or(bytes.len());
                let slice = &chunk[idx..end];
                let slice_base = idx;
                for cap in re.captures_iter(slice) {
                    // Group 0 and group 1 are guaranteed present by the
                    // regex `\$([A-Za-z][A-Za-z0-9_:-]{0,63})` whenever
                    // `captures_iter` yields a capture.
                    let Some(m) = cap.get(0) else { continue };
                    let Some(name_match) = cap.get(1) else {
                        continue;
                    };
                    hits.push(SigilHit {
                        name: name_match.as_str().to_string(),
                        byte_offset: slice_base + m.start(),
                    });
                }
                idx = end;
                if let Some(t) = triple
                    && Some(t) == next_special
                {
                    *state = FenceState::InFenced;
                    idx = t + 3;
                } else if let Some(bt) = backtick
                    && Some(bt) == next_special
                {
                    *state = FenceState::InInline;
                    idx = bt + 1;
                }
            }
            FenceState::InFenced => {
                // Skip everything until the next line-start triple-backtick.
                if let Some(close) = find_at_line_start(bytes, idx, b"```") {
                    *state = FenceState::Normal;
                    idx = close + 3;
                } else {
                    return hits;
                }
            }
            FenceState::InInline => {
                // Skip until the closing backtick or end of line.
                let backtick = find_byte(bytes, idx, b'`');
                let newline = find_byte(bytes, idx, b'\n');
                match (backtick, newline) {
                    (Some(bt), Some(nl)) if bt < nl => {
                        *state = FenceState::Normal;
                        idx = bt + 1;
                    }
                    (Some(bt), None) => {
                        *state = FenceState::Normal;
                        idx = bt + 1;
                    }
                    (_, Some(nl)) => {
                        // Inline scope ends at end of line.
                        *state = FenceState::Normal;
                        idx = nl + 1;
                    }
                    (None, None) => return hits,
                }
            }
        }
    }
    hits
}

fn find_byte(bytes: &[u8], from: usize, b: u8) -> Option<usize> {
    bytes[from..].iter().position(|&x| x == b).map(|p| from + p)
}

fn find_at_line_start(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let mut i = from;
    while i + needle.len() <= bytes.len() {
        let at_line_start = i == 0 || bytes[i - 1] == b'\n';
        if at_line_start && bytes[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn pick_min<const N: usize>(opts: [Option<usize>; N]) -> Option<usize> {
    opts.into_iter().flatten().min()
}
