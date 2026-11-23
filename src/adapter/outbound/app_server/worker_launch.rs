use super::*;
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerStartAuthorization;

pub(super) fn run(
    adapter: &CodexAppServerAdapter,
    request: ParallelAgentWorkerStreamRequest<'_>,
    event_sender: ConversationStreamSender,
    start_authorization: Option<&dyn ParallelAgentWorkerStartAuthorization>,
) -> Result<ConversationTurnTerminalReceipt> {
    // Parallel worker sessions use isolated processes but persist app-server threads so `:peek` can read them later.
    let result = adapter.with_isolated_streaming_runtime(|connection| {
        let mut workspace = protected_thread_workspace(request.cwd)?;
        let evidence_root = worker_evidence::WorkerEvidenceRoot::prepare(
            request.evidence_incoming_root,
            adapter.execution_policy.sandbox_mode,
        )?;
        if let Some(root) = &evidence_root {
            root.apply_config(&mut workspace)?;
        }
        let requested_cwd = workspace.cwd.clone();
        let mut thread_request = runtime_configuration_request(
            None,
            None,
            Some(&requested_cwd),
            Some(adapter.execution_policy.approval_policy),
            adapter.execution_policy.approvals_reviewer,
            Some(adapter.execution_policy.sandbox_mode),
        );
        if let Some(root) = &evidence_root {
            root.record_request(&mut thread_request);
        }
        let thread_response = connection.start_thread(ThreadStartParams {
            cwd: Some(workspace.cwd),
            approval_policy: Some(adapter.execution_policy.approval_policy),
            approvals_reviewer: adapter.execution_policy.approvals_reviewer,
            sandbox: Some(adapter.execution_policy.sandbox_mode),
            config: Some(workspace.config),
            model: None,
            developer_instructions: Some(request.developer_instructions.to_string()),
            service_name: Some(request.service_name.to_string()),
            ephemeral: Some(false),
        })?;
        let thread_id = thread_response.thread.id.clone();
        if thread_id.is_empty() {
            anyhow::bail!("thread/start response omitted a nonempty thread id");
        }
        let runtime_envelope = to_runtime_envelope(
            thread_request,
            &thread_response.runtime_envelope_fields,
            &thread_response.thread,
            adapter.connection_config.runtime_launch_environment(),
        )?;
        connection.discard_notifications_before_turn_binding();
        let applied_cwd =
            exact_applied_workspace_cwd(&runtime_envelope, &requested_cwd, "thread/start")?;
        if let Some(root) = &evidence_root {
            root.verify_applied(&runtime_envelope, &requested_cwd)?;
        }
        send_required_app_server_event(
            &event_sender,
            ConversationStreamEvent::codex_app_server_launch_attachment(),
            "attachment/launch",
        )?;
        send_required_app_server_event(
            &event_sender,
            ConversationStreamEvent::ThreadPrepared {
                thread_id: thread_id.clone(),
                title: thread_title(&thread_response.thread),
                cwd: applied_cwd,
                runtime_envelope: Box::new(runtime_envelope),
            },
            "thread/prepared",
        )?;

        let stream_result = adapter.start_worker_turn_and_wait_for_stream(
            connection,
            request.prompt,
            &event_sender,
            AppServerPromptTraceContext {
                workspace_dir: request.cwd.to_string(),
                session_kind: "parallel-worker".to_string(),
                operation: "isolated_parallel_thread".to_string(),
                service_name: Some(request.service_name.to_string()),
                developer_instructions: Some(request.developer_instructions.to_string()),
                thread_id: thread_id.clone(),
            },
            evidence_root.as_ref(),
            start_authorization,
        );
        // The application owns completion and may still need this thread for AwaitingUser.
        stream_result
    });

    finish_stream_result(result, &event_sender)
}
