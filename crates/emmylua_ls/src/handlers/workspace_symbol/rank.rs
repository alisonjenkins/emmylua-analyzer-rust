use std::path::{Component, Path, PathBuf};

/// How well a symbol name matches the query; lower sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchTier {
    Exact,
    Prefix,
    Substring,
}

/// Smartcase: a query containing an uppercase letter matches case-sensitively,
/// otherwise case is ignored. An empty query matches everything.
pub fn match_tier(text: &str, query: &str) -> Option<MatchTier> {
    if query.is_empty() {
        return Some(MatchTier::Substring);
    }
    if query.chars().any(|c| c.is_uppercase()) {
        return tier_of(text, query);
    }
    if query.is_ascii() && text.is_ascii() {
        // Runs once per name use in the workspace; skip allocating lowercased copies.
        return ascii_tier_ignore_case(text, query);
    }
    tier_of(&text.to_lowercase(), &query.to_lowercase())
}

fn ascii_tier_ignore_case(text: &str, query: &str) -> Option<MatchTier> {
    let (text, query) = (text.as_bytes(), query.as_bytes());
    if text.eq_ignore_ascii_case(query) {
        Some(MatchTier::Exact)
    } else if text
        .get(..query.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(query))
    {
        Some(MatchTier::Prefix)
    } else if text
        .windows(query.len())
        .any(|window| window.eq_ignore_ascii_case(query))
    {
        Some(MatchTier::Substring)
    } else {
        None
    }
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
            ("needle", "Needle", None),
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
    fn test_tiers_sort_exact_prefix_substring() {
        assert!(MatchTier::Exact < MatchTier::Prefix);
        assert!(MatchTier::Prefix < MatchTier::Substring);
    }
}
