use std::collections::{HashMap, HashSet};

use chrono::DateTime;

use super::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningFileKind, PlanningValidationReport,
    TaskActor, TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

#[derive(Default, Clone)]
pub struct PlanningSemanticValidationService;

impl PlanningSemanticValidationService {
    pub fn new() -> Self {
        Self
    }

    pub fn validate(
        &self,
        direction_catalog: Option<&DirectionCatalogDocument>,
        task_authority: Option<&TaskAuthorityDocument>,
        report: &mut PlanningValidationReport,
    ) {
        if let Some(direction_catalog) = direction_catalog {
            self.validate_direction_catalog(direction_catalog, report);
        }
        if let Some(task_authority) = task_authority {
            self.validate_task_authority(task_authority, report);
        }
        if let (Some(direction_catalog), Some(task_authority)) = (direction_catalog, task_authority)
        {
            self.validate_cross_references(direction_catalog, task_authority, report);
        }
    }

    fn validate_direction_catalog(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        report: &mut PlanningValidationReport,
    ) {
        if direction_catalog.version != PLANNING_FORMAT_VERSION {
            report.push_error(
                PlanningFileKind::Directions,
                "unsupported_directions_version",
                format!(
                    "directions.toml version {} does not match supported version {}",
                    direction_catalog.version, PLANNING_FORMAT_VERSION
                ),
            );
        }

        if direction_catalog.directions.is_empty() {
            report.push_error(
                PlanningFileKind::Directions,
                "missing_directions",
                "directions.toml must contain at least one direction",
            );
            return;
        }

        let mut seen_ids = HashSet::new();
        for direction in &direction_catalog.directions {
            let direction_id = direction.id.trim();
            if direction_id.is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_id",
                    "direction ids must not be blank",
                );
            } else if !seen_ids.insert(direction_id.to_string()) {
                report.push_error(
                    PlanningFileKind::Directions,
                    "duplicate_direction_id",
                    format!("duplicate direction id: {direction_id}"),
                );
            }

            if direction.title.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_title",
                    format!("direction {direction_id} must have a non-empty title"),
                );
            }
            if direction.summary.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::Directions,
                    "blank_direction_summary",
                    format!("direction {direction_id} must have a non-empty summary"),
                );
            }
            if direction.success_criteria.is_empty()
                || direction
                    .success_criteria
                    .iter()
                    .any(|criterion| criterion.trim().is_empty())
            {
                report.push_error(
                    PlanningFileKind::Directions,
                    "invalid_success_criteria",
                    format!(
                        "direction {direction_id} must include at least one non-empty success criterion"
                    ),
                );
            }
        }
    }

    fn validate_task_authority(
        &self,
        task_authority: &TaskAuthorityDocument,
        report: &mut PlanningValidationReport,
    ) {
        if task_authority.version != PLANNING_FORMAT_VERSION {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "unsupported_task_authority_version",
                format!(
                    "task authority version {} does not match supported version {}",
                    task_authority.version, PLANNING_FORMAT_VERSION
                ),
            );
        }

        let mut seen_ids = HashSet::new();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if task_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_task_id",
                    "task ids must not be blank",
                );
            } else if !seen_ids.insert(task_id.to_string()) {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "duplicate_task_id",
                    format!("duplicate task id: {task_id}"),
                );
            }

            if task.direction_id.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_direction_reference",
                    format!("task {task_id} must reference a direction_id"),
                );
            }
            if task.title.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_task_title",
                    format!("task {task_id} must have a non-empty title"),
                );
            }
            if task.description.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_task_description",
                    format!("task {task_id} must have a non-empty description"),
                );
            }
            if task.requires_relation_note() && task.direction_relation_note.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "missing_direction_relation_note",
                    format!("LLM-authored task {task_id} must include direction_relation_note"),
                );
            }
            if task.dynamic_priority_delta != 0 && task.priority_reason.trim().is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "missing_priority_reason",
                    format!(
                        "task {task_id} must include priority_reason when dynamic_priority_delta is non-zero"
                    ),
                );
            }
            if DateTime::parse_from_rfc3339(task.updated_at.as_str()).is_err() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "invalid_updated_at",
                    format!("task {task_id} must use RFC3339 updated_at"),
                );
            }

            self.validate_task_links(task, report);
        }
    }

    fn validate_task_links(&self, task: &TaskDefinition, report: &mut PlanningValidationReport) {
        let task_id = task.id.trim();
        let mut dependency_ids = HashSet::new();
        for dependency_id in &task.depends_on {
            let normalized_dependency_id = dependency_id.trim();
            if normalized_dependency_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_dependency_id",
                    format!("task {task_id} contains a blank depends_on entry"),
                );
                continue;
            }
            if normalized_dependency_id == task_id {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "self_dependency",
                    format!("task {task_id} cannot depend on itself"),
                );
            }
            if !dependency_ids.insert(normalized_dependency_id.to_string()) {
                report.push_warning(
                    PlanningFileKind::TaskAuthority,
                    "duplicate_dependency_id",
                    format!("task {task_id} repeats dependency id {normalized_dependency_id}"),
                );
            }
        }

        let mut blocker_ids = HashSet::new();
        for blocker_id in &task.blocked_by {
            let normalized_blocker_id = blocker_id.trim();
            if normalized_blocker_id.is_empty() {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "blank_blocker_id",
                    format!("task {task_id} contains a blank blocked_by entry"),
                );
                continue;
            }
            if !blocker_ids.insert(normalized_blocker_id.to_string()) {
                report.push_warning(
                    PlanningFileKind::TaskAuthority,
                    "duplicate_blocker_id",
                    format!("task {task_id} repeats blocker id {normalized_blocker_id}"),
                );
            }
            if normalized_blocker_id == task_id {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "self_blocker",
                    format!("task {task_id} cannot block itself"),
                );
            }
        }

        let mut conflict_ids = HashSet::new();
        for dependency_id in &task.depends_on {
            let normalized_dependency_id = dependency_id.trim();
            if normalized_dependency_id.is_empty() {
                continue;
            }
            if blocker_ids.contains(normalized_dependency_id)
                && conflict_ids.insert(normalized_dependency_id.to_string())
            {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "dependency_blocker_conflict",
                    format!(
                        "task {task_id} cannot list {normalized_dependency_id} in both depends_on and blocked_by"
                    ),
                );
            }
        }

        if matches!(task.status, TaskStatus::Proposed) && task.created_by == TaskActor::Llm {
            report.push_warning(
                PlanningFileKind::TaskAuthority,
                "llm_proposed_task",
                format!(
                    "task {task_id} is proposed by the LLM and will stay out of normal execution until promoted"
                ),
            );
        }
    }

    fn validate_cross_references(
        &self,
        direction_catalog: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
        report: &mut PlanningValidationReport,
    ) {
        let direction_ids = direction_catalog
            .directions
            .iter()
            .map(|direction| direction.id.trim().to_string())
            .collect::<HashSet<_>>();
        let task_map = task_authority
            .tasks
            .iter()
            .map(|task| (task.id.trim().to_string(), task))
            .collect::<HashMap<_, _>>();

        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if !direction_ids.contains(task.direction_id.trim()) {
                report.push_error(
                    PlanningFileKind::TaskAuthority,
                    "missing_direction_reference",
                    format!(
                        "task {task_id} references unknown direction_id {}",
                        task.direction_id.trim()
                    ),
                );
            }
            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if !task_map.contains_key(normalized_dependency_id) {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "missing_dependency_reference",
                        format!(
                            "task {task_id} references unknown dependency {normalized_dependency_id}"
                        ),
                    );
                }
            }
            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if !task_map.contains_key(normalized_blocker_id) {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "missing_blocker_reference",
                        format!(
                            "task {task_id} references unknown blocker {normalized_blocker_id}"
                        ),
                    );
                }
            }
        }

        self.validate_task_semantics(task_authority, &task_map, report);

        if self.contains_dependency_cycle(task_authority) {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "dependency_cycle_detected",
                "task authority contains a dependency cycle",
            );
        }
    }

    fn contains_dependency_cycle(&self, task_authority: &TaskAuthorityDocument) -> bool {
        let adjacency_map = task_authority
            .tasks
            .iter()
            .map(|task| {
                (
                    task.id.trim().to_string(),
                    task.depends_on
                        .iter()
                        .map(|dependency_id| dependency_id.trim().to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut temporary_marks = HashSet::new();
        let mut permanent_marks = HashSet::new();

        adjacency_map.keys().any(|task_id| {
            self.detect_cycle(
                task_id,
                &adjacency_map,
                &mut temporary_marks,
                &mut permanent_marks,
            )
        })
    }

    fn detect_cycle(
        &self,
        task_id: &str,
        adjacency_map: &HashMap<String, Vec<String>>,
        temporary_marks: &mut HashSet<String>,
        permanent_marks: &mut HashSet<String>,
    ) -> bool {
        if permanent_marks.contains(task_id) {
            return false;
        }
        if !temporary_marks.insert(task_id.to_string()) {
            return true;
        }

        if let Some(dependencies) = adjacency_map.get(task_id) {
            for dependency_id in dependencies {
                if adjacency_map.contains_key(dependency_id)
                    && self.detect_cycle(
                        dependency_id,
                        adjacency_map,
                        temporary_marks,
                        permanent_marks,
                    )
                {
                    return true;
                }
            }
        }

        temporary_marks.remove(task_id);
        permanent_marks.insert(task_id.to_string());
        false
    }

    fn validate_task_semantics(
        &self,
        task_authority: &TaskAuthorityDocument,
        task_map: &HashMap<String, &TaskDefinition>,
        report: &mut PlanningValidationReport,
    ) {
        let mut in_progress_task_ids = Vec::new();

        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            if task.status == TaskStatus::InProgress {
                in_progress_task_ids.push(task_id);
            }
            if task.status != TaskStatus::Done {
                continue;
            }

            for dependency_id in &task.depends_on {
                let normalized_dependency_id = dependency_id.trim();
                if let Some(dependency) = task_map.get(normalized_dependency_id)
                    && !dependency.status.is_dependency_complete()
                {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "done_task_unresolved_dependency",
                        format!(
                            "done task {task_id} cannot depend on incomplete task {normalized_dependency_id} ({})",
                            dependency.status.label()
                        ),
                    );
                }
            }

            for blocker_id in &task.blocked_by {
                let normalized_blocker_id = blocker_id.trim();
                if let Some(blocker) = task_map.get(normalized_blocker_id)
                    && !blocker.status.clears_blocker()
                {
                    report.push_error(
                        PlanningFileKind::TaskAuthority,
                        "done_task_unresolved_blocker",
                        format!(
                            "done task {task_id} cannot remain blocked by task {normalized_blocker_id} ({})",
                            blocker.status.label()
                        ),
                    );
                }
            }
        }

        if in_progress_task_ids.len() > 1 {
            report.push_error(
                PlanningFileKind::TaskAuthority,
                "multiple_in_progress_tasks",
                format!(
                    "task authority may contain at most one in_progress task; found {}: {}",
                    in_progress_task_ids.len(),
                    in_progress_task_ids.join(", ")
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningSemanticValidationService;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningFileKind,
        PlanningValidationReport, QueueIdleConfig, TaskActor, TaskAuthorityDocument,
        TaskDefinition, TaskStatus,
    };

    fn direction(id: &str) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("{id} title"),
            summary: format!("{id} summary"),
            success_criteria: vec![format!("{id} done")],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }
    }

    fn task(id: &str, status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: format!("{id} title"),
            description: format!("{id} description"),
            status,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            updated_at: "2026-04-09T09:00:00Z".to_string(),
        }
    }

    fn validate(
        directions: &DirectionCatalogDocument,
        ledger: &TaskAuthorityDocument,
    ) -> PlanningValidationReport {
        let mut report = PlanningValidationReport::new();
        PlanningSemanticValidationService::new().validate(
            Some(directions),
            Some(ledger),
            &mut report,
        );
        report
    }

    #[test]
    fn validates_cross_references_and_dependency_cycles() {
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction("direction-a")],
        };
        let mut first = task("task-a", TaskStatus::Ready);
        first.depends_on = vec!["task-b".to_string()];
        let mut second = task("task-b", TaskStatus::Ready);
        second.depends_on = vec!["task-a".to_string(), "missing-task".to_string()];
        let ledger = TaskAuthorityDocument {
            version: 1,
            tasks: vec![first, second],
        };

        let report = validate(&directions, &ledger);
        let codes = report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<Vec<_>>();

        assert!(codes.contains(&"missing_dependency_reference"));
        assert!(codes.contains(&"dependency_cycle_detected"));
    }

    #[test]
    fn validates_done_task_dependency_and_single_in_progress_rules() {
        let directions = DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction("direction-a")],
        };
        let mut done = task("done-task", TaskStatus::Done);
        done.depends_on = vec!["open-task".to_string()];
        let ledger = TaskAuthorityDocument {
            version: 1,
            tasks: vec![
                done,
                task("open-task", TaskStatus::Ready),
                task("active-a", TaskStatus::InProgress),
                task("active-b", TaskStatus::InProgress),
            ],
        };

        let report = validate(&directions, &ledger);

        assert!(report.issues.iter().any(|issue| {
            issue.file_kind == PlanningFileKind::TaskAuthority
                && issue.code == "done_task_unresolved_dependency"
        }));
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "multiple_in_progress_tasks")
        );
    }
}
