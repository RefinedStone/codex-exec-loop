# Protected Git destinations

Ordinary push remains fast-forward only. A lease rewrite is a separate operator action. These rules are local to Akra; they never change a GitHub protection setting.

Open the branch publisher’s **보호 브랜치 및 lease push**, or use:

- `akra git protect list [--remote NAME]`
- `akra git protect review [--remote NAME]`
- `akra git protect add 'release/*' [--remote NAME]`
- `akra git protect remove 'release/*' [--remote NAME]`
- `akra git force-push REMOTE DESTINATION`

Review the actual remote default and Akra integration branches. Both are protected unless explicitly disabled by name. Extra patterns use `*` as a wildcard. Rules live outside the worktree in the private repository runtime, keyed by the canonical push URL; worktrees and aliases of that endpoint share the rules. Changes are audited. There is no per-push override.

The native confirmation window requires typing the displayed branch, pattern, or remote. Ordinary Gateway and MCP requests cannot set rules or approve pushes. The CLI refuses noninteractive mutation; a Desktop request can open native confirmation but cannot supply the confirmation text. An OS process controlling the same user's GUI is outside this boundary. Native UI must be available; cancellation or timeout is not approval.

The explicit fetch uses the push destination’s isolated Git context. Review names the source SHA, observed destination SHA, and a bounded list of disappearing commits. Execution rechecks the checked-out branch, source SHA, destination URL, current rules, and automatic protection. The push pins both source and expected destination; a later tracking fetch is irrelevant. Server-side rules can still refuse. Remote deletion also checks Akra policy. The background rewrite path cannot perform a non-fast-forward update. Neither local reset nor local branch moves use this policy.
