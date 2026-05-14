use super::config::{
    TelegramBotEnvironment, apply_environment_file, load_environment_from_sources,
    parse_args_with_environment,
};
use super::{
    TelegramBotPolicy, TelegramBotRunner, TelegramInboundCommand, TelegramParallelControlSurface,
    TelegramParsedMessage, parse_message,
};
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotPort, TelegramInboundMessage, TelegramPollRequest, TelegramSendMessageRequest,
    TelegramUpdate,
};
use crate::application::service::planning::PlanningResetTarget;
use crate::application::service::planning::control::{
    PlanningControlCommand, PlanningControlQueueEntry, PlanningControlResetOutcome,
    PlanningControlService, PlanningControlStatusSnapshot, PlanningControlSurface,
};
use anyhow::{Result, anyhow, bail};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/*
 * These tests pin the Telegram adapter as a thin remote-control surface over
 * planning control. The fixtures avoid network access and the real planning
 * store, but they preserve the same command parser, allowlist policy, poll-loop
 * resilience, and Korean operator copy that production chat users see.
 */
struct FakePlanningControlSurface;

impl PlanningControlSurface for FakePlanningControlSurface {
    fn workspace_dir(&self) -> &str {
        "/tmp/repo"
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        /*
         * Keep this snapshot intentionally small but not empty. Telegram status
         * replies are read outside the TUI, so the queue head title is the
         * easiest durable signal that the inbound adapter reached the shared
         * planning-control service instead of rendering a Telegram-only stub.
         */
        Ok(PlanningControlStatusSnapshot {
            workspace_dir: "/tmp/repo".to_string(),
            planning_state: "ready".to_string(),
            task_authority_signature: Some(42),
            queue_head_task_signature: Some(7),
            queue_summary: Some("queue head ready".to_string()),
            proposal_summary: None,
            health: Some("planning workspace ready".to_string()),
            issue: None,
            note: None,
            preview_status_label: "queue ready".to_string(),
            preview_detail: None,
            queue_head: Some(PlanningControlQueueEntry {
                task_id: "task-1".to_string(),
                task_title: "Ship Telegram control".to_string(),
                direction_id: "general-workstream".to_string(),
                status: "ready".to_string(),
                combined_priority: 90,
            }),
            visible_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
        })
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        /*
         * Reset tests do not need file IO; they need proof that Telegram target
         * words have already been mapped into the same PlanningResetTarget labels
         * used by admin and TUI control surfaces.
         */
        Ok(PlanningControlResetOutcome {
            target: target.label().to_string(),
            rewritten_paths: vec!["DB task authority".to_string()],
            removed_paths: Vec::new(),
            planning_state: "ready".to_string(),
            health: Some("queue reset complete".to_string()),
            issue: None,
        })
    }
}

struct FlakyPlanningControlSurface {
    load_calls: AtomicUsize,
}

impl PlanningControlSurface for FlakyPlanningControlSurface {
    fn workspace_dir(&self) -> &str {
        "/tmp/repo"
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        let call = self.load_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            /*
             * The first status call simulates a service-layer failure after
             * parsing and authorization have already succeeded. The runner must
             * convert that into one failed reply without poisoning the next
             * update in the same Telegram batch.
             */
            bail!("temporary planning failure");
        }
        FakePlanningControlSurface.load_status_snapshot()
    }
    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        FakePlanningControlSurface.reset_workspace(target)
    }
}

struct FakeTelegramParallelControlSurface;

impl TelegramParallelControlSurface for FakeTelegramParallelControlSurface {
    fn render_parallel_status(&self) -> Result<String> {
        Ok("병렬 상태\nreadiness: ready\npool: supervised\nactive_agents: 1\nqueue_depth: 2\nevents: 3".to_string())
    }
}

#[derive(Default)]
struct FakeTelegramBotPort {
    /*
     * Stored in reverse-pop order so each test can script a poll transcript
     * without an async runtime or real Telegram HTTP state. That keeps offset
     * assertions focused on runner behavior rather than mock bookkeeping.
     */
    poll_errors: Mutex<Vec<anyhow::Error>>,
    updates: Mutex<Vec<Vec<TelegramUpdate>>>,
    // Captured send requests are the observable side effect for runner tests.
    sent_messages: Mutex<Vec<TelegramSendMessageRequest>>,
}

impl TelegramBotPort for FakeTelegramBotPort {
    fn get_updates(&self, _request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
        if let Some(error) = self
            .poll_errors
            .lock()
            .expect("poll error mutex should lock")
            .pop()
        {
            return Err(error);
        }
        Ok(self
            .updates
            .lock()
            .expect("updates mutex should lock")
            .pop()
            .unwrap_or_default())
    }
    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()> {
        self.sent_messages
            .lock()
            .expect("sent messages mutex should lock")
            .push(request.clone());
        Ok(())
    }
}

fn build_runner(allowed_chat_ids: &[i64]) -> (Arc<FakeTelegramBotPort>, TelegramBotRunner) {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        PlanningControlService::new(Arc::new(FakePlanningControlSurface)),
        Arc::new(FakeTelegramParallelControlSurface),
        TelegramBotPolicy::new(allowed_chat_ids.iter().copied().collect()),
        /*
         * A limit of one mirrors the narrowest long-poll batch. The runner still
         * uses production offset logic, but tests can tell whether a retry held
         * or advanced the cursor after exactly one update.
         */
        1,
        false,
        Duration::ZERO,
    );
    (gateway, runner)
}

// Parser tests protect the user-facing chat grammar before service dispatch is involved.
#[test]
fn parse_message_accepts_plan_status_command_with_bot_mention() {
    /*
     * Group chats append the bot username to slash commands. This case protects
     * the normalization step that strips `@AkraBot` before the adapter maps
     * `/plan status` onto the shared planning Status command.
     */
    let parsed = parse_message(Some("/plan@AkraBot status"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status
        ))
    );
}

#[test]
fn parse_message_maps_supported_planning_commands_to_shared_control_enum() {
    /*
     * Telegram command spelling is transport vocabulary. Planning operations
     * must leave the parser as PlanningControlCommand so CLI and Telegram keep
     * the same application command surface.
     */
    for (raw, expected) in [
        ("/start", PlanningControlCommand::Help),
        ("/help", PlanningControlCommand::Help),
        ("help", PlanningControlCommand::Help),
        ("/status", PlanningControlCommand::Status),
        ("status", PlanningControlCommand::Status),
        ("/queue", PlanningControlCommand::Queue),
        ("queue", PlanningControlCommand::Queue),
        ("/plan", PlanningControlCommand::Status),
        ("/plan status", PlanningControlCommand::Status),
        (
            "/reset queue",
            PlanningControlCommand::Reset(PlanningResetTarget::Queue),
        ),
        (
            "/reset_directions",
            PlanningControlCommand::Reset(PlanningResetTarget::Directions),
        ),
        (
            "/reset_all",
            PlanningControlCommand::Reset(PlanningResetTarget::All),
        ),
    ] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Command(TelegramInboundCommand::Planning(expected)),
            "Telegram input `{raw}` should map to the shared planning control command"
        );
    }
}

#[test]
fn parse_message_maps_parallel_status_to_parallel_control_surface() {
    for raw in [
        "/parallel",
        "parallel",
        "/parallel status",
        "/parallel_status",
    ] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Command(TelegramInboundCommand::ParallelStatus),
            "Telegram input `{raw}` should map to the shared parallel control surface"
        );
    }
    assert_eq!(
        parse_message(Some("/parallel now")),
        TelegramParsedMessage::Error("사용법: /parallel [status]".to_string())
    );
}

#[test]
fn parse_message_ignores_empty_and_plain_chat_text() {
    for raw in [None, Some(""), Some("   "), Some("hello akra")] {
        assert_eq!(parse_message(raw), TelegramParsedMessage::Ignore);
    }
}

#[test]
fn parse_message_reports_help_for_unknown_slash_command() {
    let parsed = parse_message(Some("/deploy"));

    match parsed {
        TelegramParsedMessage::Error(error) => {
            assert!(error.contains("지원하지 않는 명령어입니다: /deploy"));
            assert!(error.contains("/status"));
            assert!(error.contains("/reset queue"));
        }
        other => panic!("unknown slash command should produce help error, got {other:?}"),
    }
}

#[test]
fn parse_message_rejects_extra_arguments_for_query_commands() {
    for (raw, usage) in [
        ("/help now", "/help"),
        ("/whoami please", "/whoami"),
        ("/status detail", "/status"),
        ("/queue all", "/queue"),
        ("/parallel_status now", "/parallel_status"),
    ] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Error(format!("사용법: {usage}")),
            "Telegram input `{raw}` should reject trailing arguments"
        );
    }
}

#[test]
fn parse_message_rejects_unknown_plan_and_reset_targets() {
    assert_eq!(
        parse_message(Some("/plan repair")),
        TelegramParsedMessage::Error("사용법: /plan [status]".to_string())
    );
    assert_eq!(
        parse_message(Some("/reset cache")),
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_reports_usage_for_reset_without_target() {
    /*
     * `/reset` is destructive enough that Telegram must reject an omitted target
     * at the parser boundary. The application service should never receive a
     * best-guess reset command from ambiguous chat text.
     */
    let parsed = parse_message(Some("/reset"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_rejects_reset_with_extra_arguments() {
    /*
     * Extra words after a reset target often mean the operator thought another
     * scope or confirmation was available. Returning usage text is safer than
     * silently accepting a partial destructive command.
     */
    let parsed = parse_message(Some("/reset queue now"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_rejects_reset_alias_with_extra_arguments() {
    /*
     * Alias commands skip the generic `/reset <target>` parser path, so this
     * regression case keeps shorthand reset commands equally strict about
     * accepting no trailing chat text.
     */
    let parsed = parse_message(Some("/reset_queue now"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error("사용법: /reset_queue".to_string())
    );
}

// Allowlist tests are security-sensitive: unauthorized chats must receive setup guidance, not data.
#[test]
fn runner_rejects_unauthorized_chat_with_current_chat_id() {
    /*
     * An empty allowlist is treated as "not configured", not "allow everyone".
     * The reply must reveal only the current chat id and environment key so an
     * operator can complete setup without leaking workspace status or queue data.
     */
    let (_gateway, runner) = build_runner(&[]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("현재 chat_id: 777"));
    assert!(reply.contains("AKRA_TELEGRAM_ALLOWED_CHAT_IDS"));
}

#[test]
fn runner_executes_planning_command_for_allowed_chat() {
    /*
     * The queue title assertion proves the allowed path crosses the adapter
     * boundary and executes PlanningControlService. Checking only a generic
     * heading would miss a regression that returned static Telegram help text.
     */
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("상태 요약"));
    assert!(reply.contains("Ship Telegram control"));
}

#[test]
fn runner_executes_parallel_status_for_allowed_chat() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            text: Some("/parallel".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("병렬 상태"));
    assert!(reply.contains("queue_depth: 2"));
}

#[test]
fn help_reply_mentions_whoami_without_allowlist() {
    /*
     * Help remains open because it is the bootstrap surface for remote setup.
     * `/whoami` must be visible here so an operator can discover the exact chat
     * id before any privileged planning command is accepted.
     */
    let (_gateway, runner) = build_runner(&[]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            text: Some("/help".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("/whoami"));
    assert!(reply.contains("/parallel"));
    assert!(reply.contains("/status"));
}

// Config tests cover precedence: process env overrides file values, flags add explicit chat IDs.
#[test]
fn parse_args_reads_token_and_chat_ids_from_environment_and_flags() {
    /*
     * CLI chat ids are additive so a one-off operator can be allowed without
     * rewriting the local env file. The token still comes from the environment
     * because the flag parser should not require secrets in shell history.
     */
    let args = parse_args_with_environment(
        [
            "--allow-chat-id".to_string(),
            "12".to_string(),
            "--poll-timeout-seconds".to_string(),
            "45".to_string(),
        ],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: [10, 11].into_iter().collect(),
        },
    )
    .expect("args should parse");

    assert_eq!(args.token, "env-token");
    assert_eq!(args.allowed_chat_ids.len(), 3);
    assert!(args.allowed_chat_ids.contains(&10));
    assert!(args.allowed_chat_ids.contains(&12));
    assert_eq!(args.poll_timeout_seconds, 45);
    assert!(args.drop_pending_updates);
}

#[test]
fn parse_args_cli_token_overrides_environment_and_keep_pending_disables_drop() {
    let args = parse_args_with_environment(
        [
            "--token".to_string(),
            "cli-token".to_string(),
            "--allow-chat-id".to_string(),
            "-100".to_string(),
            "--keep-pending".to_string(),
        ],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: [10].into_iter().collect(),
        },
    )
    .expect("args should parse");

    assert_eq!(args.token, "cli-token");
    assert_eq!(args.allowed_chat_ids, [-100, 10].into_iter().collect());
    assert_eq!(args.poll_timeout_seconds, 30);
    assert!(!args.drop_pending_updates);
}

#[test]
fn parse_args_rejects_missing_invalid_and_unsupported_values() {
    for (args, expected_error) in [
        (
            vec!["--token".to_string()],
            "missing value for --token".to_string(),
        ),
        (
            vec!["--allow-chat-id".to_string()],
            "missing value for --allow-chat-id".to_string(),
        ),
        (
            vec!["--allow-chat-id".to_string(), "not-a-chat".to_string()],
            "failed to parse telegram chat id from `not-a-chat`".to_string(),
        ),
        (
            vec!["--poll-timeout-seconds".to_string(), "abc".to_string()],
            "failed to parse poll timeout seconds from `abc`".to_string(),
        ),
        (
            vec!["--poll-timeout-seconds".to_string(), "0".to_string()],
            "--poll-timeout-seconds must be greater than zero".to_string(),
        ),
        (
            vec!["--unknown".to_string()],
            "unsupported telegram-bot argument: --unknown".to_string(),
        ),
    ] {
        let error = parse_args_with_environment(
            args,
            TelegramBotEnvironment {
                token: Some("env-token".to_string()),
                allowed_chat_ids: BTreeSet::new(),
            },
        )
        .expect_err("args should fail");
        assert!(
            error.to_string().contains(&expected_error),
            "expected `{expected_error}`, got `{error:#}`"
        );
    }

    let missing_token = parse_args_with_environment(Vec::<String>::new(), Default::default())
        .expect_err("missing token should fail");
    assert!(
        missing_token
            .to_string()
            .contains("telegram bot token is required")
    );
}

#[test]
fn load_environment_from_sources_merges_config_file_and_process_env() {
    /*
     * Process env wins over config file content for both token and allowlist.
     * That lets deployment wrappers rotate credentials or narrow access without
     * editing the user's persistent Telegram config file.
     */
    let environment = load_environment_from_sources(
        Some(
            r#"
            AKRA_TELEGRAM_BOT_TOKEN=config-token
            AKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,11
            "#,
        ),
        Some("env-token".to_string()),
        Some("12,13".to_string()),
    )
    .expect("environment should load");

    assert_eq!(environment.token.as_deref(), Some("env-token"));
    assert_eq!(environment.allowed_chat_ids, [12, 13].into_iter().collect());
}

#[test]
fn load_environment_from_sources_accepts_empty_and_sparse_allowlists() {
    let environment = load_environment_from_sources(
        Some("AKRA_TELEGRAM_BOT_TOKEN=config-token\nAKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,, -20,\n"),
        None,
        None,
    )
    .expect("environment should load");

    assert_eq!(environment.token.as_deref(), Some("config-token"));
    assert_eq!(
        environment.allowed_chat_ids,
        [-20, 10].into_iter().collect()
    );

    let environment = load_environment_from_sources(None, None, Some("".to_string()))
        .expect("empty process allowlist should load");
    assert!(environment.allowed_chat_ids.is_empty());
}

#[test]
fn apply_environment_file_reads_token_and_allowlist() {
    let mut environment = TelegramBotEnvironment::default();

    /*
     * The file parser accepts shell-like `export` and quoted values because the
     * default config path is meant to be hand-edited. Unknown keys stay ignored
     * so users can keep notes or future settings in the same file.
     */
    apply_environment_file(
        &mut environment,
        r#"
        # local bot config
        export AKRA_TELEGRAM_BOT_TOKEN="stored-token"
        AKRA_TELEGRAM_ALLOWED_CHAT_IDS='10,11'
        UNUSED_KEY=ignored
        "#,
    )
    .expect("config file should parse");

    assert_eq!(environment.token.as_deref(), Some("stored-token"));
    assert_eq!(environment.allowed_chat_ids, [10, 11].into_iter().collect());
}

#[test]
fn apply_environment_file_reports_malformed_lines_and_bad_chat_ids() {
    let mut environment = TelegramBotEnvironment::default();
    let malformed = apply_environment_file(&mut environment, "AKRA_TELEGRAM_BOT_TOKEN");
    assert!(
        malformed
            .expect_err("missing equals should fail")
            .to_string()
            .contains("invalid Telegram config entry on line 1")
    );

    let bad_chat_id =
        apply_environment_file(&mut environment, "AKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,abc");
    assert!(
        bad_chat_id
            .expect_err("bad chat id should fail")
            .to_string()
            .contains("failed to parse telegram chat id from `abc`")
    );
}

// Poll-loop tests keep the bot alive across transport and per-message failures.
#[test]
fn run_poll_cycle_keeps_loop_alive_after_poll_error() {
    /*
     * A failed getUpdates call cannot advance the cursor; otherwise Telegram
     * could drop a command the bot never saw. Returning the previous offset is
     * the retry contract for transient network or API errors.
     */
    let (gateway, runner) = build_runner(&[42]);
    gateway
        .poll_errors
        .lock()
        .expect("poll error mutex should lock")
        .push(anyhow!("network unavailable"));
    let next_offset = runner.run_poll_cycle(Some(99));

    assert_eq!(next_offset, Some(99));
}

#[test]
fn process_updates_continues_after_individual_message_failure() {
    /*
     * Batch processing isolates each message. The first update exercises the
     * failure reply path, and the second proves the runner keeps draining the
     * batch instead of letting one bad planning call block later chat commands.
     */
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        PlanningControlService::new(Arc::new(FlakyPlanningControlSurface {
            load_calls: AtomicUsize::new(0),
        })),
        Arc::new(FakeTelegramParallelControlSurface),
        TelegramBotPolicy::new([42].into_iter().collect()),
        1,
        false,
        Duration::ZERO,
    );

    runner.process_updates(&[
        // Same chat and same command isolate the variable to service call order.
        TelegramUpdate {
            update_id: 1,
            message: Some(TelegramInboundMessage {
                message_id: 10,
                chat_id: 42,
                text: Some("/status".to_string()),
                sender_display_name: None,
            }),
        },
        TelegramUpdate {
            update_id: 2,
            message: Some(TelegramInboundMessage {
                message_id: 11,
                chat_id: 42,
                text: Some("/status".to_string()),
                sender_display_name: None,
            }),
        },
    ]);
    let sent_messages = gateway
        .sent_messages
        .lock()
        .expect("sent messages mutex should lock");
    assert_eq!(sent_messages.len(), 2);
    // First message reports the injected planning failure; second proves the loop recovered.
    assert!(sent_messages[0].text.contains("명령 처리에 실패했습니다."));
    assert!(sent_messages[1].text.contains("상태 요약"));
}

#[test]
fn runner_whoami_reports_allowlist_state_without_planning_access() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            text: Some("/whoami".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed")
        .expect("reply should exist");

    assert!(reply.contains("chat_id: 42"));
    assert!(reply.contains("allowed: yes"));
    assert!(reply.contains("allowlist_configured: yes"));
}

#[test]
fn runner_rejects_unauthorized_chat_against_configured_allowlist() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            text: Some("/parallel".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed")
        .expect("reply should exist");

    assert!(reply.contains("허용되지 않은 chat_id입니다."));
    assert!(reply.contains("현재 chat_id: 777"));
}
