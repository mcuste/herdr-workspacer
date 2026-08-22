use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

use crate::candidate::Candidate;

/// Returns candidate indexes that fuzzy-match a query in their original order.
pub fn filter_indices(candidates: &[Candidate], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..candidates.len()).collect();
    }

    let matcher = SkimMatcherV2::default().smart_case();
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            matcher
                .fuzzy_match(&candidate.search_text, query)
                .map(|_| index)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::candidate::CandidateKind;

    use super::*;

    fn candidate(label: &str) -> Candidate {
        Candidate {
            kind: CandidateKind::Directory,
            path: PathBuf::from(format!("/code/{label}")),
            canonical_path: PathBuf::from(format!("/code/{label}")),
            display_path: format!("/code/{label}"),
            label: label.to_string(),
            search_text: format!("{label} /code/{label} {label}"),
            zoxide_score: None,
            source_order: 0,
        }
    }

    #[test]
    fn keeps_input_order_after_fuzzy_filtering() {
        let candidates = vec![
            candidate("herdr"),
            candidate("herd-web"),
            candidate("old-herdr"),
        ];

        assert_eq!(filter_indices(&candidates, "hdr"), vec![0, 1, 2]);
    }

    #[test]
    fn matches_unicode_paths() {
        let candidates = vec![candidate("プロジェクト")];

        assert_eq!(filter_indices(&candidates, "プロ"), vec![0]);
    }

    #[test]
    fn accepts_empty_query() {
        let candidates = vec![candidate("one"), candidate("two")];

        assert_eq!(filter_indices(&candidates, ""), vec![0, 1]);
    }
}
