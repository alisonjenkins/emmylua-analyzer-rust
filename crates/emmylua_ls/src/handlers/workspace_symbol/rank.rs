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
    tier_of(&text.to_lowercase(), &query.to_lowercase())
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

#[cfg(test)]
mod tests {
    use super::*;

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
