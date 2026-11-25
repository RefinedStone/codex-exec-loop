# Result Output Prompt

- Summarize the operator-visible improvement that actually shipped in this turn.
- Call out which shell, queue, planning, session, or validation surfaces changed.
- If DB task authority changed, list which task ids moved between `ready`, `in_progress`, `done`, `blocked`, `awaiting_user`, or `proposed` and why.
- Include only verification that was actually run. Do not claim real-terminal validation unless it happened in this turn.
- If the work uncovers concrete alternate slices, encode them as `proposed` tasks instead of leaving them only in prose.
- Keep the final answer grounded in shipped behavior, residual risks, and the next most likely follow-up.
