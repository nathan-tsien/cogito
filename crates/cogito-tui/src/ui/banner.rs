//! Startup banner — accent (`Art`) and metadata (`Meta`) lines pushed
//! into `ChatModel` at App build time. Scrolls away naturally with chat
//! history (spec §"Chrome strategy: Ambient (C3)"). The accent lines are
//! painted bold in the cogito green; the metadata line stays dim.

/// One startup banner line plus how the chat widget should paint it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerLine {
    /// Accent line (sigil / wordmark / tagline) — bold cogito green.
    Art(String),
    /// Metadata line (version / model / strategy / session) — dim.
    Meta(String),
}

/// Build the startup banner lines.
///
/// Layout (3-space indent aligns with the chat content column):
///
/// ```text
///    ∴∴∴  cogito
///    cogito, ergo sum
///    v0.2  ·  <model_id>  ·  <strategy>  ·  <session[:8]>
/// ```
#[must_use]
pub fn startup_lines(model_id: &str, strategy_name: &str, session_id: &str) -> Vec<BannerLine> {
    let version = env!("CARGO_PKG_VERSION");
    let short_session: String = session_id.chars().take(8).collect();
    vec![
        BannerLine::Art("   \u{2234}\u{2234}\u{2234}  cogito".to_string()),
        BannerLine::Art("   cogito, ergo sum".to_string()),
        BannerLine::Meta(format!(
            "   v{version}  \u{00b7}  {model_id}  \u{00b7}  {strategy_name}  \u{00b7}  {short_session}"
        )),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn startup_lines_has_two_art_then_one_meta() {
        let v = startup_lines("opus-4.7", "coder", "01abcdefghij");
        assert_eq!(v.len(), 3);
        assert!(matches!(v[0], BannerLine::Art(_)));
        assert!(matches!(v[1], BannerLine::Art(_)));
        assert!(matches!(v[2], BannerLine::Meta(_)));
    }

    #[test]
    fn first_art_line_carries_sigil_and_wordmark() {
        let v = startup_lines("m", "s", "sid");
        match &v[0] {
            BannerLine::Art(s) => {
                assert!(s.contains("\u{2234}\u{2234}\u{2234}"));
                assert!(s.contains("cogito"));
            }
            BannerLine::Meta(_) => panic!("expected Art"),
        }
    }

    #[test]
    fn tagline_present() {
        let v = startup_lines("m", "s", "sid");
        match &v[1] {
            BannerLine::Art(s) => assert!(s.contains("cogito, ergo sum")),
            BannerLine::Meta(_) => panic!("expected Art"),
        }
    }

    #[test]
    fn meta_line_carries_version_and_identity() {
        let v = startup_lines("opus-4.7", "coder", "01abcdefghij");
        let version = env!("CARGO_PKG_VERSION");
        match &v[2] {
            BannerLine::Meta(s) => {
                assert!(s.contains(version));
                assert!(s.contains("opus-4.7"));
                assert!(s.contains("coder"));
                assert!(s.contains("01abcdef"));
                assert!(!s.contains("01abcdefghij"));
            }
            BannerLine::Art(_) => panic!("expected Meta"),
        }
    }
}
