//! `CommandGuardHook` — accident guard that blocks catastrophic shell
//! commands before the `bash` tool dispatches them.
//!
//! This is an ACCIDENT GUARD for a trusted local operator (the TUI), NOT a
//! security boundary. It is intentionally a denylist (not an allowlist), is
//! trivially bypassable (string encoding, alternate tools, environment
//! indirection, etc.), and exists solely to stop fat-finger mistakes and
//! model errors -- not adversaries. True isolation for untrusted or
//! multi-tenant execution is deferred to ADR-0012 (sandbox) and ADR-0013
//! (Credential Broker); this guard merely complements the H09 admission
//! seam reserved in ADR-0027. See ADR-0037 for the rationale and scope.

#![allow(clippy::expect_used)] // gated to LazyLock regex constants below

use std::sync::LazyLock;

use cogito_protocol::hook::{HookDecision, HookHandler};
use regex::Regex;

/// Stable hook identifier used in events and rejection reasons.
const HOOK_NAME: &str = "command-guard";

// --- Denylist patterns. One rule each, conservative and high-signal. ---

// Rule 1: `recursive-force-rm-on-system-path`.
// Matches `rm` invoked with both recursive and force flags (in any of the
// common spellings) whose target is a catastrophic root location: `/`,
// `/*`, `~`, `$HOME`/`${HOME}`, or a top-level system directory. Relative
// or project-local targets (`./build`, `target`, `node_modules`) are NOT
// matched on purpose.
static RM_RF_SYSTEM_PATH: LazyLock<Regex> = LazyLock::new(|| {
    // Strategy: anchor on `rm`, then require BOTH a recursive flag and a
    // force flag (in any order / spelling) appearing as flag tokens before
    // the target, then require the target token to be a catastrophic root
    // location. Flags and target are matched as whitespace-delimited tokens
    // so project-local targets (`./build`, `target`) never match.
    //
    // The recursive+force requirement is expressed as two lookahead-free
    // ordered alternations: we accept either `rf-bundle then target`,
    // `r ... f ... target`, or `f ... r ... target`. To keep this readable
    // and avoid catastrophic backtracking, we match a leading run of flag
    // tokens, assert it contains recursive AND force via two embedded
    // patterns, then match the target. Rust's `regex` lacks lookahead, so
    // we enumerate the flag combinations explicitly.
    Regex::new(
        r"(?x)
        \brm\b
        (?:\s+\S+)*?                                   # optional intervening flag tokens (e.g. sudo already handled by \brm\b anchor)
        \s+
        (?:                                            # recursive + force, any order
            -[a-zA-Z]*r[a-zA-Z]*f[a-zA-Z]*             #   bundled -rf / -frf etc (r before f)
          | -[a-zA-Z]*f[a-zA-Z]*r[a-zA-Z]*             #   bundled -fr (f before r)
          | -[a-zA-Z]*r[a-zA-Z]*\s+-[a-zA-Z]*f[a-zA-Z]*  # -r -f
          | -[a-zA-Z]*f[a-zA-Z]*\s+-[a-zA-Z]*r[a-zA-Z]*  # -f -r
          | --recursive\s+--force
          | --force\s+--recursive
        )
        (?:\s+(?:-{1,2}\S+))*                          # any further flags before the target
        \s+
        (?:                                            # catastrophic target token
            /\*?                                       #   / or /*
          | ~                                          #   home shorthand
          | \$\{?HOME\}?                               #   $HOME or ${HOME}
          | /(?:etc|usr|bin|var|boot|lib|sys|proc|dev|root|home)(?:/\S*)?
        )
        (?:\s|$|;|&|\|)                                # boundary after target
        ",
    )
    .expect("rm -rf system-path regex compiles")
});

// Rule 2: `fork-bomb`. The classic `:(){ :|:& };:` and whitespace variants.
static FORK_BOMB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r":\s*\(\s*\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:").expect("fork-bomb regex compiles")
});

// Rule 3: `mkfs`. Any `mkfs` or `mkfs.<type>` invocation.
static MKFS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bmkfs(?:\.\w+)?\b").expect("mkfs regex compiles"));

// Rule 4: `dd-to-block-device`. A `dd` command writing `of=` a raw block
// device (sd/nvme/hd/vd) or disk/mapper node.
static DD_TO_DEVICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bdd\b[^\n]*\bof=/dev/(?:sd|nvme|hd|vd|disk|mapper)")
        .expect("dd-to-device regex compiles")
});

// Rule 5: `redirect-to-block-device`. A shell redirect (`>`) writing a raw
// block device node (sd/nvme/hd/vd).
static REDIRECT_TO_DEVICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r">\s*/dev/(?:sd|nvme|hd|vd)").expect("redirect-to-device regex compiles")
});

// Rule 6: `chmod-recursive-root`. `chmod -R` (or `--recursive`) applied to
// `/` or a top-level system directory.
static CHMOD_RECURSIVE_ROOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        \bchmod\b
        (?:\s+\S+)*?                            # mode and other args, lazily
        \s+(?:-[a-zA-Z]*R[a-zA-Z]*|--recursive) # recursive flag
        (?:\s+\S+)*?
        \s+
        (?:
            /\*?
          | /(?:etc|usr|bin|var|boot|lib|sys|proc|dev|root|home)(?:/\S*)?\*?
        )
        (?:\s|$|;|&|\|)
        ",
    )
    .expect("chmod recursive-root regex compiles")
});

/// Hook that blocks a curated denylist of catastrophic shell commands.
#[derive(Debug, Default)]
pub struct CommandGuardHook;

impl CommandGuardHook {
    /// Creates a new `CommandGuardHook`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Returns the rule label matched by `cmd`, or `None` if `cmd` is
    /// allowed. Each label corresponds to one denylist regex.
    fn check(cmd: &str) -> Option<&'static str> {
        if RM_RF_SYSTEM_PATH.is_match(cmd) {
            return Some("recursive-force-rm-on-system-path");
        }
        if FORK_BOMB.is_match(cmd) {
            return Some("fork-bomb");
        }
        if MKFS.is_match(cmd) {
            return Some("mkfs");
        }
        if DD_TO_DEVICE.is_match(cmd) {
            return Some("dd-to-block-device");
        }
        if REDIRECT_TO_DEVICE.is_match(cmd) {
            return Some("redirect-to-block-device");
        }
        if CHMOD_RECURSIVE_ROOT.is_match(cmd) {
            return Some("chmod-recursive-root");
        }
        None
    }
}

impl HookHandler for CommandGuardHook {
    fn name(&self) -> &str {
        HOOK_NAME
    }

    fn pre_dispatch(
        &self,
        _call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> HookDecision {
        // Only the bash tool runs shell commands; everything else is out of
        // scope for this guard.
        if tool_name != "bash" {
            return HookDecision::Allow;
        }
        // Extract the `command` string (confirmed field name in
        // cogito-jobs/src/bash.rs). Absent or non-string -> nothing to guard.
        let Some(cmd) = args.get("command").and_then(serde_json::Value::as_str) else {
            return HookDecision::Allow;
        };
        match Self::check(cmd) {
            Some(rule) => HookDecision::Reject {
                hook_name: HOOK_NAME.into(),
                reason: format!("blocked by command-guard: {rule}"),
            },
            None => HookDecision::Allow,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use cogito_protocol::hook::{HookDecision, HookHandler};
    use serde_json::json;

    use super::*;

    fn decide(cmd: &str) -> HookDecision {
        let h = CommandGuardHook::new();
        h.pre_dispatch("c1", "bash", &json!({ "command": cmd }))
    }

    fn assert_reject(cmd: &str, rule: &str) {
        match decide(cmd) {
            HookDecision::Reject { hook_name, reason } => {
                assert_eq!(hook_name, "command-guard");
                assert!(reason.contains(rule), "reason `{reason}` missing `{rule}`");
            }
            other => panic!("expected Reject for `{cmd}`, got {other:?}"),
        }
    }

    fn assert_allow(cmd: &str) {
        assert!(
            matches!(decide(cmd), HookDecision::Allow),
            "expected Allow for `{cmd}`"
        );
    }

    #[test]
    fn rejects_rm_rf_root() {
        assert_reject("rm -rf /", "recursive-force-rm-on-system-path");
    }

    #[test]
    fn rejects_rm_rf_glob_root() {
        assert_reject("rm -rf /*", "recursive-force-rm-on-system-path");
    }

    #[test]
    fn rejects_rm_rf_home() {
        assert_reject("rm -rf ~", "recursive-force-rm-on-system-path");
    }

    #[test]
    fn rejects_rm_rf_system_dir() {
        assert_reject("sudo rm -rf /etc", "recursive-force-rm-on-system-path");
    }

    #[test]
    fn rejects_fork_bomb() {
        assert_reject(":(){ :|:& };:", "fork-bomb");
    }

    #[test]
    fn rejects_mkfs() {
        assert_reject("mkfs.ext4 /dev/sda1", "mkfs");
    }

    #[test]
    fn rejects_dd_to_device() {
        assert_reject("dd if=/dev/zero of=/dev/sda bs=1M", "dd-to-block-device");
    }

    #[test]
    fn rejects_redirect_to_device() {
        assert_reject("echo x > /dev/sda", "redirect-to-block-device");
    }

    #[test]
    fn allows_local_rm() {
        assert_allow("rm -rf ./build");
        assert_allow("rm -rf target");
        assert_allow("rm -rf node_modules");
    }

    #[test]
    fn allows_safe_commands() {
        assert_allow("ls -la");
        assert_allow("cargo test");
        assert_allow("git status");
    }

    #[test]
    fn allows_non_bash_tool() {
        let h = CommandGuardHook::new();
        let dec = h.pre_dispatch("c1", "read_file", &json!({ "command": "rm -rf /" }));
        assert!(matches!(dec, HookDecision::Allow));
    }

    #[test]
    fn allows_when_no_command_field() {
        let h = CommandGuardHook::new();
        let dec = h.pre_dispatch("c1", "bash", &json!({}));
        assert!(matches!(dec, HookDecision::Allow));
    }
}
