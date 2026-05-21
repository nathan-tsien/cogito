//! `${VAR}` and `${VAR:-default}` substitution over `toml::Value`
//! trees. See ADR-0017 §6.
//!
//! Only `String` leaves are touched; numbers, booleans, datetimes etc.
//! pass through unchanged. A reference of the form `${NAME}` requires
//! `NAME` to be set and non-empty in the process environment; the
//! `${NAME:-fallback}` form supplies a literal default to use when the
//! variable is unset or empty. Lone `$` (not followed by `{`) and
//! unclosed `${` are passed through verbatim.

use crate::loader::ConfigError;

/// Walk a `toml::Value` and interpolate every string in place. Numbers,
/// booleans, dates, etc. are returned unchanged.
///
/// # Errors
///
/// Returns [`ConfigError::MissingEnv`] when a `${VAR}` reference
/// without a `:-default` fallback names an environment variable that
/// is unset or empty.
pub fn interpolate_value(value: toml::Value) -> Result<toml::Value, ConfigError> {
    match value {
        toml::Value::String(s) => Ok(toml::Value::String(interpolate_str(&s)?)),
        toml::Value::Array(items) => {
            let out: Result<Vec<_>, _> = items.into_iter().map(interpolate_value).collect();
            Ok(toml::Value::Array(out?))
        }
        toml::Value::Table(t) => {
            let mut out = toml::map::Map::new();
            for (k, v) in t {
                out.insert(k, interpolate_value(v)?);
            }
            Ok(toml::Value::Table(out))
        }
        other => Ok(other),
    }
}

fn interpolate_str(s: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        // Only inspect bytes at byte index `i` when we know it is a
        // char boundary (it always is here: we either advance by the
        // full UTF-8 length of the current char, or by 2/end+1 past
        // an ASCII `${...}` marker — both of which land on boundaries).
        let rest = &s[i..];
        if rest.starts_with("${") {
            let start = i + 2;
            let Some(end_rel) = s[start..].find('}') else {
                // Unclosed `${`; treat the `$` as a literal and resume
                // scanning at the next byte.
                out.push('$');
                i += 1;
                continue;
            };
            let end = start + end_rel;
            let body = &s[start..end];
            let (var, default) = match body.find(":-") {
                Some(p) => (&body[..p], Some(&body[p + 2..])),
                None => (body, None),
            };
            let resolved = match std::env::var(var) {
                Ok(v) if !v.is_empty() => v,
                _ => match default {
                    Some(d) => d.to_string(),
                    None => return Err(ConfigError::MissingEnv(var.to_string())),
                },
            };
            out.push_str(&resolved);
            i = end + 1;
        } else {
            // UTF-8 safe: advance by the byte-length of the current
            // char rather than casting the raw byte to `char`.
            let c = rest.chars().next().unwrap_or('\0');
            if c == '\0' {
                // `rest` was empty; bail out of the loop. (Defensive:
                // the `while` guard already prevents this branch.)
                break;
            }
            out.push(c);
            i += c.len_utf8();
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn empty_var_uses_default() {
        // temp-env scopes the env mutation to this closure body.
        temp_env::with_vars([("COGITO_EMPTY_VAR", Some(""))], || {
            let s = interpolate_str("${COGITO_EMPTY_VAR:-fallback}").unwrap();
            assert_eq!(s, "fallback");
        });
    }

    #[test]
    fn unclosed_brace_passes_through() {
        let s = interpolate_str("${VAR no closing").unwrap();
        assert_eq!(s, "${VAR no closing");
    }

    #[test]
    fn utf8_literal_round_trips() {
        // Non-ASCII bytes in literal regions must survive unchanged.
        let s = interpolate_str("café — 中文").unwrap();
        assert_eq!(s, "café — 中文");
    }
}
