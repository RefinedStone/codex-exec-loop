# Planning Runtime Engagement Bug Note

Date: `2026-04-10`

This note supersedes the earlier manual-prompt drift report.

The original manual-submit mismatch has already been repaired. The remaining operator-visible gap is narrower and is now about the queue-empty behavior of builtin `next-task`.

## Current Observed Behavior

1. `:planning` simple mode can create and promote valid planning files.
2. The first manual user prompt after that now runs through planning-aware prompt assembly.
3. If the selected auto-follow template is builtin `next-task` and planning is valid but the queue has no actionable `next_task`, the runtime queues a planning refresh prompt instead of blocking.

## Impact

- planning engagement is now consistent across manual and auto prompt origins
- the remaining ambiguity is semantic:
  - an operator may read builtin `next-task` as "execute the current queue head only"
  - the runtime currently allows it to mean "refresh or derive queue work from the latest answer when the queue is empty"

## Minimal Repro

1. Run `:planning`.
2. Choose simple mode and promote the staged scaffold.
3. Submit a normal manual prompt and confirm that planning context is attached.
4. Let builtin `next-task` auto follow-up evaluate while the planning runtime is still `ReadyNoTask`.
5. Observe that the runtime queues a planning refresh prompt instead of pausing with a queue-empty block.

## Expected Behavior

The system should pick one contract and keep code, UI copy, and docs aligned:

- strict contract: builtin `next-task` blocks unless planning is `ReadyWithTask`
- refresh contract: builtin `next-task` is explicitly documented as "next task or refresh queue from latest answer when needed"
