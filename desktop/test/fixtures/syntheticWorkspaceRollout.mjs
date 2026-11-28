// Minimal saved history for an otherwise unmaterialized thread/start. No live turns.
export function syntheticWorkspaceRollout(thread) {
  const timestamp = new Date(thread.createdAt * 1000).toISOString();
  const line = (type, payload) => JSON.stringify({ timestamp, type, payload });
  return [
    line("session_meta", {
      id: thread.id, session_id: thread.id, timestamp, cwd: thread.cwd,
      originator: "codex", cli_version: "0.171.0", source: "cli", model_provider: "openai",
    }),
    line("event_msg", { type: "task_started", turn_id: "fixture-turn", model_context_window: null }),
    line("response_item", { type: "message", role: "user", content: [{ type: "input_text", text: "Synthetic question" }] }),
    line("event_msg", { type: "user_message", message: "Synthetic question", images: [], local_images: [] }),
    line("event_msg", { type: "task_complete", turn_id: "fixture-turn", last_agent_message: null }),
    "",
  ].join("\n");
}
