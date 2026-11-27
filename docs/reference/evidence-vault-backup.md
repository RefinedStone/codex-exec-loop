# Evidence Vault Backup and Recovery Key

This page describes the shipped backup of the Evidence Vault (#2432): the archive format, the
user-held recovery key, the import rules, and the limits. Nothing wires the backup service to a
client yet; there is no screen that shows the recovery key or selects an archive.

Code: `EvidenceVaultBackupService` (`src/application/service/evidence_vault_backup.rs`),
`EvidenceVaultBackupPort` (`src/application/port/outbound/evidence_vault_backup_port.rs`),
`FilesystemEvidenceVaultBackupAdapter`
(`src/adapter/outbound/filesystem/evidence_vault_backup.rs`).

## Export

- `export` takes a list of issues. For each issue that has a restore binding it exports the
  `CurrentVersion` record and the `ConfirmedVersion` records of that version. Issues without a
  binding and publish-intent records are not exported.
- Each record is verified with the device key before it goes into the archive. A record that
  fails stops the export. The archived `CurrentVersion` must match the selected field scopes;
  otherwise `BindingChanged` asks the caller to retry with a fresh selection.
- `export` makes a new recovery key (32 bytes from the OS random source) and returns it together
  with the archive content. Akra does not store the recovery key. The caller shows it one time
  (`VaultRecoveryKey::expose_text`: 64 hexadecimal characters in groups of 8) and the user keeps
  it outside the archive.
- `export` does not change the vault. It returns no restored plaintext. The caller writes the
  archive content to a file that the user selects.

## Archive format (version 1)

The archive is one JSON document.

| Field | Content |
| --- | --- |
| `formatVersion` | `1`. An unknown version is rejected as `UnsupportedFormat`. |
| `algorithm` | `chacha20-poly1305` (key wrap). |
| `kdf` | `hkdf-sha256`. |
| `backupId` | 16 random bytes, hexadecimal. Stable identifier of the archive and HKDF salt. |
| `keyId` | Key identifier of the device key that sealed the records. |
| `wrapNonce` | base64, 96-bit random nonce of the key wrap. |
| `wrappedKey` | base64, the device key sealed under the wrap key, with the authentication tag. |
| `records` | List of `issue`, `field`, `binding`, and `record`. `record` is the unchanged content of the record file (see the record format in [architecture](./architecture.md)). |

- Wrap key: HKDF-SHA256 from `ring`, input key material = recovery key, salt = `backupId` bytes,
  info = `akra-evidence-vault-backup-wrap-key-v1`, output = 32 bytes. The recovery key has 256 bits
  of entropy, so no cost-based KDF is used and the archive holds no KDF cost parameter.
- Key wrap: ChaCha20-Poly1305 from `ring`. The associated data binds the format version,
  algorithm, KDF, `backupId`, `keyId`, and a SHA-256 over the ordered record list (scope and record
  content of each record).
- The archive does not contain the recovery key, the unwrapped device key, or restored plaintext.
- The archive does contain, readable without a key: the local project path, the provider server,
  the repository and issue identifiers, and the bound version name of each record.
- Limits: 8 MiB archive size and 4,096 records. A larger archive is `ArchiveTooLarge`.

## Import

`import` needs the archive content, the recovery key, a confirmed user decision, and a list of
approved projects. Each approval names one project of the archive and the local project that
receives its records.

1. A cancelled decision stops before the key store and the vault are touched (`NotApproved`).
2. The whole archive is verified before the first write: the device key must unwrap with the
   recovery key, and every record must open with that key and its original scope. A wrong
   recovery key or a changed header or record list is `RecoveryKeyRejected`. An unreadable archive
   or a failed record is `CorruptArchive`. Nothing is written in these cases.
3. Records of approved projects are sealed again with the local device key and the local project
   scope. The remote issue (provider, server, repository, issue) is never changed. Records of
   projects without an approval are not written; the report lists those projects. An import with
   an empty approval list is a check of the recovery key and a list of the projects in the archive.
4. Only an authenticated import with at least one approved issue asks for the local sealing key.
   Malformed archives, wrong recovery keys, and project-list previews leave a fresh device key store
   untouched. An approved installation on a device without a key uses the
   [coordinated initializer](./evidence-vault-key-initialization.md). An unresolved attempt stops
   installation with `Locked(KeyStoreUnavailable)` before the first record write. The exporting
   device key is used in memory only and is not stored.
5. A readable local `CurrentVersion` is never changed (`kept_local`), including in recovery mode.
   An unreadable binding returns `LocalRecoveryRequired` in `PreserveLocal` mode. Only explicit
   `RecoverUnreadable` approval replaces that issue's backup field records and installs its binding
   last; the report lists the issue in `recovered`. This recovers a device whose OS key was lost.
   An issue without a binding validates and reuses an identical partial generation from an
   interrupted import. A repeated import with a readable binding writes nothing.
6. Each issue uses the shared binding transaction. The readable-local check, immutable generation
   validation, and CurrentVersion CAS run under the same cross-process lease. An issue is bound
   only after both selected field records are in place. See [binding contract](./evidence-vault-bindings.md).

Import does not make plaintext available by itself. `EvidenceVaultRestoreService` reads the latest
published copy before each restore; when the published copy differs from the imported version, the
restore is held and the user reconnects on that issue (see [architecture](./architecture.md)).

## Interrupted import

- An import that stops before its CurrentVersion CAS leaves its previous reachable binding
  unchanged. Identical unreferenced generation files can be reused on a retry. A missing binding
  remains missing until the complete selected field set is ready.
- An import that stops between two `CurrentVersion` writes leaves some issues bound and the others
  not bound. The import is atomic for each issue, not for the whole archive.
- The same import can be run again. It preserves every readable local binding, including one
  installed by a competing publisher or reconnect. Incomplete or explicitly approved unreadable
  bindings use an immutable generation and a CAS against the observed pointer.
- Failure after the pointer replacement is reported as
  `Record(VaultStoreError::WriteOutcomeUnknown)`. Reload before retrying; do not assume rollback.
  A retry which sees a readable pointer preserves it and reports `kept_local`.
- Restored plaintext and the unwrapped device key exist in process memory only during the import.
  They are not written to a temporary file or a log.

## Limits

- A lost recovery key cannot be made again. Without it the archive cannot be opened by Akra or by
  anyone else; the records in it stay ciphertext. The vault on the original device is not affected
  and a new export makes a new recovery key.
- The recovery key and the archive together are equal to the device key of the exporting device at
  the time of the export. Keep them in different places.
- A new export does not cancel an older archive or an older recovery key.
- Records made after the export are not in the archive.
- The exporting device and the importing device do not synchronize. An import is a one-time copy.
- Import does not check that an issue still exists on the provider. A deleted or transferred issue
  stays imported and its restore is held by the restore service.
- Platform: record access uses the Vault-specific file boundary. It retains the Unix implementation
  and supports Windows local NTFS with verified private ownership, ACLs, and pinned paths. ReFS and
  network shares are not supported. Import requires an existing safe Vault root and does not repair
  an unsafe root or create a missing one. See [Windows validation](../validation/windows-vault-restore.md).
- Concurrent import, publish finalization, and service reconnect use the same per-issue binding
  lease. This does not make the entire multi-issue archive atomic or add a distributed transaction
  with the provider. Native Desktop reconnect uses the same transaction behind its
  [dedicated confirmation and session boundary](../validation/desktop-vault-native-reconnect.md).
