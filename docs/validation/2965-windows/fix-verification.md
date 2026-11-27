# Windows verification of the #2965 follow-up

Ran the [same Windows recorder](README.md#reproduce-on-windows) against the fix in this PR, built on prerelease `865e0ade1862`. The recorder now brings the right-click target into view before the baseline, so Playwright's automatic scrolling does not count as a keyboard-menu effect.

| Windows Electron scenario | Conversation client / scroll width | Code client / scroll width | Scroll left / top before and after | Focus after editor Escape |
| --- | --- | --- | --- | --- |
| 1200×800 DIP; scale 1; body 100%; panel closed | 898 / 898 | 780 / 1875 | 0 / 0 | Conversation |
| 1024×701 DIP measured; forced scale 1.25; body 125%; Skills open | 204 / 204 | 156 / 1875 | 0 / 544.64 | Conversation |

Selection changes, keyboard-menu Escape, and editor Escape preserved geometry and scroll in both runs. The fake Gateway recorded no task mutations or model requests. Compact pending/attached snapshots differ because the recorder now scrolls to the menu target between those snapshots; this comparison is not an isolated kernel-attachment test.

Fresh synthetic-only PNGs and JSON receipts: `C:\Users\dorani\AppData\Local\Temp\akra-2965-fix-st01a21e78-validated`. The recorder writes both `before` and `after` captures plus the menu and editor frames. These original PNGs are not committed.
