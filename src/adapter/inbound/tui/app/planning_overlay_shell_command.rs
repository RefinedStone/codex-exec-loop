#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningOverlayShellArgumentError {
    argument: String,
}

impl PlanningOverlayShellArgumentError {
    pub(super) fn argument(&self) -> &str {
        self.argument.as_str()
    }
}

pub(super) fn parse_planning_overlay_shell_argument(
    argument: Option<&str>,
) -> Result<(), PlanningOverlayShellArgumentError> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    Err(PlanningOverlayShellArgumentError {
        argument: argument.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_overlay_shell_argument_accepts_only_empty_argument() {
        assert_eq!(parse_planning_overlay_shell_argument(None), Ok(()));
        assert_eq!(parse_planning_overlay_shell_argument(Some("  ")), Ok(()));
        assert_eq!(
            parse_planning_overlay_shell_argument(Some("later"))
                .expect_err("overlay command should reject arguments")
                .argument(),
            "later"
        );
    }
}
