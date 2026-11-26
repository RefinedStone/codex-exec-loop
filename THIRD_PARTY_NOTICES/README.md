# Third-party notices

Akra ADE ships these third-party components. Each keeps its own license. The project license in
[`../LICENSE`](../LICENSE) does not limit the rights that these licenses give.

| Component | Where it is used | License | Notice |
| --- | --- | --- | --- |
| Rust crates linked into the release binaries | all native binaries | MIT, Apache-2.0, and other permissive licenses | [`rust-crates.txt`](rust-crates.txt) |
| Galmuri 11 font v2.40.3 | Admin UI | SIL Open Font License 1.1 | [`../assets/admin/fonts/LICENSE-Galmuri.txt`](../assets/admin/fonts/LICENSE-Galmuri.txt), shipped as `Galmuri-OFL.txt` |
| PixiJS | Admin diorama bundle | MIT | [`PixiJS-MIT.txt`](PixiJS-MIT.txt) |

Native archives and npm packages carry this directory as `THIRD_PARTY_NOTICES/`, with `LICENSE` at
their root.

`rust-crates.txt` is generated. After a dependency change, run:

```text
node scripts/third-party-notices.mjs
```

`node scripts/third-party-notices.mjs --check` fails when the committed file is out of date.
Both fail when a shipped crate has no license text in its package. For such a crate, add the
upstream license file at the crate's published commit to
[`../scripts/rust-license-fallbacks/`](../scripts/rust-license-fallbacks/index.json), keyed by
crate name and version.
