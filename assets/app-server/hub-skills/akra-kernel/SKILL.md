---
name: akra-kernel
description: Inspect and control the repository kernel through the Akra JSON CLI.
---

# Akra kernel

Generated from schema/akra-kernel-spec.json. Do not edit by hand.

Run `akra describe --json` to inspect input schemas, reasons, and next actions.
Run commands from the target repository.
Each invocation prints one JSON line. Exit codes: 0 accepted, 2 rejected, 1 invalid input, unavailable, or failed.
A stored command (`state: store` in `describe`) answers after it ran: `result` (done), `rejection`, or `failure` (the cause).
Pass an object field as JSON text, for example `--task '{"title":"..."}'`.
Use `--idempotency-key` for mutations that may be retried across separate invocations.
An omitted key is generated for this invocation only. Preserve an explicit key when retrying.
When a command ends with an error, the output carries `idempotency_key` and, for a kernel error, `error.code`.
Send the same command again with that key: the kernel does not run it twice. Find the code in `errors` of `describe`.
Destructive commands require `--confirm true`.
A missing Gateway permits store commands only. Runtime commands require a running Gateway.
A command from this CLI starts the kernel only for that command. The session list is not kept between invocations:
`load_sessions` helps only while another client (Akra Desktop) keeps the kernel up.
Follow structured `next` actions after rejection. Admission does not mean completion.
Use `akra kernel operation <op_id> --wait --json` to observe completion.
Use `akra kernel history --after <sequence> --wait --json` to wait for records.

## Commands

| Invocation | Effect | Summary |
| --- | --- | --- |
| `akra kernel session load_sessions --json` | read | Load the session list of the workspace. The result arrives as the `sessions` event. |
| `akra kernel session rename_session --json` | write | Rename a session (thread). |
| `akra kernel thread submit_prompt --json` | write | Send a prompt to a thread. The kernel prepares it with planning context and submits a turn. Progress arrives as `prompt_submission` events. |
| `akra kernel thread steer_turn --json` | write | Add a prompt to the turn that is running on a thread. |
| `akra kernel thread stop_all --json` | write | Request a stop of every running turn. |
| `akra kernel approval decide_approval --json` | write | Accept or decline a pending approval request. |
| `akra kernel auto_follow set_auto_follow_max_turns --json` | write | Set how many automatic follow-up turns may run after a manual turn. |
| `akra kernel auto_follow pause_auto_follow --json` | write | Pause automatic follow-up turns. |
| `akra kernel planning reset_planning --json` | destructive | Delete planning data (the queue, the directions, or all of it). |
| `akra kernel planning refresh_planning --json` | read | Read the planning queue again. The result arrives as the `planning` event. |
| `akra kernel planning create_task --json` | write | Add a planning task as the operator. The answer carries the new task id and the new planning revision. |
| `akra kernel planning update_task --json` | write | Replace the fields of a planning task as the operator. `expected_planning_revision` is the revision that the sender read; a different current revision rejects the command. |
| `akra kernel planning delete_task --json` | destructive | Delete a planning task and the references to it. `expected_planning_revision` is the revision that the sender read; a different current revision rejects the command. |

Pass input fields as `--field value`. Use the input schema from `describe` for required fields and types.

## Queries

- `akra kernel query sessions --json`: The session list of the workspace and its load state.
- `akra kernel query auto_follow --json`: The automatic follow-up settings. They are the same for every thread.
- `akra kernel query thread_activity --json`: The threads that still have work: a running turn, a pending approval, or automatic follow-up.
- `akra kernel query planning --json`: The planning queue.
- `akra kernel query parallel --json`: The parallel mode state: pool, agents, merge queue, and pull request validation.
- `akra kernel query history --json`: A page of the decision log after a cursor. Send the answer's `next_after` as the next `after`.
- `akra kernel query operation --json`: The state of one operation and its records. `op_id` comes from the `operation` of a command answer.
