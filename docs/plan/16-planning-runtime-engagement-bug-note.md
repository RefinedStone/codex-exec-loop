# Planning Runtime Engagement Bug Note

Date: `2026-04-10`

## Observed Behavior

1. `:planning` simple mode can create and promote valid planning files.
2. The first manual user prompt after that still runs without planning-aware prompt assembly.
3. The next builtin auto follow-up prompt can append `Planning Context` even when `Queue Summary` reports `next_task: none`.

## Impact

- the operator sees planning as initialized, but the first real turn is still unmanaged
- builtin next-task automation can continue with an empty planning queue
- recent overlay fixes can look correct locally while the real operator journey is still broken

## Minimal Repro

1. Run `:planning`.
2. Choose simple mode and promote the staged scaffold.
3. Submit a normal manual prompt.
4. Observe that the first turn is not planning-aware.
5. Observe that builtin auto follow-up may still queue with `Planning Context` and `next_task: none`.

## Expected Behavior

- planning engagement should be consistent across manual and auto prompt origins, or explicitly disabled in UI
- builtin next-task auto follow-up should not queue when the planning runtime has no actionable queue head
