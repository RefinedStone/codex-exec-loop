use crate::application::service::planning::PlanningResetTarget;

/*
 * TUI reset spelling is an inbound grammar detail. The parser emits
 * PlanningResetTarget so command execution, buffered hints, and other inbound
 * surfaces stay on the same destructive reset vocabulary.
 */
pub(super) const PLANNING_RESET_USAGE_TEXT: &str =
    "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParsedPlanningResetShellCommand {
    pub(super) target: PlanningResetTarget,
    pub(super) confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningResetShellArgumentError {
    Missing,
    UnsupportedTarget,
    UnsupportedConfirmation,
    TooManyArguments,
}

pub(super) fn parse_planning_reset_shell_argument(
    argument: Option<&str>,
) -> Result<ParsedPlanningResetShellCommand, PlanningResetShellArgumentError> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(PlanningResetShellArgumentError::Missing);
    };
    let mut parts = argument.split_whitespace();
    let target = parts
        .next()
        .expect("non-empty trimmed reset argument should contain a target token");
    let confirmation = parts.next();
    let confirmed = match confirmation {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("confirm") => true,
        Some(_) => {
            return Err(PlanningResetShellArgumentError::UnsupportedConfirmation);
        }
    };
    if parts.next().is_some() {
        return Err(PlanningResetShellArgumentError::TooManyArguments);
    }
    let target = match target.to_ascii_lowercase().as_str() {
        "queue" => PlanningResetTarget::Queue,
        "directions" => PlanningResetTarget::Directions,
        "all" => PlanningResetTarget::All,
        _ => {
            return Err(PlanningResetShellArgumentError::UnsupportedTarget);
        }
    };
    Ok(ParsedPlanningResetShellCommand { target, confirmed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_shell_argument_maps_to_shared_application_targets() {
        for (raw, expected, confirmed) in [
            ("queue", PlanningResetTarget::Queue, false),
            ("directions", PlanningResetTarget::Directions, false),
            ("directions confirm", PlanningResetTarget::Directions, true),
            ("all", PlanningResetTarget::All, false),
            ("all confirm", PlanningResetTarget::All, true),
        ] {
            let parsed =
                parse_planning_reset_shell_argument(Some(raw)).expect("reset target should parse");
            assert_eq!(parsed.target, expected);
            assert_eq!(parsed.confirmed, confirmed);
        }

        assert_eq!(
            parse_planning_reset_shell_argument(Some("tasks")),
            Err(PlanningResetShellArgumentError::UnsupportedTarget)
        );
        assert_eq!(
            parse_planning_reset_shell_argument(Some("queue now")),
            Err(PlanningResetShellArgumentError::UnsupportedConfirmation)
        );
        assert_eq!(
            parse_planning_reset_shell_argument(Some("directions confirm now")),
            Err(PlanningResetShellArgumentError::TooManyArguments)
        );
        assert_eq!(
            parse_planning_reset_shell_argument(None),
            Err(PlanningResetShellArgumentError::Missing)
        );
        assert_eq!(
            parse_planning_reset_shell_argument(Some("  \t  ")),
            Err(PlanningResetShellArgumentError::Missing)
        );
    }
}
