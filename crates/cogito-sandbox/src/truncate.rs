//! Head+tail byte-budget truncation, UTF-8 safe. Extracted to match the
//! behavior already used by `cogito-jobs::run_tests` so both eventually
//! share one implementation.

/// Truncate `s` so the head and tail together stay within `2 * max` bytes,
/// joined by an elision marker. Returns `(text, truncated)`.
pub fn head_tail(s: &str, max: usize) -> (String, bool) {
    let bytes = s.as_bytes();
    if bytes.len() <= max.saturating_mul(2) {
        return (s.to_string(), false);
    }
    let head_end = floor_char_boundary(s, max);
    let tail_start = ceil_char_boundary(s, bytes.len() - max);
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let elided = bytes.len() - head.len() - tail.len();
    (
        format!("{head}\n... [{elided} bytes elided] ...\n{tail}"),
        true,
    )
}

/// Round `idx` down to the nearest UTF-8 char boundary in `s`.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `idx` up to the nearest UTF-8 char boundary in `s`.
fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn within_budget_passes_through() {
        let (out, trunc) = head_tail("short", 100);
        assert_eq!(out, "short");
        assert!(!trunc);
    }

    #[test]
    fn over_budget_is_elided() {
        let s = "a".repeat(1000);
        let (out, trunc) = head_tail(&s, 10);
        assert!(trunc);
        assert!(out.contains("bytes elided"));
        assert!(out.len() < s.len());
    }
}
