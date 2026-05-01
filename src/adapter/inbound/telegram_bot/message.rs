use std::sync::Arc;

use anyhow::{Result, bail};

use crate::application::service::planning::control::{
    PlanningControlResetOutcome, PlanningControlService, PlanningControlStatusSnapshot,
    PlanningControlSurface,
};
use crate::application::service::planning::{PlanningControlCommand, PlanningResetTarget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TelegramInboundCommand {
    WhoAmI,
    Planning(PlanningControlCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TelegramParsedMessage {
    Ignore,
    Error(String),
    Command(TelegramInboundCommand),
}

pub(super) fn parse_message(text: Option<&str>) -> TelegramParsedMessage {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return TelegramParsedMessage::Ignore;
    };

    let mut parts = text.split_whitespace();
    let Some(raw_command) = parts.next() else {
        return TelegramParsedMessage::Ignore;
    };
    let command = normalize_command(raw_command);
    let arguments = parts.collect::<Vec<_>>();

    match command.as_str() {
        "/start" | "/help" | "help" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Help),
            "/help",
        ),
        "/whoami" => {
            parse_command_without_arguments(&arguments, TelegramInboundCommand::WhoAmI, "/whoami")
        }
        "/status" | "status" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Status),
            "/status",
        ),
        "/queue" | "queue" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Queue),
            "/queue",
        ),
        "/plan" => parse_plan_arguments(&arguments),
        "/reset_queue" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Reset(
                PlanningResetTarget::Queue,
            )),
            "/reset_queue",
        ),
        "/reset_directions" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Reset(
                PlanningResetTarget::Directions,
            )),
            "/reset_directions",
        ),
        "/reset_all" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Reset(
                PlanningResetTarget::All,
            )),
            "/reset_all",
        ),
        "/reset" => parse_reset_arguments(&arguments),
        token if token.starts_with('/') => TelegramParsedMessage::Error(format!(
            "지원하지 않는 명령어입니다: {token}\n{}",
            PlanningControlService::new(Arc::new(NoopPlanningControlSurface)).help_text()
        )),
        _ => TelegramParsedMessage::Ignore,
    }
}

fn parse_command_without_arguments(
    arguments: &[&str],
    command: TelegramInboundCommand,
    usage: &'static str,
) -> TelegramParsedMessage {
    if arguments.is_empty() {
        TelegramParsedMessage::Command(command)
    } else {
        TelegramParsedMessage::Error(format!("사용법: {usage}"))
    }
}

fn parse_plan_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    match arguments {
        [] | ["status"] => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status,
        )),
        _ => TelegramParsedMessage::Error("사용법: /plan [status]".to_string()),
    }
}

fn parse_reset_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    let [target] = arguments else {
        return TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string(),
        );
    };

    let command = match *target {
        "queue" => PlanningControlCommand::Reset(PlanningResetTarget::Queue),
        "directions" => PlanningControlCommand::Reset(PlanningResetTarget::Directions),
        "all" => PlanningControlCommand::Reset(PlanningResetTarget::All),
        _ => {
            return TelegramParsedMessage::Error(
                "사용법: /reset queue | /reset directions | /reset all".to_string(),
            );
        }
    };
    TelegramParsedMessage::Command(TelegramInboundCommand::Planning(command))
}

fn normalize_command(raw_command: &str) -> String {
    let lowered = raw_command.to_ascii_lowercase();
    let mut parts = lowered.split('@');
    parts.next().unwrap_or_default().to_string()
}

struct NoopPlanningControlSurface;

impl PlanningControlSurface for NoopPlanningControlSurface {
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        bail!("noop control surface should not execute");
    }

    fn reset_workspace(&self, _target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        bail!("noop control surface should not execute");
    }
}
