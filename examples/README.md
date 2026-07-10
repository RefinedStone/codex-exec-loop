# Bundled Prompt Examples

These files are prompt templates for an interactive Akra session. Akra does not execute them
automatically.

1. Start `akra` from the repository you want to inspect.
2. Review the matching language file and adapt the task boundary if needed.
3. Paste the prompt into the TUI input.

Available templates:

- `reviewable_task.en.txt`: English prompt for one guarded, reviewable repository task.
- `reviewable_task.ko.txt`: Korean equivalent.

Repository instructions remain authoritative. The templates deliberately require an isolated
worktree and verified remote identity, and they prohibit remote writes when the repository does not
define a delivery workflow. A prompt example is not permission to create branches, push, open a PR,
or merge into a remote repository.
