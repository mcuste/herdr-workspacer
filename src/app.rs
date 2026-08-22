use herdr_workspacer::{Candidate, filter_indices};

#[derive(Debug)]
pub(crate) struct PickerModel {
    candidates: Vec<Candidate>,
    visible: Vec<usize>,
    query: String,
    selected: usize,
    warning: Option<String>,
}

impl PickerModel {
    pub(crate) fn new(candidates: Vec<Candidate>, warnings: &[String]) -> Self {
        let visible = filter_indices(&candidates, "");
        let warning = (!warnings.is_empty()).then(|| warnings.join(" "));

        Self {
            candidates,
            visible,
            query: String::new(),
            selected: 0,
            warning,
        }
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn visible(&self) -> &[usize] {
        &self.visible
    }

    pub(crate) fn candidate(&self, index: usize) -> Option<&Candidate> {
        self.candidates.get(index)
    }

    pub(crate) fn selected_candidate_index(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub(crate) fn push_query_character(&mut self, character: char) {
        self.query.push(character);
        self.refresh_visible();
    }

    pub(crate) fn backspace(&mut self) {
        self.query.pop();
        self.refresh_visible();
    }

    pub(crate) fn clear_query(&mut self) {
        self.query.clear();
        self.refresh_visible();
    }

    pub(crate) fn move_selection(&mut self, amount: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }

        let length = self.visible.len();
        self.selected = if amount.is_negative() {
            self.selected
                .saturating_sub(amount.unsigned_abs())
                .min(length - 1)
        } else {
            self.selected
                .saturating_add(amount.unsigned_abs())
                .min(length - 1)
        };
    }

    fn refresh_visible(&mut self) {
        self.visible = filter_indices(&self.candidates, &self.query);
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use herdr_workspacer::{MruState, Workspace, merge_candidates};

    use super::*;

    fn candidates() -> Option<Vec<Candidate>> {
        merge_candidates(
            vec![
                Workspace {
                    id: "alpha".to_string(),
                    label: "alpha".to_string(),
                    path: PathBuf::from("/code/alpha"),
                    native_order: 0,
                },
                Workspace {
                    id: "beta".to_string(),
                    label: "beta".to_string(),
                    path: PathBuf::from("/code/beta"),
                    native_order: 1,
                },
            ],
            Vec::new(),
            &MruState::default(),
        )
        .ok()
    }

    #[test]
    fn filtering_resets_selection_to_the_first_visible_result() {
        let candidates = candidates();
        assert!(candidates.is_some());
        if let Some(candidates) = candidates {
            let mut model = PickerModel::new(candidates, &[]);
            model.move_selection(1);
            model.push_query_character('a');

            assert_eq!(model.selected(), 0);
            assert_eq!(model.selected_candidate_index(), Some(0));
        }
    }

    #[test]
    fn selection_stays_within_visible_results() {
        let candidates = candidates();
        assert!(candidates.is_some());
        if let Some(candidates) = candidates {
            let mut model = PickerModel::new(candidates, &[]);

            model.move_selection(100);
            assert_eq!(model.selected(), 1);
            model.move_selection(-100);
            assert_eq!(model.selected(), 0);
        }
    }
}
