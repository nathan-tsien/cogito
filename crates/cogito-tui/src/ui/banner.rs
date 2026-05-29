//! Startup banner — three `SystemNotice` lines pushed into
//! `ChatModel` at App build time. Scrolls away naturally with chat
//! history (spec §"Chrome strategy: Ambient (C3)").

/// Build the three banner lines.
///
/// Layout:
///
/// ```text
///    ∴∴∴
///    cogito  v0.2
///    <model_id>  ·  <strategy_name>  ·  <session_id[:8]>
/// ```
///
/// All lines have a 3-space leading indent so they align with the
/// chat content column.
#[must_use]
pub fn startup_lines(model_id: &str, strategy_name: &str, session_id: &str) -> Vec<String> {
    let version = env!("CARGO_PKG_VERSION");
    let short_session: String = session_id.chars().take(8).collect();
    vec![
        "   ∴∴∴".to_string(),
        format!("   cogito  v{version}"),
        format!("   {model_id}  ·  {strategy_name}  ·  {short_session}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_lines_contains_three_rows() {
        let v = startup_lines("opus-4.7", "coder", "01abcdefghij");
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn startup_lines_first_row_is_sigil() {
        let v = startup_lines("m", "s", "sid");
        assert!(v[0].contains("∴∴∴"));
    }

    #[test]
    fn startup_lines_second_row_carries_version() {
        let v = startup_lines("m", "s", "sid");
        let version = env!("CARGO_PKG_VERSION");
        assert!(v[1].contains("cogito"));
        assert!(v[1].contains(version));
    }

    #[test]
    fn startup_lines_third_row_carries_identity() {
        let v = startup_lines("opus-4.7", "coder", "01abcdefghij");
        assert!(v[2].contains("opus-4.7"));
        assert!(v[2].contains("coder"));
        assert!(v[2].contains("01abcdef"));
        assert!(!v[2].contains("01abcdefghij"));
    }
}
