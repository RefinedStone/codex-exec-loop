use std::collections::BTreeSet;

#[derive(Debug, Default, Clone)]
// task reference policy는 depends_on/blocked_by 배열을 authority graph용 semantic set으로 정규화한다.
pub struct PlanningTaskReferencePolicy;

impl PlanningTaskReferencePolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn normalize_references(&self, values: &[String]) -> Vec<String> {
        values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningTaskReferencePolicy;

    #[test]
    fn trims_deduplicates_and_sorts_task_references() {
        let normalized = PlanningTaskReferencePolicy::new().normalize_references(&[
            " task-b ".to_string(),
            "".to_string(),
            "task-a".to_string(),
            "task-b".to_string(),
            "   ".to_string(),
        ]);

        assert_eq!(normalized, vec!["task-a", "task-b"]);
    }
}
