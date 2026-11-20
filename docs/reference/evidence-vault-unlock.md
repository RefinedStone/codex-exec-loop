# Evidence Vault Key and Unlock

This page describes the shipped key handling of the Evidence Vault (#2422): where the key lives,
how an issue is unlocked, what can be recovered after a key loss, and what the Vault does not
protect against. Backup export and import with a user-held recovery key are in
[evidence-vault-backup.md](./evidence-vault-backup.md). The Desktop restore window (#2427) is
shipped as a held screen, see [Desktop restore window](#desktop-restore-window). Nothing wires the
unlock service to a client yet.

## Key location: device-bound mode

- The Vault has one key for each OS user on each device. `OsVaultKeyStoreAdapter` keeps it in the
  OS key store through the `keyring` crate, under service `akra-evidence-vault`, account
  `device-key-v1`.

  | Platform | Key store |
  | --- | --- |
  | macOS | Keychain (login keychain) |
  | Windows | Credential Manager |
  | Linux | Secret Service (for example GNOME Keyring, KWallet) |
  | Other platforms, Linux without a Secret Service | Not supported |

- The key is 32 random bytes from the OS random source. `EvidenceVaultUnlockService::sealing_key`
  makes it the first time a record is sealed. The key is not written to the vault folder, the
  repository, a Kernel event, or a Gateway response.
- Each record stores a key identifier: the first 64 bits of a SHA-256 fingerprint of the key. The
  identifier does not disclose the key.

## Unlock

- `EvidenceVaultUnlockService` (`src/application/service/evidence_vault_unlock.rs`) is the only
  code that makes a `VaultUserConfirmation`. Adapters, core, and composition cannot construct one.
- A viewing session starts with `begin_session` and ends with `end_session` or when the process
  stops. Sessions are held in memory only.
- `unlock` takes one issue, one live session, and the user decision from the trusted local UI. On
  success it returns a `VaultUnlockGrant` bound to that issue and that session. The grant holds no
  key.
- `access` exchanges a live grant for the confirmation and the record key. The key is read from
  the OS key store on each call and is not cached by the service. A grant for another issue,
  another session, or an ended session gives `SessionExpired`.

## When unlock fails: shared copy only

The caller gets one of these reasons and shows the shared (masked) copy only.

| Reason | Cause |
| --- | --- |
| `KeyStoreUnsupported` | The environment has no OS key store. |
| `AuthenticationCancelled` | The user cancelled the confirmation, or the OS denied or the user cancelled the key store authentication. |
| `SessionExpired` | The viewing session ended, or the session holds no grant for the issue. |
| `KeyLost` | The key store works but holds no Vault key. |
| `KeyStoreUnavailable` | The key store failed for another reason. A retry is possible. |

The unlock service has no access to the record store. A cancel or a failure does not delete or
change ciphertext records, and does not create or replace the key. A cancelled user confirmation
does not call the key store.

## Recovery range after a key loss

In device-bound mode the key exists only in the OS key store of one user on one device. There is
no second copy unless the user made a backup archive and kept its recovery key
([evidence-vault-backup.md](./evidence-vault-backup.md)).

- The key is lost when the key store entry is deleted, the OS user profile or login keychain is
  reset or removed, the OS is reinstalled without a key store restore, or the vault folder is
  copied to another device or another OS user.
- After a loss, the masked segments of records sealed with that key cannot be restored by Akra.
  The ciphertext files stay on disk unchanged. They become readable again only if the same key
  returns to the key store (for example from an OS-level keychain backup).
- The shared copy on the provider is not affected. It stays readable, with the masked segments
  masked.
- The next sealed record makes a new key with a new key identifier. Old records then fail with
  `KeyMismatch`; they are not overwritten by this.
- Records that are in a backup archive can be imported again with the recovery key of that
  archive. Records made after the export, and records without an archive, stay lost.

## Protection limits

The Vault protects masked segments from the issue provider, from Kernel events, from Gateway
clients, and from another OS user who reads the vault folder. It does not protect against code
that runs with the same OS user rights on the same device:

- An external shell, script, or process of the same user can ask the OS key store for the key.
  macOS can show a consent prompt for a different application, but a process that the user
  allowed, or that runs as the same application identity, reads the key without the Akra
  confirmation. Windows Credential Manager and Linux Secret Service give the entry to any process
  in the unlocked user session.
- The Akra user confirmation is an in-process rule. It is not an OS authentication (no password or
  biometric check is requested by Akra).
- While a restored copy is on screen, a screen capture, screen recording, screen sharing, or an
  accessibility tool of the same user can read it. Akra does not block capture.
- Restored plaintext and the key are in process memory while in use. A debugger or memory dump
  with the same user rights can read them. The types zero their memory on drop, which reduces the
  time window only.
- An administrator or root user, and malware with user rights, are out of scope.

## Desktop restore window

Akra Desktop has one dedicated window for the local restored copy of one issue (#2427). Code:
`desktop/src/main/vaultRestoreWindow.ts`, `desktop/src/main/vaultRestoreSession.ts`,
`desktop/src/shared/vaultRestore.ts`.

- **Separate surface.** The window has its own preload and its own IPC channels
  (`akra-vault-restore:*`). It has no `window.akra`, no Gateway connection, and no conversation IPC.
  It is not registered with the browser panel, so thread-owned browser automation and its capture
  cannot target it. DevTools, navigation, and new windows are off. Main accepts a restore request
  only from the main frame of a restore window. A general window can only ask to open the window
  for a `github.com` issue address and to lock it.
- **Plaintext path.** Restored text goes from main to this window only, on the dedicated state
  channel. It does not use the Gateway or the kernel protocol. Main does not keep the text after it
  sends it. The screen writes the text with `textContent` (no Markdown, no HTML).
- **Not connected yet.** The local restore source (`VaultRestoreSource`, the Desktop end of the
  restore service) has no shipped implementation. The window shows "viewing held" with the reason
  `source_unavailable`. A local-only channel from the Akra process to this window is proposed work.
- **User confirmation.** Each unlock shows a system dialog from main on the restore window. The
  renderer cannot answer it. After each lock a new confirmation is necessary.
- **Execution profile.** Main holds viewing when it cannot enforce the boundary: the app runs with
  remote debugging or an inspector (automation, screen tests), the renderer comes from a dev
  server, windows are hidden for tests, or a turn runs in a window of this app (the default agent
  profile has a general shell). Turns of other clients and of background workers are not seen by
  this check.
- **Re-lock.** The window locks again when the user locks it, the window closes, hides, or reloads,
  the viewing session ends (5 minutes), the OS screen locks, the system suspends, the opener window
  changes its project folder or closes, or a turn starts. A lock closes the restore channel, drops
  late replies (generation check), and replaces the DOM.
- **Reconnect review.** When the restore service reports a changed published copy, the window shows
  the current shared copy and, for each marker, the earlier place and the current places. The user
  selects a place for each marker; a marker without a selection stays a marker. Main accepts only
  places that the review gave. The earlier shared copy is not stored, so the full earlier text is
  not shown.
- **Limits.** Content protection (`setContentProtection`) is requested, but Akra does not guarantee
  that capture is blocked (it has no effect on Linux). Selection and copy by the user are possible.
  The window has no print and no find function. The limits in
  [Protection limits](#protection-limits) apply.
