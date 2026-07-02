// bole-1hu
/// Maximum accepted pattern or path length. Both come from policy/data and are a
/// few segments in practice; anything past this is not a legitimate glob and is
/// rejected fail-closed, bounding even the memoized matcher's worst case.
const MAX_GLOB_LEN: usize = 8192;

// bole-mhs
/// Glob matcher. The recursion is **memoized** on `(pat_len, s_len)` — every
/// recursive call passes a *suffix* of the original pattern and path, so their
/// lengths uniquely identify the (pat_offset, s_offset) state. Without the memo
/// this is a naive backtracker with catastrophic (exponential) blowup on inputs
/// containing several `**` groups against a non-matching path (a ReDoS/CPU DoS
/// on attacker-influenced patterns/paths); the memo makes it polynomial while
/// preserving the exact matching semantics. See bole-1hu (audit #8).
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat = pattern.as_bytes();
    let s = path.as_bytes();
    // bole-1hu: reject absurdly long inputs before matching.
    if pat.len() > MAX_GLOB_LEN || s.len() > MAX_GLOB_LEN {
        return false;
    }

    // bole-1hu: memo of failing (pat suffix len, s suffix len) states. All
    // recursive calls pass suffixes of `pat`/`s`, so the pair of lengths is a
    // sound state key. Caching failures collapses the exponential re-exploration.
    use std::collections::HashSet;
    fn matches(pat: &[u8], s: &[u8], false_memo: &mut HashSet<(usize, usize)>) -> bool {
        let key = (pat.len(), s.len());
        if false_memo.contains(&key) {
            return false;
        }
        let result = match (pat, s) {
            ([], []) => true,
            ([], _) => false,
            // bole-l54
            ([b'*', b'*', b'/', rest @ ..], _) => {
                // `**/` matches zero or more leading path segments before `rest`,
                // so `a/**/z` matches `a/z`, `a/x/z`, `a/x/y/z`.
                if matches(rest, s, false_memo) {
                    true
                } else {
                    let mut hit = false;
                    for i in 0..s.len() {
                        if s[i] == b'/' && matches(pat, &s[i + 1..], false_memo) {
                            hit = true;
                            break;
                        }
                    }
                    hit
                }
            }
            ([b'*', b'*', rest @ ..], _) => {
                // ** matches zero or more path segments
                if matches(rest, s, false_memo) {
                    true
                } else {
                    let mut hit = false;
                    for i in 0..=s.len() {
                        if i == s.len() || s[i] == b'/' {
                            let tail = if i == s.len() { &s[i..] } else { &s[i + 1..] };
                            if matches(rest, tail, false_memo) {
                                hit = true;
                                break;
                            }
                        }
                    }
                    hit
                }
            }
            ([b'*', rest @ ..], _) => {
                // * matches any sequence of non-separator chars
                let mut i = 0;
                loop {
                    if matches(rest, &s[i..], false_memo) {
                        break true;
                    }
                    if i == s.len() || s[i] == b'/' {
                        break false;
                    }
                    i += 1;
                }
            }
            ([p, pat_rest @ ..], [c, s_rest @ ..]) if p == c => {
                matches(pat_rest, s_rest, false_memo)
            }
            _ => false,
        };
        if !result {
            false_memo.insert(key);
        }
        result
    }

    let mut false_memo = HashSet::new();
    matches(pat, s, &mut false_memo)
}

#[cfg(test)]
mod tests {
    use super::{glob_matches, MAX_GLOB_LEN};

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

    // bole-1hu
    #[test]
    fn many_double_stars_against_nonmatch_is_fast() {
        // Eight `**/` groups against a 30-segment path that cannot match (no
        // trailing `x`). The naive backtracker explores billions of states here
        // and effectively hangs; memoized, it returns quickly. If this test ever
        // times out, the ReDoS guard has regressed.
        let pattern = format!("{}x", "**/".repeat(8));
        let path = vec!["a"; 30].join("/");
        assert!(!glob_matches(&pattern, &path));
    }

    // bole-1hu
    #[test]
    fn memoization_preserves_matching_semantics() {
        // The memoized matcher must agree with the documented semantics on the
        // tricky multi-`**` cases.
        assert!(glob_matches("**/**/z", "a/b/c/z"));
        assert!(glob_matches("a/**/**/z", "a/z"));
        assert!(!glob_matches("**/**/z", "a/b/c/y"));
    }

    // bole-1hu
    #[test]
    fn over_length_inputs_fail_closed() {
        let long = "a".repeat(MAX_GLOB_LEN + 1);
        assert!(!glob_matches(&long, "a"));
        assert!(!glob_matches("**", &long));
    }
}
