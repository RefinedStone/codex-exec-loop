use super::*;

#[test]
fn gateway_kernel_client_opens_saved_history_from_the_selected_session() {
    use crate::composition::gateway_kernel_client_runtime::fixture::conversation;
    use crate::composition::gateway_kernel_client_runtime::tests::kernel_client_with_history;
    use serde_json::json;

    let client = kernel_client_with_history(
        "/repo",
        vec![json!({"type":"sessions","status":"ready","sessions":[
            {"thread_id":"thread-1","title":"Saved history for thread-1","workspace":"/repo","updated_at_epoch":7}
        ]})],
        vec![("thread-1".into(), conversation("thread-1"))],
    );
    let mut app = NativeTuiApp::new_gateway_kernel_client(Box::new(client));
    app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested);
    app.run_client_runtime_host_cycle(std::time::Instant::now(), 16);
    app.toggle_session_overlay();
    app.open_conversation_shell();
    let outcomes = app.runtime.client_runtime.run_host_cycle(
        std::time::Instant::now(),
        std::time::Duration::from_secs(2),
        1,
    );
    assert_eq!(outcomes.len(), 1);
    for outcome in outcomes {
        let _ = app.apply_native_client_dispatch_outcome(outcome);
    }

    let frame = render(&mut app, 100, 24);
    println!("--- saved history through the Gateway client ---\n{frame}");
    assert!(frame.contains("gateway-history-message"), "{frame}");
    let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
    else {
        panic!("the selected conversation must be visible");
    };
    assert_eq!(conversation.thread_id, "thread-1");
    assert_eq!(conversation.cwd, "/repo");
}

/*
 * plan 18 K6-2a: the TUI attached to the Gateway kernel draws the kernel's sessions and thread activity, and
 * refuses the screens and actions the kernel protocol does not carry yet with the reason. The printed frames
 * are the capture of the terminal validation contract for this slice (`--nocapture`).
 */
#[test]
fn gateway_kernel_client_draws_kernel_sessions_and_refuses_locked_screens_with_the_reason() {
    use crate::composition::gateway_kernel_client_runtime::tests::kernel_client_with_values;
    use serde_json::json;

    let client = kernel_client_with_values(
        "/repo",
        vec![
            json!({"type":"sessions","status":"ready","sessions":[
                {"thread_id":"thread-1","title":"Fix the CI cache","workspace":"/repo","updated_at_epoch":1700000000}
            ]}),
            json!({"type":"auto_follow","max_turns":5,"paused":false}),
            json!({"type":"thread_activity","threads":[
                {"thread_id":"thread-1","running":true,"active_turn_id":"turn-1","auto_follow_active":false}
            ]}),
        ],
    );
    let mut app = NativeTuiApp::new_gateway_kernel_client(Box::new(client));
    app.dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested);
    app.run_client_runtime_host_cycle(std::time::Instant::now(), 16);
    assert!(app.shell_action_availability().allows_actions());
    assert_eq!(
        app.runtime
            .client_runtime
            .snapshot()
            .conversation_runtime
            .auto_follow
            .max_auto_turns,
        5
    );

    for character in ":qu".chars() {
        app.push_input_character(character);
    }
    let palette = render(&mut app, 100, 24);
    println!("--- palette in the Gateway kernel client ---\n{palette}");
    assert!(palette.contains("LOCKED"), "{palette}");
    app.clear_input_buffer();

    ready_conversation_mut(&mut app).composer.input_buffer = ":queue".to_string();
    app.start_turn_submission();
    let locked = render(&mut app, 100, 24);
    println!("--- :queue in the Gateway kernel client ---\n{locked}");
    assert!(
        locked.contains("LOCKED; not in the kernel protocol"),
        "the locked screen shows its reason:\n{locked}"
    );
    assert_ne!(app.shell.chrome.shell_overlay, ShellOverlay::Queue);

    ready_conversation_mut(&mut app).composer.input_buffer = "fix the build".to_string();
    app.start_turn_submission();
    let refused = render(&mut app, 100, 24);
    println!("--- prompt in the Gateway kernel client ---\n{refused}");
    assert!(
        refused.contains("read-only Gateway kernel client"),
        "{refused}"
    );

    app.toggle_session_overlay();
    let sessions = render(&mut app, 100, 24);
    println!("--- sessions from the Gateway kernel ---\n{sessions}");
    assert!(sessions.contains("Fix the CI cache"), "{sessions}");
}
