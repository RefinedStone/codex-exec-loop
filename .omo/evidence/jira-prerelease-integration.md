# Jira provider integration with current prerelease

The late review sweep found PR 2357 conflicting with prerelease after the Notion
provider and MCP cache merged. The feature branch integrates prerelease 4e9146e8
without rewriting its published commits.

Resolution preserves both providers in production. The shared version port keeps
the full ExternalResourceRef required by Jira site identity. Notion reads its page
ID from that resource. Its existing tests now exercise this signature. A duplicate
availability check introduced by automatic merging was removed. Product and plan
documentation describe both implementations.

Validation:

- Resource-version tests: 20 passed, including Jira/Notion/GitHub providers.
- Work-context tests: 22 passed.
- Windows Rust gate: passed, including portable contracts and clippy.
- Formatting and whitespace checks passed.
- Desktop files are identical to the previously approved 599047c3 head; its
  type checks, unit tests and real Jira reverse-provider UI evidence remain valid.
- Full planner gate again stopped at the unchanged picomatch admin audit.

The existing Jira gate reviewer will review this integration delta before delivery
is considered complete. CI remains required; no failing gate is bypassed.

## Resumed delivery: prerelease 52d46b54

The next integration preserves the new Git graph service and foreign-writer
subscription checks alongside Jira registration and refresh. MCP idle release
continues to use the existing GatewayServices hub.

The current Full Windows gate passed, including Desktop type checks, 326 passing
unit tests (three existing skips), builds and native contracts. Prerelease already
contains its dated picomatch audit exception; this integration adds no exception.

The complete Electron sequence passed after fixing a settings fixture cleanup
defect: it queried app.process() after the application connection was disposed.
Retaining the ChildProcess before closing windows fixes that failure. The failing
settings scenario and the remaining Git graph and Jira scenarios then passed.
Previously passed scenarios were not repeated. Jira captures at 1440 and 900 pixels
were inspected. Existing narrow shell tab wrapping remains outside the Jira change.

## Approved linear delivery on f015b106

The user approved backing up and rewriting the three published feature branches.
The Jira original head remains at
`backup/pr2357-approved-rewrite-1182444b`.
Latest base changes were integrated; the only textual conflict was the upstream
comment for the already-identical settings cleanup fix.

The index tree was `06d34f635510c259a5b9aa0dd4630dec54a7b0ab` both before and
after moving the commit parent, proving that history preparation did not alter
the integrated source. Windows Full passed with 339 Desktop tests (three existing
skips), and the full Electron sequence including Jira passed. Updated wide/narrow
Jira captures were inspected. Force-with-lease will require remote head 1182444b.
