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

    /// Keeps the selected row when it is still visible after the replacement.
    pub(crate) fn replace_candidates(&mut self, candidates: Vec<Candidate>) {
        let selected = self
            .selected_candidate_index()
            .and_then(|index| self.candidates.get(index))
            .map(|candidate| (candidate.kind.clone(), candidate.canonical_path.clone()));
        self.candidates = candidates;
        self.visible = filter_indices(&self.candidates, &self.query);
        self.selected = selected
            .and_then(|(kind, canonical_path)| {
                self.visible.iter().position(|index| {
                    self.candidates.get(*index).is_some_and(|candidate| {
                        candidate.kind == kind && candidate.canonical_path == canonical_path
                    })
                })
            })
            .unwrap_or(0);
    }

    pub(crate) fn push_warning(&mut self, warning: &str) {
        match &mut self.warning {
            Some(existing) => {
                existing.push(' ');
                existing.push_str(warning);
            }
            None => self.warning = Some(warning.to_string()),
        }
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

    use super::*;
    use herdr_workspacer::{MruState, Workspace, ZoxideEntry, merge_candidates};

    fn workspace(label: &str, native_order: usize) -> Workspace {
        Workspace {
            id: label.to_string(),
            label: label.to_string(),
            is_worktree: false,
            path: PathBuf::from(format!("/projects/{label}")),
            native_order,
        }
    }

    fn entry(name: &str) -> ZoxideEntry {
        ZoxideEntry {
            path: PathBuf::from(format!("/zoxide/{name}")),
            score: 1.0,
        }
    }

    fn model(workspaces: &[&str], entries: &[&str]) -> anyhow::Result<PickerModel> {
        Ok(PickerModel::new(candidates(workspaces, entries)?, &[]))
    }

    fn candidates(workspaces: &[&str], entries: &[&str]) -> anyhow::Result<Vec<Candidate>> {
        merge_candidates(
            workspaces
                .iter()
                .enumerate()
                .map(|(order, label)| workspace(label, order))
                .collect(),
            entries.iter().map(|name| entry(name)).collect(),
            &MruState::default(),
        )
    }

    fn selected_label(model: &PickerModel) -> Option<&str> {
        model
            .selected_candidate_index()
            .and_then(|index| model.candidate(index))
            .map(|candidate| candidate.label.as_str())
    }

    #[test]
    fn filtering_resets_selection_to_the_first_visible_result() -> anyhow::Result<()> {
        let mut model = model(&["alpha", "beta"], &[])?;
        model.move_selection(1);

        model.push_query_character('a');

        anyhow::ensure!(model.selected() == 0, "selection was not reset");
        anyhow::ensure!(
            selected_label(&model) == Some("alpha"),
            "first visible row was not selected"
        );
        Ok(())
    }

    #[test]
    fn selection_stays_within_visible_results() -> anyhow::Result<()> {
        let mut model = model(&["alpha", "beta"], &[])?;

        model.move_selection(100);
        anyhow::ensure!(model.selected() == 1, "selection passed the last row");
        model.move_selection(-100);
        anyhow::ensure!(model.selected() == 0, "selection passed the first row");
        Ok(())
    }

    #[test]
    fn replacing_candidates_keeps_the_selected_row() -> anyhow::Result<()> {
        let mut model = model(&["first", "second"], &[])?;
        model.move_selection(1);

        model.replace_candidates(candidates(&["first", "second"], &["directory"])?);

        anyhow::ensure!(model.visible().len() == 3, "zoxide row was not added");
        anyhow::ensure!(
            selected_label(&model) == Some("second"),
            "selection moved after replacing candidates: {:?}",
            selected_label(&model)
        );
        Ok(())
    }

    #[test]
    fn replacing_candidates_applies_the_current_query() -> anyhow::Result<()> {
        let mut model = model(&["alpha", "beta"], &[])?;
        for character in "beta".chars() {
            model.push_query_character(character);
        }

        model.replace_candidates(candidates(&["alpha", "beta"], &["beta-notes", "gamma"])?);

        let visible = model
            .visible()
            .iter()
            .filter_map(|index| model.candidate(*index))
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            visible == vec!["beta", "beta-notes"],
            "query was not applied to the new candidates: {visible:?}"
        );
        anyhow::ensure!(
            selected_label(&model) == Some("beta"),
            "selection moved away from the matching row"
        );
        Ok(())
    }

    #[test]
    fn replacing_candidates_resets_selection_when_the_row_is_gone() -> anyhow::Result<()> {
        let mut model = model(&["alpha"], &["old"])?;
        model.move_selection(1);

        model.replace_candidates(candidates(&["alpha"], &["new"])?);

        anyhow::ensure!(
            selected_label(&model) == Some("alpha"),
            "selection did not return to the first row: {:?}",
            selected_label(&model)
        );
        Ok(())
    }

    #[test]
    fn warnings_accumulate_in_one_line() {
        let mut model = PickerModel::new(Vec::new(), &[]);
        assert_eq!(model.warning(), None);

        model.push_warning("first.");
        model.push_warning("second.");

        assert_eq!(model.warning(), Some("first. second."));
    }
}
