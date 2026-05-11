pub(super) const PARALLEL_MODE_SHELL_USAGE_TEXT: &str =
    "supported: :parallel, :pa, :parallel off, :pa off";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParsedParallelModeShellCommand {
    Enable,
    Disable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelModeShellArgumentError {
    argument: String,
}

impl ParallelModeShellArgumentError {
    pub(super) fn argument(&self) -> &str {
        self.argument.as_str()
    }
}

pub(super) fn parse_parallel_mode_shell_argument(
    argument: Option<&str>,
) -> Result<ParsedParallelModeShellCommand, ParallelModeShellArgumentError> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ParsedParallelModeShellCommand::Enable);
    };
    if argument.eq_ignore_ascii_case("off") {
        return Ok(ParsedParallelModeShellCommand::Disable);
    }
    Err(ParallelModeShellArgumentError {
        argument: argument.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_shell_argument_maps_to_shared_tui_command() {
        assert_eq!(
            parse_parallel_mode_shell_argument(None),
            Ok(ParsedParallelModeShellCommand::Enable)
        );
        assert_eq!(
            parse_parallel_mode_shell_argument(Some("  ")),
            Ok(ParsedParallelModeShellCommand::Enable)
        );
        assert_eq!(
            parse_parallel_mode_shell_argument(Some("off")),
            Ok(ParsedParallelModeShellCommand::Disable)
        );
        assert_eq!(
            parse_parallel_mode_shell_argument(Some("OFF")),
            Ok(ParsedParallelModeShellCommand::Disable)
        );
        assert_eq!(
            parse_parallel_mode_shell_argument(Some("on"))
                .expect_err("unsupported argument should fail")
                .argument(),
            "on"
        );
        assert_eq!(
            parse_parallel_mode_shell_argument(Some("off now"))
                .expect_err("extra argument should fail")
                .argument(),
            "off now"
        );
    }
}
