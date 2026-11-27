# Conversation title candidates and next-prompt prediction

Decision for #2913 and #2914. The reference baseline is `865e0ade1`,
including #2950. A protocol type, a model version string, and a successful
fake response are not evidence that a production generation route is safe.

## Available without inference

The first meaningful text accepted in a new local thread can supply a short
display title. Keep that candidate in the originating renderer; it is not a
stored name, a model output, or permission to send the text anywhere else.
Allow the user to review, edit, and explicitly save the candidate through
the existing local rename action. An unchanged candidate is still a useful
write; an unchanged authoritative name is not. Discard a stale editor when
a provider name supersedes the candidate, including a name with identical
text. Opening or dismissing the editor must not write anything.

`thread/name/set` takes only a thread ID and a name. Reading the name again
before an automatic write would still race another writer. **No automatic
remote rename** unless the name owner adds an atomic expected-revision
operation. A user-initiated ordinary rename retains its existing semantics.

## Admission for any future generation adapter

| Required proof | Why the current route does not qualify |
| --- | --- |
| Authorized conversation sharing copy for this provider and purpose | Issue masking takes an issue draft and secret spans, not a conversation with authoritative Vault provenance. Display filtering in `threadTitles.ts` is not a disclosure policy. |
| Version-pinned, separately negotiated, tool-less runtime | Desktop uses stable `initialize`; the Gateway forwards ordinary RPCs. The upstream [0.171 isolated helper](https://github.com/openai/codex/blob/rust-v0.171.0/codex-rs/tui/src/temporary_structured_request.rs) reads effective MCP configuration and also sets experimental empty environment, workspace and tool fields. The existing stable connection proves none of this. |
| No parent history, hidden instructions, files, tools, hooks, or credentials | [`thread/sample`](https://github.com/openai/codex/blob/rust-v0.171.0/codex-rs/app-server/src/thread_sample.rs) loads the parent's effective model history. A short prompt and a tool-interception hook do not make that a bounded, redacted request. |
| Real provider capability and source correlation | `thread/prediction/request` is a stub in the [investigated dispatcher](https://github.com/openai/codex/blob/1f9549e265e3ed7604710aae4d12eb82557a7a0c/codex-rs/app-server/src/message_processor.rs). An unsolicited prediction notification is not a negotiated adapter. No Claude execution path or alternate provider is authorized here. |

Fail closed **before reading or sending context** if any proof is absent.
Do not probe by sending a turn. No new own-model calls, retries, or inferred
zero-cost usage. Keep typing, manual rename, and queued user input available.

## Contract before enabling a supported host

- The repository kernel owns one cancellable request per host, repository,
  thread, kind, source turn/revision, prompt version, and model configuration.
  Resume, replay and multiple windows must not trigger more calls.
- Supply only approved, bounded text fields. Cap UTF-8 bytes before dispatch;
  truncate at character boundaries or skip if an essential constraint cannot
  fit. Never include restored Vault values, reasoning, raw tool output, or
  whole parent history. Apply the same disclosure policy to output. Keep
  raw candidate and source text out of auxiliary diagnostics.
- Verify runtime isolation and same-provider routing before starting the
  ephemeral job. Bound response bytes, time, and spend per purpose. On
  timeout, cancellation, tool request, or reconnect, dispose the child and
  ignore late results. Do not emit main-task activity or approvals. Record
  observed usage under `aux.title` or `aux.next_prompt`; unknown stays unknown.
- Titles: a single plain-text line of at most 36 grapheme clusters. Present
  it for review only. Predictions: one structured nullable suggestion, at
  most 240 Unicode scalars. Null is normal. Use only successful live
  completions; skip if there is pending input, approval, or another writer.
- A prediction is never a draft until explicit Tab or button acceptance.
  Recheck host, thread, source turn, context revision, expiry and composer
  interaction revision at acceptance. IME, file menus, attachments and focus
  traversal take precedence. Enter alone must never accept, send or queue it.
  Editing and erasing a draft invalidates the previous suggestion.

Test negotiation, isolation, input/output boundaries, stale callbacks and
draft-only acceptance against a fake transport before enabling any route.
Fake transport tests cannot establish real provider isolation, token pricing
or model quality. Until the admission criteria hold, no generated title or
next-prompt ghost text is a supported production feature.
