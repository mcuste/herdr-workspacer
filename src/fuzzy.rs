use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

use crate::candidate::Candidate;

/// Returns fuzzy-match indexes ranked by relevance, retaining input order for score ties.
pub fn filter_indices(candidates: &[Candidate], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..candidates.len()).collect();
    }

    let matcher = SkimMatcherV2::default().smart_case();
    let mut ranked = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            matcher
                .fuzzy_match(&candidate.search_text, query)
                .map(|score| (score, index))
        })
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|(left_score, left_index), (right_score, right_index)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_index.cmp(right_index))
    });
    ranked.into_iter().map(|(_, index)| index).collect()
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

    fn workspace_candidate(label: &str) -> Candidate {
        let mut candidate = candidate(label);
        candidate.kind = CandidateKind::Workspace {
            workspace_id: label.to_string(),
        };
        candidate
    }

    #[test]
    fn ranks_exact_matches_before_partial_matches() {
        let candidates = vec![candidate("old-workspace"), candidate("workspace")];

        assert_eq!(filter_indices(&candidates, "workspace"), vec![1, 0]);
    }

    #[test]
    fn ranks_exact_zoxide_matches_before_partial_workspace_matches() {
        let candidates = vec![candidate("workspace"), workspace_candidate("old-workspace")];

        assert_eq!(filter_indices(&candidates, "workspace"), vec![0, 1]);
    }

    #[test]
    fn does_not_match_hidden_absolute_path_segments() {
        let mut dotfiles = candidate("dotfiles");
        dotfiles.search_text = "dotfiles ~/dotfiles".to_string();

        assert!(filter_indices(&[dotfiles], "tism").is_empty());
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
