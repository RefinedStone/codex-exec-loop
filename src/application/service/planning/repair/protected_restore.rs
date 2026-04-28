#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProtectedFileRestoration {
    pub relative_path: &'static str,
    pub archived_candidate_path: Option<String>,
}
