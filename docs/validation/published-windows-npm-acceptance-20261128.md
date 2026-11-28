# Published Windows npm install and upgrade acceptance (#2589)

**Runtime passed; published npm notice inclusion did not pass.** This checks
the existing public `@refinedstone/akra` channel. It is not acceptance of an
unpublished Akra ADE package or a decision to publish one.

#3021 and #3024 checked the genuine v1.3.5 native archive and a local debug
candidate. Those runs did not install public npm packages. This receipt tests
the separate npm launcher and platform resolver. It does not supersede the
old-to-new schema or repository-identity findings in #3030 and #3038.

## Inputs

Windows x64 (10.0.26200), Node v24.13.0, npm 11.12.1. The four public registry
tarballs below matched their published SHA-512 integrity pins when downloaded
and inspected independently. npm recorded the same integrities for the
installed launcher and Windows companion in each lockfile.

| `@refinedstone/akra` registry version | `sha512-` integrity (base64 suffix) |
| --- | --- |
| `1.3.3` | `fTgr5l7eP62dj/5YvoKntyfXEl+5Q3znm6mYI+bv3/6Q4MfRsP4tebUOSZyk4cpTXhRsNOP+az6RSMEhMmqbgQ==` |
| `1.3.3-win32-x64` | `PbBAerEXJfTjoSnRPvsv5AdzqFVrrKk+KHdF+js8hWTD0OMAfsQp4YXCteHj4cZM/QVpqjVIxjEbxc6tREHt2Q==` |
| `1.3.5` | `is7aqCAuqjakU2vd6lAz7NRRnpQMvEs2TLbrp/G9uy0K6VNMSXG1ctIVQONS2uaQ8jRlbzBxLEahhLOVzojWsQ==` |
| `1.3.5-win32-x64` | `9t+GDe6PgAJfw2k9tpb67qO4Dv4+kpFT+GWua3/54oA8b4W46EDAui2KSmvIt9l1vOQ3hL+3UfjwoApnEiAP+w==` |

## Executed checks

Use separate, empty user and global npm config files. Set npm's cache and the
home, AppData, temp and `AKRA_HOME` paths to disposable locations. Run outside
the checkout with only the Node and System32 directories on `PATH`. Both the
prefix and working directory included spaces and Korean characters.

```text
npm install --prefix <upgrade-prefix> --save-exact --ignore-scripts --no-audit --no-fund @refinedstone/akra@1.3.3
npm install --prefix <upgrade-prefix> --save-exact --ignore-scripts --no-audit --no-fund @refinedstone/akra@1.3.5
npm install --prefix <fresh-prefix>   --save-exact --ignore-scripts --no-audit --no-fund @refinedstone/akra@1.3.5
```

Each install resolved only the main package and the `@refinedstone/akra-win32-x64`
alias. The installed executable's SHA-256 matched the executable from the
corresponding verified npm tarball. These are npm payload checks, not claims that
the platform packages match the GitHub release archives byte for byte.

| Scenario | Observed result |
| --- | --- |
| Fresh 1.3.3 in the upgrade prefix | Generated `node_modules/.bin/akra.cmd` ran `--help` and `doctor`. `planning-tool run` created and read one synthetic task. |
| Upgrade to 1.3.5 in the same prefix and profile | The same shim path ran `--help`. Before any new task write, `list_tasks` returned the exact original task row, including its ID, title and description. |
| Fresh 1.3.5 in a separate prefix and profile | The generated shim ran `--help` and `doctor`. `list_tasks` returned an empty list. |
| Project terms and third-party notice inclusion | Each of the four tarballs contained five regular files. None shipped `LICENSE`, `THIRD_PARTY_NOTICES`, or other license/notice-named files. None declared a `license` field in its manifest. |

The historical payload result does not establish a legal conclusion or relax
the current staging requirements. The current staging script copies the project
license and platform notices. Verify inclusion in the exact new candidate
tarballs before any publication decision.

This receipt covers only the historical Windows x64 npm channel. It does not
cover Linux, macOS, Desktop installers, real credentials, Gateway, settings
other than the synthetic task profile, or installation of a release-profile
Akra ADE candidate. The fixture removed its temporary profiles, prefixes and
cache. It did not change machine-wide installs, release settings or registry
contents. No model turns ran.
