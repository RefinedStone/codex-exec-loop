# Herdr

## Snapshot

| Field | Value |
| --- | --- |
| Site | release host `releases.herdr.dev` (read from the binary; the site was not opened) |
| Official repository | not resolved in this audit |
| Release | Herdr 0.1.16 (macOS arm64, bundle id `dev.herdr.app`), marked BETA in its own first-run window |
| Source commit | none; this audit read the installed application only |
| License | Apache-2.0 (the `LICENSE` file inside the application bundle) |
| Audit date | 2026-11-18 (Asia/Seoul) |
| Previous Akra pin | none (first audit) |
| Akra baseline | `874d1851` (`prerelease`) |

Evidence is static inspection of the installed application: bundle layout, linked system libraries,
the third-party notice file, and the printable strings of the one executable. The operator supplied
one screenshot of the first-run window. Akra did not run the application, did not read its source
repository, and did not read any Herdr user data. Every claim below is `verified/runtime` only for
"this text is in the 0.1.16 binary"; what a person sees on screen is `inferred` unless the
screenshot shows it.

## Verdict

Herdr is a native terminal fleet for agent CLIs with a separate background core. Its threat to Akra
is not the engine: it runs agent CLIs in terminals and learns their state from what the agent
reports. Its threat is finish. A first-run window answers the one question a new user has, every
action is one named command, and updates, feedback, and attention marks are all present in a 0.1
release.

Akra already has the same process split (one kernel and a Gateway daemon behind thin clients) and a
stronger state source (typed `codex app-server` events). Akra keeps Electron for Desktop and takes
behavior only.

## Architecture Facts

- **One native executable, no web view.** The bundle holds one 58 MB Mach-O file and no
  `Frameworks` directory. It links AppKit, Metal, and CoreText, and does not link WebKit.
- **GUI on a vendored `gpui`.** Source paths under `vendor/gpui-pre` are in the binary.
- **Terminal through `libghostty`.** The notice file lists `crates/ghostty-sys` and a Zig license.
- **Screens are JavaScript, not Rust.** The binary embeds QuickJS and the source text of its screens.
  That text builds `gpui` elements from JavaScript (`div().px(12).text_size(12).child(...)`). The
  panels, the settings window, and the first-run window are such scripts, named "mods"
  (`mods/files`, `mods/changes`, `mods/workspace-chrome`, `mods/tabs`, `mods/pane`).
- **A separate core.** `herdr-core` (command `hc`) runs every terminal and agent. The application
  window is a client; terminals continue when the window closes, updates, or stops.

"Pure Rust application" is therefore two-thirds right: a Rust shell and renderer, a Zig terminal, and
JavaScript screens.

## Strengths

- **First-run window that answers a worry.** Its first title is "Herdr runs your terminals in the
  background", followed by what continues when the window closes. A later step turns panels on with
  switches and the panel opens behind the window at once (shown in the screenshot). The window can
  be moved, and it says where the same control is afterward.
- **Four attention sections in the person's words.** The agent list sorts into `Needs you`, `Done`,
  `Working`, `Idle`. A blocked row carries a "Needs you" mark, shortened to `!` where a header
  already says it.
- **One command table.** Every action is a named command (`space.new`, `tab.settle`,
  `terminal.peek`, `terminal.pin`, `app.zoom_in`, `app.whats_new`). An entry holds a group, a title,
  and search words. The palette, menus, context menus, and agents call the same names.
- **Peek and pin.** `terminal.peek` opens another agent over the current tab with "keep as tab" and
  "close". `terminal.pin` holds an agent at the top of the lists; the pin is stored in the core, so
  every client shows it.
- **Honest updates.** The application checks each hour and tells the person in a corner note. It
  shows application and core updates separately, says that terminals continue through a core
  update, and shows "Up to date" only after a check that succeeded.
- **Feedback with a person in the loop.** `app.feedback` opens a review window that takes up to
  three screenshots. An agent can draft feedback; nothing is sent until the person clicks Send.
  Usage data stops with `DO_NOT_TRACK=1`, and a build from source never sends.
- **Agents drive the application.** `hc ls` and `hc attach` let an agent start and watch other
  agents. Layouts and mods are plain files an agent can change or add, so a person asks an agent for
  a new panel.
- **Other machines in one window.** A host from the SSH configuration gets its own core and its own
  section in the sidebar.

## Limits

- **State comes from the agent's own report.** The empty agent list says so: "Older versions and
  agents inside tmux may not." No typed engine event stands behind a mark.
- **The terminal is the conversation surface.** Diffs, tables, and plans are terminal text.
- **A large private toolkit to maintain.** A vendored GUI fork, a JavaScript binding layer, and a
  Zig terminal library are all load-bearing.
- **Beta.** Version 0.1.16; one release note asks people to reopen terminals after a core update.

## Akra Decisions

### Adopt

- Attention section names in the person's words, with one reason line per row. Akra can state the
  reason because the kernel knows the cause; Herdr cannot.
- One command table in Desktop that the palette, the shortcuts window, the application menu, and
  the context menus all read.
- A first-run window whose first step says that work continues when Desktop closes, and whose panel
  step shows each panel behind the window.
- Peek at another thread without leaving the current one.
- A pin that the kernel stores, so TUI and Desktop show the same pinned threads.
- Update rules: separate client and daemon versions, "work continues" wording, and "up to date"
  only after a successful check.
- Feedback as a draft the person sends, opened as a GitHub issue draft because Akra is public.

The proposed items and their order are in
[plan 26](../../plan/26-akra-desktop-usability.md) (section 3.11).

### Reject

- Copying Herdr source. The license allows it, but plan 17 keeps Herdr as a reference only.
- Replacing Electron with a native GUI now. The kernel and Gateway are already Rust and clients are
  thin, so a second client stays possible later without a kernel change.
- A terminal-first conversation model and state read from an agent's report.
- User-written panels (mods). The isolation boundary and the panel API are a large design; revisit
  only with a concrete need.
- Installing a daemon on other machines over SSH before the Gateway scope decision.
- Decorative themes.

### Differentiate

- Typed app-server events and a kernel spec with reasons, ordered next steps, and idempotent
  commands, where Herdr has self-reported status.
- Structured conversation, diff, and plan views instead of terminal text.
- Reviewed delivery (worktree, review evidence, merge, cleanup) as one lifecycle.

### Watch

- The public source repository, to replace string evidence with `verified/source`.
- Whether mods become a real extension surface that people share.
- Whether status reporting moves from agent self-report to an engine contract.

## Sources and Evidence Gaps

- Installed application 0.1.16: `Contents/Info.plist`, `Contents/Resources/LICENSE`,
  `Contents/Resources/THIRD-PARTY-NOTICES`, `otool -L` on the executable, and `strings` on the
  executable.
- One operator screenshot of the first-run window ("Choose your panels").
- Not verified: any runtime behavior beyond that screenshot, the source repository and its commit,
  the size and content of the `gpui` fork, `herdr-core` internals, remote machines, and performance.
- The reason Herdr writes screens in JavaScript is not stated in the binary; any reason given is a
  guess.
