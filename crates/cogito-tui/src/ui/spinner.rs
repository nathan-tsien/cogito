//! Braille spinner — animates running tools and the
//! between-content thinking marker (spec §"Spinner animation source").
//!
//! Frames cycle through the conventional 10-frame braille sequence.
//! Index by `tick / period`, where `tick` is the redraw counter and
//! `period` is chosen so visible frame rate is ~10 Hz at the 33ms
//! redraw cadence (period = 3 ticks ≈ 99ms per frame).

/// One frame per advance; advance once every `PERIOD_TICKS` redraw
/// ticks. Lower = faster (busier); higher = calmer.
pub const PERIOD_TICKS: u64 = 3;

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Return the spinner glyph for the given redraw tick.
#[must_use]
pub fn frame(tick: u64) -> &'static str {
    let len = u64::try_from(FRAMES.len()).unwrap_or(1);
    let idx = (tick / PERIOD_TICKS) % len;
    FRAMES[usize::try_from(idx).unwrap_or(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_zero_is_first_frame() {
        assert_eq!(frame(0), "⠋");
    }

    #[test]
    fn frame_advances_every_period_ticks() {
        assert_eq!(frame(0), frame(PERIOD_TICKS - 1));
        assert_ne!(frame(0), frame(PERIOD_TICKS));
        assert_eq!(frame(PERIOD_TICKS), "⠙");
    }

    #[test]
    fn frame_cycles_through_all_braille_frames() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..(PERIOD_TICKS * 10) {
            seen.insert(frame(i));
        }
        assert_eq!(seen.len(), 10);
    }
}
