//! Trivial fixture crate used by the run_tests integration test.

#[cfg(test)]
mod tests {
    #[test]
    fn echo_passes() {
        assert_eq!(1 + 1, 2);
    }
}
