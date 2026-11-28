# Local Evidence Vault maintenance

Use the canonical primary workspace, provider, server ID, stable repository ID, and stable issue
ID. These are the same coordinates that identify the local Vault binding. The command does not
contact the provider or delete an issue there.

```text
akra vault retain <workspace> <github|jira|linear> <server-id> <repository-id> <issue-id>
akra vault delete <workspace> <github|jira|linear> <server-id> <repository-id> <issue-id>
```

`retain` removes only inventoried, superseded generations retired for at least 30 days. It does
not remove the live generation, sources retained by confirmed reconnect attempts, or
uncertain staging artifacts. Viewing or cancelling a reconnect review does not write to the Vault.
An unresolved or held publication blocks maintenance for that issue.

`delete` requires a local interactive terminal and the exact `DELETE <issue-id>` response.
The command checks the same Vault revision again after confirmation. It records the issue's
deletion marker before attempting ciphertext cleanup. Restore, restorable publish, reconnect,
and backup import cannot recreate the deleted binding. Normal backup recovery is not a
deletion override.

If cleanup stops, run `delete` for the same scope again. The command reports the number of
tracked records it removed, retained, or could not finish in this pass. Status `1` indicates
cancellation. Status `2` indicates tracked cleanup is pending, records were retained, or
the command cannot verify erasure of legacy data. A storage error at the marker write may mean
the write succeeded; confirm and retry the same scope rather than assuming the issue remains bound.

The inventory covers new issue-scoped writes only. Unknown older filenames are not attributed
to issues and are never erased by these commands. Existing backup files, pre-issue creation
staging, and remote data are outside this operation. File removal is not a secure overwrite of
device blocks. Do not use older Akra binaries against a Vault with deletion markers; they do
not honor the marker.
