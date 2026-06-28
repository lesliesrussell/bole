// bole-mhs
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    fn matches(pat: &[u8], s: &[u8]) -> bool {
        match (pat, s) {
            ([], []) => true,
            ([], _) => false,
            // bole-l54
            ([b'*', b'*', b'/', rest @ ..], _) => {
                // `**/` matches zero or more leading path segments before `rest`,
                // so `a/**/z` matches `a/z`, `a/x/z`, `a/x/y/z`.
                if matches(rest, s) { return true; }
                for i in 0..s.len() {
                    if s[i] == b'/' && matches(pat, &s[i + 1..]) { return true; }
                }
                false
            }
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

    // bole-l54
    #[test]
    fn single_star_matches_one_mid_segment() {
        assert!(glob_matches("src/*/mod.rs", "src/a/mod.rs"));
        // a single * spans exactly one segment, not two
        assert!(!glob_matches("src/*/mod.rs", "src/a/b/mod.rs"));
        assert!(!glob_matches("src/*/mod.rs", "src/mod.rs"));
    }

    // bole-l54
    #[test]
    fn single_star_partial_segment_does_not_span_separator() {
        assert!(glob_matches("src/*.rs", "src/a.rs"));
        assert!(!glob_matches("src/*.rs", "src/a/b.rs"));
    }

    // bole-l54
    #[test]
    fn single_star_matches_zero_chars() {
        assert!(glob_matches("a*", "a"));
        assert!(glob_matches("a*b", "ab"));
        assert!(glob_matches("*", ""));
    }

    // bole-l54
    #[test]
    fn double_star_in_middle_matches_any_depth() {
        // zero, one, and many intermediate segments
        assert!(glob_matches("a/**/z", "a/z"));
        assert!(glob_matches("a/**/z", "a/b/z"));
        assert!(glob_matches("a/**/z", "a/b/c/z"));
        assert!(!glob_matches("a/**/z", "a/b/c"));
        assert!(!glob_matches("a/**/z", "x/b/z"));
    }

    // bole-l54
    #[test]
    fn double_star_prefix_matches_any_depth() {
        assert!(glob_matches("**/z", "z"));
        assert!(glob_matches("**/z", "a/z"));
        assert!(glob_matches("**/z", "a/b/z"));
        assert!(!glob_matches("**/z", "z/a"));
    }

    // bole-l54
    #[test]
    fn double_star_trailing_requires_descendant() {
        // current semantics: `src/**` covers descendants, not the bare prefix
        assert!(glob_matches("src/**", "src/a"));
        assert!(glob_matches("src/**", "src/a/b/c"));
        assert!(!glob_matches("src/**", "src"));
        assert!(!glob_matches("src/**", "srcfile"));
    }

    // bole-l54
    #[test]
    fn matching_is_case_sensitive() {
        assert!(!glob_matches("Src/**", "src/x"));
        assert!(!glob_matches("README.md", "readme.md"));
    }

    // bole-l54
    #[test]
    fn empty_pattern_only_matches_empty_path() {
        assert!(glob_matches("", ""));
        assert!(!glob_matches("", "a"));
        assert!(glob_matches("**", ""));
    }

    // bole-l54
    #[test]
    fn literal_prefix_is_not_a_partial_match() {
        assert!(!glob_matches("secret", "secrets"));
        assert!(!glob_matches("secrets/key", "secrets/key2"));
    }
}
