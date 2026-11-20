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
  fails stops the export.
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
4. The local device key is not replaced. A device without a key gets a new device key. The device
   key of the exporting device is used in memory only and is not stored.
5. An issue that already has a local `CurrentVersion` record is not changed (`kept_local`). An
   existing local record file is never overwritten. A repeated import of the same archive writes
   nothing.
6. The `CurrentVersion` records are written last. An issue is bound only after its other records
   are in place.

Import does not make plaintext available by itself. `EvidenceVaultRestoreService` reads the latest
published copy before each restore; when the published copy differs from the imported version, the
restore is held and the user reconnects on that issue (see [architecture](./architecture.md)).

## Interrupted import

- Each record file is replaced atomically. An import that stops before the `CurrentVersion`
  records leaves unreferenced record files and no binding; no issue is restorable from a partial
  set.
- An import that stops between two `CurrentVersion` writes leaves some issues bound and the others
  not bound. The import is atomic for each issue, not for the whole archive.
- The same import can be run again. It skips the records that exist and writes the remaining ones.
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
- Platform: the vault record files use `secure_fs`, which is implemented for Unix only (macOS and
  Linux). The backup code builds on Windows, but the file operations fail there and the tests run
  on Unix only.
- The checks before each write and the write are not one locked step. Two processes that import
  into the same vault at the same time are not supported.
