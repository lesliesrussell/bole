// bole-mhs
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    fn matches(pat: &[u8], s: &[u8]) -> bool {
        match (pat, s) {
            ([], []) => true,
            ([], _) => false,
            ([b'*', b'*', rest @ ..], _) => {
                // ** matches zero or more path segments
                if matches(rest, s) { return true; }
                for i in 0..=s.len() {
                    if i == s.len() || s[i] == b'/' {
                        let tail = if i == s.len() { &s[i..] } else { &s[i + 1..] };
                        if matches(rest, tail) { return true; }
                    }
                }
                false
            }
            ([b'*', rest @ ..], _) => {
                // * matches any sequence of non-separator chars
                let mut i = 0;
                loop {
                    if matches(rest, &s[i..]) { return true; }
                    if i == s.len() || s[i] == b'/' { return false; }
                    i += 1;
                }
            }
            ([p, pat_rest @ ..], [c, s_rest @ ..]) if p == c => matches(pat_rest, s_rest),
            _ => false,
        }
    }
    matches(pattern.as_bytes(), path.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::glob_matches;

    #[test]
    fn double_star_matches_nested() {
        assert!(glob_matches("secrets/**", "secrets/prod.key"));
        assert!(glob_matches("secrets/**", "secrets/a/b/c"));
        assert!(!glob_matches("secrets/**", "src/main.rs"));
    }

    #[test]
    fn double_star_matches_direct_child() {
        assert!(glob_matches("src/**", "src/main.rs"));
    }

    #[test]
    fn single_star_does_not_span_separator() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(!glob_matches("*.rs", "src/main.rs"));
    }

    #[test]
    fn exact_match() {
        assert!(glob_matches("README.md", "README.md"));
        assert!(!glob_matches("README.md", "readme.md"));
    }

    #[test]
    fn no_pattern_chars_literal() {
        assert!(glob_matches("src/lib.rs", "src/lib.rs"));
        assert!(!glob_matches("src/lib.rs", "src/main.rs"));
    }

    #[test]
    fn star_star_at_root() {
        assert!(glob_matches("**", "anything/nested/deeply"));
        assert!(glob_matches("**", "flat"));
    }
}
