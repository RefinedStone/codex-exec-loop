pub(super) const PLANNING_SHELL_USAGE_TEXT: &str =
    "supported: :planning, :planning doctor, :doctor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParsedPlanningShellCommand {
    OpenControlCenter,
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningShellArgumentError {
    argument: String,
}

impl PlanningShellArgumentError {
    pub(super) fn argument(&self) -> &str {
        self.argument.as_str()
    }
}

pub(super) fn parse_planning_shell_argument(
    argument: Option<&str>,
) -> Result<ParsedPlanningShellCommand, PlanningShellArgumentError> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ParsedPlanningShellCommand::OpenControlCenter);
    };
    if argument.eq_ignore_ascii_case("doctor") {
        return Ok(ParsedPlanningShellCommand::Doctor);
    }
    Err(PlanningShellArgumentError {
        argument: argument.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_shell_argument_maps_to_tui_planning_command() {
        assert_eq!(
            parse_planning_shell_argument(None),
            Ok(ParsedPlanningShellCommand::OpenControlCenter)
        );
        assert_eq!(
            parse_planning_shell_argument(Some("  ")),
            Ok(ParsedPlanningShellCommand::OpenControlCenter)
        );
        assert_eq!(
            parse_planning_shell_argument(Some("doctor")),
            Ok(ParsedPlanningShellCommand::Doctor)
        );
        assert_eq!(
            parse_planning_shell_argument(Some("DOCTOR")),
            Ok(ParsedPlanningShellCommand::Doctor)
        );
        assert_eq!(
            parse_planning_shell_argument(Some("status"))
                .expect_err("unsupported argument should fail")
                .argument(),
            "status"
        );
        assert_eq!(
            parse_planning_shell_argument(Some("doctor now"))
                .expect_err("extra argument should fail")
                .argument(),
            "doctor now"
        );
    }
}
