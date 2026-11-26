# Issue Publish, Local Restore, and External Change Validation

This page records the automated validation of the issue flow of epic #2414 (issue #2429). The
tests use a GitHub double, a fake OS key store, and synthetic text. They use no real credential, no
real data, no model turn, and no GitHub call.

## Tests

| Test | Command | Level |
| --- | --- | --- |
| `src/adapter/inbound/gateway/tests/issue_flow.rs` | `cargo test --locked --lib issue_flow` | Real Gateway session (WebSocket) with the session recording, `IssueDraftService`, `IssueJournaledPublishService`, `IssueHandoffService`, `EvidenceVaultUnlockService`, `EvidenceVaultRestoreService`, the filesystem journal adapter, and the filesystem Vault store adapter in temporary folders. Unix only. |
| `desktop/test/e2e/issue-draft.mjs` | `cd desktop && npm run e2e` (runs with the other specs) | The issue draft panel against the fake Gateway. |

The Rust tests run in the `cargo test` step of `CI Gate`. A "restart" drops the services and the
Gateway session and builds them again on the same folders. The GitHub double and the key store stay.

## Proven

1. **Validation, preview, edit, unlock, restore.** `open` gives the template and the required
   sections. A draft without a required section is not publishable and a publish of it sends no
   write. The preview removes the secret span and masks the log span with one marker. The approved
   shared copy is written one time to the existing issue. A cancelled confirmation gives no grant.
   A confirmed viewing session restores the log span. The removed secret does not come back. A
   grant of an ended session restores nothing. The hand-off answer is the shared copy.
2. **Held restore and reconnect.** A failed latest read, an external edit, and a marker copied to a
   different issue hold the restore (`LatestReadFailed`, `PublishedCopyChanged`, `NotBound`). A
   reconnect approval of an older review is refused (`ChangedDuringReview`). An approval of the
   current review moves the link of the same issue to the new version, sends no write, makes no
   link for the other issue, and stays after a restart.
3. **Lost answer and restart.** A lost answer with a successful re-read is `confirmed` with one
   write. A lost answer with a failed re-read is `needs_confirmation`; after a restart a second
   publish request returns the same intent and sends no write, and `recheck` confirms by a read
   only and finalizes the restore link. A lost answer of a write that did not reach the provider
   stays `needs_confirmation` after a restart for `open`, `recheck`, and `publish`; a new write
   occurs only after `abandon` and a new approval.
4. **No plaintext.** Each test scans the journal file, the Vault files, the session recording, all
   Gateway answers, and all writes that the provider double received for the synthetic secret and
   the synthetic log line. The `Debug` text of the restore answer and of the reconnect review is
   also checked.
5. **Panel states (Desktop).** Validation errors, the masked preview, refused approval, known
   external change, held, publish result needs confirmation, and publish confirmed. After a window
   reload the editor shows the unknown result again and sends no publish. The local text is not in
   browser storage or the chat composer, and no turn starts.

## Approved-copy follow-up (#2701)

[PR #2709](https://github.com/RefinedStone/codex-exec-loop/pull/2709) implemented the project-scoped
approved-copy check. The Gateway still passes `approved: None` intentionally: the service reads the
authoritative publication and creation journals instead of accepting a client-provided approval hash.
An external edit or an uncertain journal state now blocks scoped hand-off. The test
`the_gateway_hand_off_refuses_an_issue_edited_after_its_approved_publish` is active, without `#[ignore]`.

See [the current contract and acceptance evidence](issue-handoff-approved-copy.md) for manual versus
automatic authority, creation-only issues, UI results, cancellation boundaries, and the limits of the
recorded checks. This correction does not claim a new CI or native OS validation run.

## Original validation limits and separate follow-ups

The items below describe the scope of the original #2429 validation; they are not a current product
support matrix. Later work is tracked in [the native restore record](desktop-vault-native-restore.md)
and [the Issue/Vault failure matrix](issue-vault-failure-matrix.md).

- **Desktop native follow-up.** The original validation above had no shipped
  `VaultRestoreSource`. The [native restore follow-up](desktop-vault-native-restore.md) adds a
  local, target-bound read-only helper and an issue-detail entry point. Its synthetic UI and
  protocol checks do not prove a real OS confirmation plus key-store round trip. Windows Vault
  storage and native UI reconnect remain unsupported; automated profiles still hold viewing.
  The service/storage concurrency follow-up is recorded separately in
  [binding transaction validation](vault-binding-transactions.md).
- **Original process-stop coverage.** The original tests below do not stop the process between the Vault save,
  the journal save, and the remote write, and do not simulate a full disk. The restart tests start
  from the states that a completed `publish` call leaves.
- **UI and lock races.** Issue A to B to A, two windows, and an OS lock during a key read are not
  driven end to end. The unit tests of the unlock service and of the restore session cover the
  late-result rules.
- **First key creation by two processes.** `sealing_key` has a documented shortcut for this case.
  No test runs two processes.
- **Real OS key store.** All tests use `FakeVaultKeyStoreAdapter`.
- **Windows.** The Vault store adapter works on Unix only, so the flow test is `cfg(unix)`.

## Defects found by this validation

- The draft publish gave the journaled publish one record for each masked span with the raw
  plaintext. The restore service reads one bundle for each field (marker, position, plaintext).
  A restore after such a publish was not possible. Fixed: the publish uses `restorable_fields`.
- No Vault key reached the publish, and no code finalized the restore link. Fixed: the publish
  uses `EvidenceVaultUnlockService::sealing_key` and finalizes the link on `confirmed`, `open`, and
  `recheck`.
- The composition did not create `$AKRA_HOME/evidence-vault`, and the Vault store needs an existing
  folder. Fixed: `build_issue_drafts` creates it as a private directory.
