//! The isolated answer path reattaches only to the persisted thread and cwd.
use super::*;
use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerResumeRequest, ParallelAgentWorkerStartAuthorization,
};
use std::collections::BTreeSet;

const WORKER_THREAD_SOURCES: &[&str] = &[
    "cli",
    "vscode",
    "exec",
    "appServer",
    "subAgent",
    "subAgentReview",
    "subAgentCompact",
    "subAgentThreadSpawn",
    "subAgentOther",
    "unknown",
];

fn exact_thread(thread: &protocol::ThreadRecord, thread_id: &str, cwd: &str) -> Result<()> {
    if thread.id != thread_id || thread.cwd != cwd {
        anyhow::bail!("isolated continuation thread identity or cwd changed");
    }
    Ok(())
}

fn is_archived(
    connection: &mut connection::AppServerConnection,
    thread_id: &str,
    cwd: &str,
) -> Result<bool> {
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    // A bounded listing failure is uncertainty, never permission to create another thread.
    for _ in 0..64 {
        let page = connection.list_threads(ThreadListParams {
            archived: Some(true),
            model_providers: Some(Vec::new()),
            cwd: Some(cwd.into()),
            limit: Some(100),
            cursor: cursor.clone(),
            source_kinds: Some(
                WORKER_THREAD_SOURCES
                    .iter()
                    .map(|kind| (*kind).to_string())
                    .collect(),
            ),
            ..ThreadListParams::default()
        })?;
        for thread in &page.data {
            if thread.id == thread_id {
                exact_thread(thread, thread_id, cwd)?;
                return Ok(true);
            }
        }
        match page.next_cursor {
            None => return Ok(false),
            Some(next) if !next.is_empty() && seen.insert(next.clone()) => cursor = Some(next),
            Some(_) => anyhow::bail!("archived worker listing did not advance"),
        }
    }
    anyhow::bail!("archived worker listing exceeded its bounded search")
}

pub(super) fn resume(
    adapter: &CodexAppServerAdapter,
    request: ParallelAgentWorkerResumeRequest<'_>,
    event_sender: ConversationStreamSender,
    start_authorization: &dyn ParallelAgentWorkerStartAuthorization,
) -> Result<ConversationTurnTerminalReceipt> {
    let result = adapter.with_isolated_streaming_runtime(|connection| {
        let workspace = protected_thread_workspace(request.cwd)?;
        if workspace.cwd != request.cwd || request.thread_id.trim().is_empty() {
            anyhow::bail!("isolated continuation requires its exact canonical cwd and thread");
        }
        let requested_cwd = workspace.cwd.clone();
        let read = connection.read_thread(request.thread_id, false)?;
        exact_thread(&read.thread, request.thread_id, &requested_cwd)?;
        if is_archived(connection, request.thread_id, &requested_cwd)? {
            let restored = connection.unarchive_thread(request.thread_id)?;
            exact_thread(&restored.thread, request.thread_id, &requested_cwd)?;
        }
        let thread_request = runtime_configuration_request(
            None,
            None,
            Some(&requested_cwd),
            Some(adapter.execution_policy.approval_policy),
            adapter.execution_policy.approvals_reviewer,
            Some(adapter.execution_policy.sandbox_mode),
        );
        let resumed = connection.resume_thread(ThreadResumeParams {
            thread_id: request.thread_id.into(),
            cwd: Some(workspace.cwd),
            approval_policy: Some(adapter.execution_policy.approval_policy),
            approvals_reviewer: adapter.execution_policy.approvals_reviewer,
            sandbox: Some(adapter.execution_policy.sandbox_mode),
            config: Some(workspace.config),
            model: None,
        })?;
        exact_thread(&resumed.thread, request.thread_id, &requested_cwd)?;
        let envelope = to_runtime_envelope(
            thread_request,
            &resumed.runtime_envelope_fields,
            &resumed.thread,
            adapter.connection_config.runtime_launch_environment(),
        )?;
        let applied_cwd = exact_applied_workspace_cwd(&envelope, &requested_cwd, "thread/resume")?;
        connection.discard_notifications_before_turn_binding();
        send_required_app_server_event(
            &event_sender,
            ConversationStreamEvent::codex_app_server_reattach_attachment(),
            "attachment/reattach",
        )?;
        send_required_app_server_event(
            &event_sender,
            ConversationStreamEvent::ThreadPrepared {
                thread_id: request.thread_id.into(),
                title: thread_title(&resumed.thread),
                cwd: applied_cwd.clone(),
                runtime_envelope: Box::new(envelope),
            },
            "thread/prepared",
        )?;
        start_authorization.validate()?;
        // This is the only turn/start call in the continuation path. Neither transport failure
        // nor missing ACK is retried; the planning authority already owns an Unknown receipt.
        adapter.start_turn_and_wait_for_stream(
            connection,
            vec![TurnInputItem::text(request.prompt)],
            None,
            None,
            &event_sender,
            AppServerPromptTraceContext {
                workspace_dir: applied_cwd,
                session_kind: "parallel-worker".into(),
                operation: "isolated_parallel_continuation".into(),
                service_name: Some(request.service_name.into()),
                developer_instructions: None,
                thread_id: request.thread_id.into(),
            },
        )
    });
    finish_stream_result(result, &event_sender)
}

pub(super) fn archive(adapter: &CodexAppServerAdapter, cwd: &str, thread_id: &str) -> Result<()> {
    adapter.with_isolated_streaming_runtime(|connection| {
        let thread = connection.read_thread(thread_id, false)?.thread;
        exact_thread(&thread, thread_id, cwd)?;
        connection.archive_thread(thread_id)
    })
}
