use std::path::{Component, Path, PathBuf};

/// How well a symbol name matches the query; lower sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    Exact,
    Prefix,
    Substring,
    /// Only LuaLS' letters-bag rule (`letters_match`) accepts the name.
    Letters,
}

/// Smartcase: a query containing an uppercase letter matches case-sensitively,
/// otherwise case is ignored. An empty query matches everything. A name that
/// fails exact/prefix/substring may still be accepted by the letters-bag rule.
pub fn match_tier(text: &str, query: &str) -> Option<MatchTier> {
    if query.is_empty() {
        return Some(MatchTier::Substring);
    }
    let tier = if query.chars().any(|c| c.is_uppercase()) {
        tier_of(text, query)
    } else {
        tier_of(&text.to_lowercase(), &query.to_lowercase())
    };
    tier.or_else(|| letters_match(query, text).then_some(MatchTier::Letters))
}

/// LuaLS' `core/matchkey.lua`: the first letter of the query must start the
/// name, a word of it (after a non-letter) or a camel-case hump, and every
/// query letter must occur in the name, counted as a case-insensitive multiset.
pub fn letters_match(query: &str, text: &str) -> bool {
    let (query, text) = (query.as_bytes(), text.as_bytes());
    if query.is_empty() {
        return true;
    }
    query.len() <= text.len() && valid_first_letter(query[0], text) && letters_covered(query, text)
}

fn valid_first_letter(first: u8, text: &[u8]) -> bool {
    let first = first.to_ascii_uppercase();
    if first == text[0].to_ascii_uppercase() {
        return true;
    }
    let Some(offset) = text[1..].iter().position(|&byte| byte == first) else {
        // an uppercase letter may also start a lowercase word after a non-letter
        return first.is_ascii_uppercase()
            && text[1..].windows(2).any(|pair| {
                !pair[0].is_ascii_alphabetic() && pair[1] == first.to_ascii_lowercase()
            });
    };
    let at = offset + 1;
    !text[at].is_ascii_uppercase() || !text[at - 1].is_ascii_uppercase()
}

fn letters_covered(query: &[u8], text: &[u8]) -> bool {
    let mut needed = [0u8; 256];
    let mut remaining = 0usize;
    for byte in query {
        let slot = &mut needed[byte.to_ascii_lowercase() as usize];
        if *slot == u8::MAX {
            return false;
        }
        *slot += 1;
        remaining += 1;
    }
    for byte in text {
        let slot = &mut needed[byte.to_ascii_lowercase() as usize];
        if *slot > 0 {
            *slot -= 1;
            remaining -= 1;
            if remaining == 0 {
                return true;
            }
        }
    }
    false
}

fn tier_of(text: &str, query: &str) -> Option<MatchTier> {
    if text == query {
        Some(MatchTier::Exact)
    } else if text.starts_with(query) {
        Some(MatchTier::Prefix)
    } else if text.contains(query) {
        Some(MatchTier::Substring)
    } else {
        None
    }
}

/// Where a file sits relative to the workspace root that contains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathRank {
    /// Some directory below the root is dot-prefixed (`.upstream/`, `.git/`, `.direnv/`).
    pub hidden_dir: bool,
    /// Number of path components below the root; shallower is likely more central.
    pub depth: usize,
}

/// The deepest root containing `path` wins, so a root nested in a hidden
/// directory counts that directory as its own base. A path outside every root
/// is ranked by its full depth and never counts as hidden: without a root there
/// is no way to tell which dot-directories are the user's own.
pub fn path_rank(path: &Path, roots: &[PathBuf]) -> PathRank {
    let relative = roots
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok().map(|rest| (root, rest)))
        .max_by_key(|(root, _)| root.components().count())
        .map(|(_, rest)| rest);
    let Some(relative) = relative else {
        return PathRank {
            hidden_dir: false,
            depth: path.components().count(),
        };
    };
    let components: Vec<_> = relative.components().collect();
    let dir_count = components.len().saturating_sub(1);
    let hidden_dir = components.iter().take(dir_count).any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    });
    PathRank {
        hidden_dir,
        depth: components.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(path: &str, roots: &[&str]) -> PathRank {
        let roots: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
        path_rank(Path::new(path), &roots)
    }

    #[test]
    fn test_path_rank_table() {
        let cases = [
            ("/w/src/a.lua", vec!["/w"], false, 2),
            ("/w/a.lua", vec!["/w"], false, 1),
            ("/w/.upstream/x/a.lua", vec!["/w"], true, 3),
            ("/w/src/.hidden/b/a.lua", vec!["/w"], true, 4),
            // a dot-prefixed file name is not a hidden directory
            ("/w/src/.a.lua", vec!["/w"], false, 2),
            // outside every root: full depth, never hidden
            ("/other/.x/a.lua", vec!["/w"], false, 4),
            ("/other/a.lua", vec![], false, 3),
            // multiple roots: the deepest containing root is the base
            ("/w/.lib/pkg/a.lua", vec!["/w", "/w/.lib"], false, 2),
            ("/w/.lib/pkg/a.lua", vec!["/w/.lib", "/w"], false, 2),
            ("/w/src/a.lua", vec!["/w", "/w/.lib"], false, 2),
            ("/lib/.git/a.lua", vec!["/w", "/lib"], true, 2),
        ];
        for (path, roots, hidden_dir, depth) in cases {
            assert_eq!(
                rank(path, &roots),
                PathRank { hidden_dir, depth },
                "{path} under {roots:?}"
            );
        }
    }

    #[test]
    fn test_match_tier_table() {
        let cases = [
            ("needle", "needle", Some(MatchTier::Exact)),
            ("needle_x", "needle", Some(MatchTier::Prefix)),
            ("xneedle", "needle", Some(MatchTier::Substring)),
            ("haystack", "needle", None),
            // a lowercase query ignores case
            ("Needle", "needle", Some(MatchTier::Exact)),
            ("NEEDLE_X", "needle", Some(MatchTier::Prefix)),
            ("xNEEDLE", "needle", Some(MatchTier::Substring)),
            // an uppercase letter in the query makes it case-sensitive
            // ... except the letters-bag rule, which always ignores case
            ("needle", "Needle", Some(MatchTier::Letters)),
            ("noodle", "Needle", None),
            ("Needle", "Needle", Some(MatchTier::Exact)),
            ("NeedleX", "Needle", Some(MatchTier::Prefix)),
            ("anything", "", Some(MatchTier::Substring)),
            ("", "", Some(MatchTier::Substring)),
        ];
        for (text, query, expected) in cases {
            assert_eq!(match_tier(text, query), expected, "{text:?} vs {query:?}");
        }
    }

    #[test]
    fn test_tiers_sort_exact_prefix_substring_letters() {
        assert!(MatchTier::Exact < MatchTier::Prefix);
        assert!(MatchTier::Prefix < MatchTier::Substring);
        assert!(MatchTier::Substring < MatchTier::Letters);
    }

    #[test]
    fn test_letters_match_table() {
        let cases = [
            // first letter starts the name, the rest are in it as a multiset
            ("ran", "resolve_external_names", true),
            ("pri", "pick_new_upstream", true),
            // first letter starts a word after a non-letter
            ("sele", "client_stream", true),
            ("on_i", "get_os_config", true),
            // first letter in the middle of a word is not a start
            ("lan", "balance", false),
            ("dis", "details", true),
            // a camel-case hump counts, a capital after a capital does not
            ("bar", "fooBar", true),
            ("bar", "fooABar", false),
            // letters are used up: two e's needed, one available
            ("see", "s_e", false),
            ("abcd", "abc", false),
            ("xyz", "abc", false),
            // case is ignored by this rule, whatever the query holds
            ("WOR", "workspace", true),
            ("Wor", "workspace", true),
            ("", "anything", true),
        ];
        for (query, text, expected) in cases {
            assert_eq!(
                letters_match(query, text),
                expected,
                "{query:?} vs {text:?}"
            );
        }
    }

    #[test]
    fn test_letters_only_match_is_the_lowest_tier() {
        assert_eq!(
            match_tier("resolve_external_names", "ran"),
            Some(MatchTier::Letters)
        );
        assert_eq!(match_tier("ran_x", "ran"), Some(MatchTier::Prefix));
        assert_eq!(match_tier("xranx", "ran"), Some(MatchTier::Substring));
        assert_eq!(match_tier("details", "xyz"), None);
    }
}
