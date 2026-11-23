# Evidence Vault Key and Unlock

This page describes the shipped key handling of the Evidence Vault (#2422): where the key lives,
how an issue is unlocked, what can be recovered after a key loss, and what the Vault does not
protect against. Backup export and import with a user-held recovery key are in
[evidence-vault-backup.md](./evidence-vault-backup.md). The Desktop restore window (#2427) connects
existing records through a local native helper on macOS/Linux and Windows with a local NTFS Vault.
Unsupported execution profiles hold viewing; see [Desktop restore window](#desktop-restore-window).

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
  requests it through `get_or_create_key`. The OS adapter coordinates first creation across
  processes with a private durable intent and verifies the stored key before returning it.
  Unresolved attempts hold creation; retrying does not send another candidate. See
  [key initialization](./evidence-vault-key-initialization.md). The key is not written to the
  initialization database, vault folder, repository, a Kernel event, or a Gateway response.
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
| `KeyStoreUnavailable` | The key store failed, or a creation attempt remains unresolved. A later request can re-read the key; it does not automatically create another key. |

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
- After a settled generation loses its key, the next sealed record can create a new generation
  and key identifier. A pending initialization with no readable key holds instead. Old records
  fail with `KeyMismatch` when a different key is used; they are not overwritten by this.
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
- The Akra user confirmation is a local application rule. It is not an OS authentication (no password or
  biometric check is requested by Akra).
- While a restored copy is on screen, a screen capture, screen recording, screen sharing, or an
  accessibility tool of the same user can read it. Akra requests content protection but does not guarantee that capture is blocked.
- Restored plaintext and the key are in process memory while in use. A debugger or memory dump
  with the same user rights can read them. Rust secret types zero their memory on drop, which reduces the
  time window only. Desktop JavaScript strings rely on garbage collection; deterministic clearing
  of every plaintext copy is not guaranteed.
- An administrator or root user, and malware with user rights, are out of scope.

## Desktop restore window

Akra Desktop has one dedicated window for the local restored copy of one issue (#2427). Code:
`desktop/src/main/vaultRestoreWindow.ts`, `desktop/src/main/vaultRestoreSession.ts`,
`desktop/src/shared/vaultRestore.ts`.

- **Separate surface.** The window has its own preload and its own IPC channels
  (`akra-vault-restore:*`). It exposes only the dedicated `window.akra.vaultRestore` contract,
  with no general AkraBridge, Gateway connection, or conversation IPC.
  It is not registered with the browser panel, so thread-owned browser automation and its capture
  cannot target it. DevTools, navigation, and new windows are off. Main accepts a restore request
  only from the main frame of a restore window. A general window can only ask to open the window
  for a `github.com` issue URL plus its full stable coordinates and workspace, and to lock it.
- **Plaintext path.** Restored text goes from main to this window only, on the dedicated state
  channel. It does not use the Gateway or the kernel protocol. Main does not persist the text or put it in general-window state. The screen writes the text with `textContent` (no Markdown, no HTML).
- **Local native source.** `vaultRestoreSource.ts` launches `akra-gateway desktop-restore-bridge`
  over inherited stdin/stdout. Protocol v2 binds one viewing and optional reconnect session to the
  URL, stable issue coordinates, and canonical primary checkout. It reads existing records on
  macOS/Linux and Windows, and updates a binding only after a separate native write confirmation.
  Windows file operations require local NTFS and verified private ownership and ACLs; unsupported
  filesystems and unsafe existing paths are rejected. The channel does not create keys or Vault roots
  and does not restore a remote Gateway host's Vault.
- **User confirmation.** The helper itself shows a native OS dialog with cancel as the default.
  Viewing and `RewriteBinding` use separate confirmations. No request field can supply an approval
  result or replace the helper-owned original review. macOS uses the trusted system `osascript`;
  Linux requires a trusted system `zenity`. Windows runs the trusted Gateway executable in a separate
  process and a native OK/Cancel dialog with Cancel selected by default. Its target and
  purpose travel over inherited private pipes, and the parent owns the child's lifetime. Missing
  native support holds viewing. Main checks the
  execution profile and session generation before preparation, after confirmation, and before
  displaying a restored response. After each lock a new confirmation is necessary.
- **Execution profile.** Main holds viewing when it cannot enforce the boundary: the app runs with
  remote debugging or an inspector (automation, screen tests), the renderer comes from a dev
  server, windows are hidden for tests, or a turn runs in a window of this app (the default agent
  profile has a general shell). Turns of other clients and of background workers are not seen by
  this check.
- **Re-lock.** The window locks again when the user locks it, the window closes, hides, reloads,
  navigates, or loses its renderer, the viewing session ends (5 minutes), the OS screen locks, the
  system suspends, the opener changes project or ends its renderer lifetime, or a turn starts.
  Lifecycle invalidation closes the channel immediately and makes the target inactive; a fresh
  open request with current full coordinates must reactivate it. Late replies are dropped by the
  generation check and the DOM is replaced. Review and write confirmation do not extend the deadline.
- **Reconnect support.** A prepared v2 helper advertises reconnect support. Only its first
  changed-copy or marker hold may return `can_reconnect: true`. The dedicated window requests a
  review, selects marker candidates, and sends the exact reviewed hash and mappings. The helper
  keeps the original review and grant, obtains separate native write confirmation, rechecks the
  latest copy and local binding, and commits through the [atomic binding store](./evidence-vault-bindings.md).
  One final restore follows; later holds are terminal even if their reason describes a changed copy.
  Duplicate or out-of-order requests do not repeat the operation.
- **Write cancellation.** A one-shot gate immediately before `binding.commit` shares the session
  state mutex with cancellation. Cancellation before admission writes nothing. After admission,
  the atomic write may complete while the channel and late plaintext are discarded.
  `write_outcome_unknown` is a terminal result: reopen to inspect the current binding rather than
  automatically resending. A changed remote copy or local binding requires a new review.
- **Limits.** Content protection (`setContentProtection`) is requested, but Akra does not guarantee
  that capture is blocked (it has no effect on Linux). Selection and copy by the user are possible.
  The window has no print and no find function. The limits in
  [Protection limits](#protection-limits) apply.

The [original native read validation](../validation/desktop-vault-native-restore.md) and
[native reconnect validation](../validation/desktop-vault-native-reconnect.md), together with the
[Windows Vault validation](../validation/windows-vault-restore.md), distinguish synthetic protocol/UI
and real filesystem/process tests from the unperformed real OS-dialog plus key-store round trip.
