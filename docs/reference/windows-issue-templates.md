# Repository issue templates on Windows

Windows reads a configured Markdown or YAML issue template through
`FilesystemIssueTemplateSourceAdapter` and the existing `IssueTemplateService`.
The adapter is read-only. The parser, required-field rules, SHA-256 template
version, and approval/version checks are shared with Unix.

## File boundary

The Windows reader is limited to this adapter. The Windows implementations of
the generic planning/Vault `secure_fs` operations keep their existing behavior.

- The workspace must be an absolute local drive path, including the equivalent
  verbatim drive spelling used for long paths. UNC shares, device namespaces,
  relative roots, and parent-directory aliases are rejected before file access.
- Template paths are relative to that workspace. Components containing alternate
  data stream syntax, control characters, trailing dots/spaces, or DOS device
  names (including COM/LPT superscript digits) are rejected. The private
  `.akra/runtime` namespace stays excluded, including aliases using
  DOS short directory names.
- The reader retains every ancestor directory handle and rejects reparse points,
  including junctions. It checks the selected root first, then each template
  parent. A broken or redirected ancestor cannot become a missing-template result.
- Workspace/template-directory and file ACLs use the existing Windows path-trust
  policy. The current user, SYSTEM, Administrators, and TrustedInstaller are trusted
  owners. File writes and directory deletion/replacement/security-control rights
  granted to other principals are rejected. Directory child-creation rights and
  inherited read access remain allowed; the opened leaf has its own trust check.
  Reading never repairs an ACL, changes ownership, or creates directories.
- The leaf must be a non-reparse, single-link regular file. Its handle permits
  concurrent readers and denies write/delete sharing until the read and final
  validation finish. The reader compares file identity and metadata before and
  after reading and checks the retained directory chain again.
- Contents must be UTF-8 and at most 1 MiB. The bounded read allows at most one
  additional byte to detect overflow; an exact-limit file is accepted.

A missing template leaf or normal parent becomes `Missing` through the service.
Unsafe paths, denied access, sharing violations, wrong file types, invalid UTF-8,
and oversized files become `Unreadable`. Neither result removes the template
requirement. Unsupported form fields and invalid YAML keep the existing parser
results.

This does not add Windows Vault restoration, native user approval, or a sandbox
for external code validators. It does not make the general Windows planning
filesystem implementation available.

## Validation

Both Windows gate entrypoints select `filesystem::issue_template::`:

- `powershell -File scripts/check_native_pr.ps1 -Mode Full`
- `bash scripts/check_windows_portable.sh` on the clean Windows CI runner

The adapter regressions use actual NTFS files, junctions, links, and retained
handles. Service tests read actual Markdown/YAML files and check required fields,
version changes, and missing versus unreadable results. No model turns or live
GitHub writes are needed.

Windows API references:
[CreateFile sharing modes](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and [file naming rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).
