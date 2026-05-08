use std::fmt::{Display, Formatter};

use super::{DirectionCatalogDocument, DirectionDefinition, DirectionState};

#[derive(Debug, Default, Clone)]
// active direction selection은 task creation/intake가 공유하는 순수 domain fallback policy다.
pub struct PlanningActiveDirectionPolicy;

impl PlanningActiveDirectionPolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn select_direction<'a>(
        &self,
        requested_direction_id: Option<&str>,
        directions: &'a DirectionCatalogDocument,
    ) -> Result<&'a DirectionDefinition, PlanningActiveDirectionSelectionError> {
        if let Some(requested_direction_id) = requested_direction_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let direction_id = validate_direction_id(requested_direction_id)?;
            let direction = directions
                .directions
                .iter()
                .find(|direction| direction.id.trim() == direction_id)
                .ok_or_else(|| PlanningActiveDirectionSelectionError::UnknownDirection {
                    direction_id: direction_id.to_string(),
                })?;
            if direction.state != DirectionState::Active {
                return Err(PlanningActiveDirectionSelectionError::InactiveDirection {
                    direction_id: direction.id.trim().to_string(),
                });
            }
            return Ok(direction);
        }

        if let Some(direction) = directions.directions.iter().find(|direction| {
            direction.id.trim() == "general-workstream" && direction.state == DirectionState::Active
        }) {
            return Ok(direction);
        }

        directions
            .directions
            .iter()
            .find(|direction| direction.state == DirectionState::Active)
            .ok_or(PlanningActiveDirectionSelectionError::NoActiveDirection)
    }

    pub fn default_relation_note(
        &self,
        raw_note: Option<&str>,
        direction: &DirectionDefinition,
    ) -> String {
        raw_note
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "Task supports direction `{}`: {}",
                    direction.id.trim(),
                    direction.summary.trim()
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningActiveDirectionSelectionError {
    InvalidDirectionId { direction_id: String },
    UnknownDirection { direction_id: String },
    InactiveDirection { direction_id: String },
    NoActiveDirection,
}

impl Display for PlanningActiveDirectionSelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDirectionId { direction_id } => write!(
                formatter,
                "direction id `{direction_id}` must not contain whitespace or path separators"
            ),
            Self::UnknownDirection { direction_id } => {
                write!(formatter, "direction `{direction_id}` does not exist")
            }
            Self::InactiveDirection { direction_id } => write!(
                formatter,
                "direction `{direction_id}` is not active; task mutations can only create tasks for active directions"
            ),
            Self::NoActiveDirection => {
                write!(
                    formatter,
                    "task mutation requires an active planning direction"
                )
            }
        }
    }
}

impl std::error::Error for PlanningActiveDirectionSelectionError {}

fn validate_direction_id(
    direction_id: &str,
) -> Result<&str, PlanningActiveDirectionSelectionError> {
    if direction_id.contains(char::is_whitespace)
        || direction_id.contains('/')
        || direction_id.contains('\\')
    {
        return Err(PlanningActiveDirectionSelectionError::InvalidDirectionId {
            direction_id: direction_id.to_string(),
        });
    }
    Ok(direction_id)
}

#[cfg(test)]
mod tests {
    use super::{PlanningActiveDirectionPolicy, PlanningActiveDirectionSelectionError};
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig,
    };

    fn direction(id: &str, state: DirectionState) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("{id} title"),
            summary: format!("{id} summary"),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state,
        }
    }

    fn catalog(directions: Vec<DirectionDefinition>) -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions,
        }
    }

    #[test]
    fn selects_requested_active_direction() {
        let directions = catalog(vec![
            direction("general-workstream", DirectionState::Active),
            direction("special", DirectionState::Active),
        ]);

        let selected = PlanningActiveDirectionPolicy::new()
            .select_direction(Some(" special "), &directions)
            .unwrap();

        assert_eq!(selected.id, "special");
    }

    #[test]
    fn rejects_inactive_requested_direction() {
        let directions = catalog(vec![direction("paused", DirectionState::Paused)]);
        let error = PlanningActiveDirectionPolicy::new()
            .select_direction(Some("paused"), &directions)
            .unwrap_err();

        assert_eq!(
            error,
            PlanningActiveDirectionSelectionError::InactiveDirection {
                direction_id: "paused".to_string()
            }
        );
    }

    #[test]
    fn prefers_general_workstream_when_request_is_absent() {
        let directions = catalog(vec![
            direction("other", DirectionState::Active),
            direction("general-workstream", DirectionState::Active),
        ]);

        let selected = PlanningActiveDirectionPolicy::new()
            .select_direction(None, &directions)
            .unwrap();

        assert_eq!(selected.id, "general-workstream");
    }

    #[test]
    fn falls_back_to_first_active_direction() {
        let directions = catalog(vec![
            direction("general-workstream", DirectionState::Paused),
            direction("other", DirectionState::Active),
        ]);

        let selected = PlanningActiveDirectionPolicy::new()
            .select_direction(None, &directions)
            .unwrap();

        assert_eq!(selected.id, "other");
    }

    #[test]
    fn rejects_when_no_active_direction_exists() {
        let directions = catalog(vec![direction("done", DirectionState::Done)]);
        let error = PlanningActiveDirectionPolicy::new()
            .select_direction(None, &directions)
            .unwrap_err();

        assert_eq!(
            error,
            PlanningActiveDirectionSelectionError::NoActiveDirection
        );
    }

    #[test]
    fn builds_default_relation_note_from_direction_summary() {
        let directions = catalog(vec![direction(
            "general-workstream",
            DirectionState::Active,
        )]);
        let direction = directions.directions.first().unwrap();

        assert_eq!(
            PlanningActiveDirectionPolicy::new().default_relation_note(None, direction),
            "Task supports direction `general-workstream`: general-workstream summary"
        );
    }
}
