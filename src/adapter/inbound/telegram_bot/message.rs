use std::sync::Arc;

use anyhow::{Result, bail};

use crate::application::service::planning::control::{
    PlanningControlResetOutcome, PlanningControlService, PlanningControlStatusSnapshot,
    PlanningControlSurface,
};
use crate::application::service::planning::{PlanningControlCommand, PlanningResetTarget};

/*
 * Telegram 메시지 파서는 inbound adapter의 가장 얇은 명령 해석 경계다.
 * 이 파일은 Telegram이 넘긴 원문 문자열을 application service가 이해하는
 * PlanningControlCommand나 봇 로컬 명령으로만 바꾸고, 채팅방 허가 여부 확인,
 * planning 실행, 응답 렌더링은 상위 runner가 맡는다. 그래서 파서 결과는
 * "무시", "사용자에게 돌려줄 오류", "실행 가능한 명령" 세 갈래로 고정된다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TelegramInboundCommand {
    /*
     * `/whoami`는 planning workspace를 읽지 않는 Telegram adapter 전용 명령이다.
     * runner는 이 값을 받으면 현재 chat_id와 allowlist 판정만 렌더링한다.
     */
    WhoAmI,
    ParallelStatus,
    /*
     * 나머지 운영 명령은 application service의 planning control 언어로 넘긴다.
     * adapter가 service 내부 모델을 다시 만들지 않도록 여기서 바로 감싼다.
     */
    Planning(PlanningControlCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TelegramParsedMessage {
    /*
     * 일반 대화나 빈 메시지는 봇이 끼어들면 안 되므로 완전히 무시한다.
     * 특히 단체방에서는 slash command가 아닌 텍스트에 응답하지 않는 것이 중요하다.
     */
    Ignore,
    /*
     * 문법은 Telegram adapter가 가장 잘 알고 있으므로 사용법 오류도 여기서 만든다.
     * runner는 이 문자열을 그대로 답장해 실행 계층까지 잘못된 입력이 내려가지 않게 한다.
     */
    Error(String),
    /*
     * 검증을 통과한 명령만 이 변형으로 나간다. 이후 단계는 authorization과
     * planning service 실행에 집중할 수 있다.
     */
    Command(TelegramInboundCommand),
}

pub(super) fn parse_message(text: Option<&str>) -> TelegramParsedMessage {
    /*
     * Telegram update에는 text가 없거나 공백뿐인 메시지가 섞일 수 있다.
     * 여기서 None/empty를 먼저 접어야 runner가 파일, 스티커, 일반 채팅에 답하지 않는다.
     */
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return TelegramParsedMessage::Ignore;
    };

    /*
     * 첫 토큰만 명령어로 보고 나머지는 명령별 인자 파서로 넘긴다.
     * 공백 기준 분리는 현재 지원 명령이 모두 짧은 키워드형 인자만 받기 때문에 충분하다.
     */
    let mut parts = text.split_whitespace();
    let Some(raw_command) = parts.next() else {
        return TelegramParsedMessage::Ignore;
    };
    let command = normalize_command(raw_command);
    let arguments = parts.collect::<Vec<_>>();

    /*
     * 이 match가 Telegram 표면의 공개 명령표다. 여기서 application command로
     * 바로 축소해 두면 mod.rs의 runner는 같은 명령을 CLI나 다른 adapter와 공유하는
     * PlanningControlService 계약으로 실행할 수 있다.
     */
    match command.as_str() {
        /*
         * `/start`는 Telegram 사용자가 처음 봇을 열 때 자동으로 누르는 진입점이다.
         * 내부적으로는 planning help와 같으므로 별도 service 명령을 만들지 않는다.
         */
        "/start" | "/help" | "help" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Help),
            "/help",
        ),
        /*
         * `/whoami`는 사용자가 현재 chat_id를 알아 allowlist 설정을 고칠 수 있게 한다.
         * 보안상 planning 실행과 분리되며, 인자를 받지 않는다.
         */
        "/whoami" => {
            parse_command_without_arguments(&arguments, TelegramInboundCommand::WhoAmI, "/whoami")
        }
        "/parallel" | "parallel" => parse_parallel_arguments(&arguments),
        "/parallel_status" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::ParallelStatus,
            "/parallel_status",
        ),
        /*
         * `status` 별칭은 slash command가 아닌 짧은 운영 입력도 받아준다.
         * 단, 다른 일반 문장까지 해석하지 않도록 정확히 이 토큰일 때만 매칭한다.
         */
        "/status" | "status" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Status),
            "/status",
        ),
        /*
         * queue도 status와 같은 가벼운 조회 명령이다. 실행 권한 검사는 파서 뒤의
         * runner에서 수행되므로 여기서는 모양 검증만 담당한다.
         */
        "/queue" | "queue" => parse_command_without_arguments(
            &arguments,
            TelegramInboundCommand::Planning(PlanningControlCommand::Queue),
            "/queue",
        ),
        /*
         * `/plan`은 planning command namespace의 확장 지점이다. 지금은 status만 열어
         * 두지만, 하위 명령이 늘어나면 parse_plan_arguments에만 분기를 추가하면 된다.
         */
        "/plan" => parse_plan_arguments(&arguments),
        /*
         * reset alias들은 target이 명령어 이름에 이미 들어 있으므로 추가 인자를 금지한다.
         * 이 제한 덕분에 `/reset_all now` 같은 애매한 입력이 조용히 실행되지 않는다.
         */
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
        /*
         * `/reset`은 안전을 위해 target을 명시해야 한다. destructive command는 기본값을
         * 두지 않는 편이 낫기 때문에 빈 인자는 usage error로 돌린다.
         */
        "/reset" => parse_reset_arguments(&arguments),
        /*
         * slash로 시작하면 사용자가 봇 명령을 의도한 것이므로 침묵하지 않고 help를 붙인다.
         * help_text를 만들기 위해 service를 쓰지만, 아래 Noop surface가 실제 I/O 실행을
         * 막아 파서가 여전히 순수한 문자열 해석 경계로 남는다.
         */
        token if token.starts_with('/') => TelegramParsedMessage::Error(format!(
            "지원하지 않는 명령어입니다: {token}\n{}",
            PlanningControlService::new(Arc::new(NoopPlanningControlSurface)).help_text()
        )),
        /*
         * slash가 없는 나머지 텍스트는 일반 대화로 간주한다. 사용자가 그룹 채팅에서
         * 봇을 호출하지 않았을 때 불필요한 답장을 만들지 않기 위한 마지막 방어선이다.
         */
        _ => TelegramParsedMessage::Ignore,
    }
}

fn parse_command_without_arguments(
    arguments: &[&str],
    command: TelegramInboundCommand,
    usage: &'static str,
) -> TelegramParsedMessage {
    /*
     * 인자를 받지 않는 명령들의 공통 문지기다. 같은 usage 형식을 쓰게 하여
     * `/status now`, `/queue detail` 같은 오입력이 service까지 내려가지 않게 한다.
     */
    if arguments.is_empty() {
        TelegramParsedMessage::Command(command)
    } else {
        TelegramParsedMessage::Error(format!("사용법: {usage}"))
    }
}

fn parse_plan_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    /*
     * `/plan`은 Telegram에서 planning 영역을 열어 두는 namespace 역할을 한다.
     * 빈 인자와 명시적 status를 같은 조회 명령으로 해석해 사용자의 짧은 입력을 허용한다.
     */
    match arguments {
        [] | ["status"] => TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status,
        )),
        /*
         * 아직 지원하지 않는 하위 명령은 service로 보내지 않는다. 이곳의 usage가
         * 현재 Telegram 표면에서 노출된 planning 하위 명령의 단일 출처다.
         */
        _ => TelegramParsedMessage::Error("사용법: /plan [status]".to_string()),
    }
}

fn parse_parallel_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    match arguments {
        [] | ["status"] => TelegramParsedMessage::Command(TelegramInboundCommand::ParallelStatus),
        _ => TelegramParsedMessage::Error("사용법: /parallel [status]".to_string()),
    }
}

fn parse_reset_arguments(arguments: &[&str]) -> TelegramParsedMessage {
    /*
     * reset은 workspace 상태를 지우는 명령이므로 반드시 하나의 target만 허용한다.
     * target 누락과 여분 인자를 같은 usage error로 처리해 애매한 입력을 남기지 않는다.
     */
    let [target] = arguments else {
        return TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string(),
        );
    };

    /*
     * Telegram의 짧은 단어 target을 application service의 reset enum으로 매핑한다.
     * 이 변환을 adapter 안에 두면 service는 Telegram 명령어 표기법을 알 필요가 없다.
     */
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
    /*
     * Telegram 단체방에서는 `/status@BotName`처럼 봇 이름이 붙은 command가 들어온다.
     * 대소문자를 낮추고 mention 접미사를 잘라 같은 명령표로 처리한다.
     */
    let lowered = raw_command.to_ascii_lowercase();
    let mut parts = lowered.split('@');
    parts.next().unwrap_or_default().to_string()
}

/*
 * 이 surface는 unsupported slash command의 help text를 렌더링하기 위한 placeholder다.
 * PlanningControlService::help_text는 상태 조회나 reset을 호출하지 않으므로 정상 경로에서는
 * 아래 메서드들이 실행되지 않는다. 실행되면 파서가 자기 책임을 넘어섰다는 뜻이므로 즉시 실패한다.
 */
struct NoopPlanningControlSurface;

impl PlanningControlSurface for NoopPlanningControlSurface {
    fn workspace_dir(&self) -> &str {
        ""
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        bail!("noop control surface should not execute");
    }

    fn reset_workspace(&self, _target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        bail!("noop control surface should not execute");
    }
}
