# 네이티브 패키징과 배포

[English](../../plan/13-native-packaging-and-operator-runbook.md)

이 문서는 Rust client의 현재 operator, bundle, publication 계약입니다.

## Bundle 빌드

```bash
./scripts/package_native_release.sh
./scripts/package_native_release.sh --target aarch64-apple-darwin
./scripts/package_native_release.sh --profile debug
```

기본 profile은 `release`입니다. Windows bundle은 검증하지 않은 cross-link보다 native packaging을
사용합니다. `SOURCE_DATE_EPOCH`이 있으면 timestamp를 고정하고, 없으면 checkout한 `HEAD` timestamp를
사용합니다. 모든 target은 `.tar.gz` 형식을 유지합니다.

```text
dist/native/
  codex-exec-loop-native-<version>-<target>/
  codex-exec-loop-native-<version>-<target>.tar.gz
  codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

Bundle에는 native binary, launcher, app-server skill, `scripts/gh-akra.sh`, `README.md`,
`OPERATOR.md`, font notice, `VERSION.txt`, 보호된 예제 prompt, `SHA256SUMS.txt`가 포함됩니다. 예제
prompt는 자동 실행하지 않습니다.

npm 배포는 `@refinedstone/akra`와 platform optional dependency를 사용합니다. JavaScript launcher는
Rust binary를 선택해 실행하기만 합니다.

## 무결성 검증

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target> \
  --version <version> \
  --target <target> \
  --profile release
```

Node.js 18 이상이 필요하고 archive policy 검증은 `tar`도 필요합니다. Bundle manifest는 unpacked
file을, 옆의 `.sha256`은 archive를 hash합니다. 정렬된 `ustar` member, 정규화 metadata, 고정 epoch,
timestamp 없는 gzip header로 같은 source/binary 입력은 재현 가능해야 합니다.

## 운영 사전 조건과 실행

- 신뢰하는 `PATH`의 공식 Codex CLI와 완료된 `codex login`
- 대상 workspace 및 Codex session/history 접근
- Linux sandbox helper: Codex 설치본의 `codex-resources/bwrap` 또는 system `bwrap`
- Unix에서는 private `$CODEX_HOME`; Akra는 upstream auth/rollout file mode를 수정하지 않음

Bundle 빌드 후 운영 machine에는 Rust가 필요하지 않습니다.

```bash
cd /path/to/workspace
akra
```

Windows PowerShell에서도 workspace로 이동해 `akra`를 실행합니다. Bundle launcher는 `akra.cmd`이며
native binary를 직접 실행할 수도 있습니다.

주요 설정:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`
- `AKRA_GITHUB_LOGIN=<login>` 또는 repo-local `akra.githubLogin`
- 신뢰하는 `gh auth token`을 사용할 수 없을 때만 `AKRA_GITHUB_TOKEN=<token>`

GitHub 쓰기 전 `bash scripts/gh-akra.sh auth write-status`를 실행합니다. 제거된 legacy credential
scan은 지원하지 않으며 GitHub API helper는 repository credential helper나 credential file을 직접
읽지 않습니다.

## Smoke check

1. Linux에서는 `codex sandbox -C "$PWD" -P :read-only /usr/bin/true` 통과를 확인합니다.
2. 실제 workspace에서 Akra를 실행하고 시작 진단을 확인합니다.
3. 이전 session 또는 새 draft를 열고 prompt 하나를 streaming합니다.
4. Planning이 범위에 있으면 `:planning`, `:queue`를 엽니다.
5. GitHub polling을 설정했다면 기대한 review 상태를 확인합니다.
6. 종료 후 terminal restore를 확인합니다.

Platform 변경은 [검증 가이드](validation.md)에 따라 evidence를 기록합니다.

## Publication 계약

Tag workflow는 `.github/workflows/release-native-assets.yml`입니다.

- Stable `vMAJOR.MINOR.PATCH` tag만 허용합니다.
- Resolve한 tag commit은 `GITHUB_SHA`와 같고 fetched `origin/prerelease`에 포함되며
  `Cargo.toml` version과 일치해야 합니다.
- Linux x64, Windows x64, macOS Apple Silicon을 배포합니다.
- 모든 upload는 archive와 checksum을 포함하며 mutation 전에 exact asset, 안전하고 제한된
  extraction, regular member type, archive identity를 검증합니다.
- 보호된 `npm-release` environment가 `NPM_TOKEN`과 reviewer/tag policy, 최소 registry scope를
  제공합니다. Repository rule은 `v*` tag 생성·이동·삭제를 제한해야 합니다.
- Platform package를 main package보다 먼저 publish합니다. Immutable version은 먼저
  `release-staging` dist-tag를 사용하고 이후 `latest`와 `platform`을 뒤로 이동하지 않게 수렴시킵니다.
- Package마다 검증한 `npm pack --json` tarball 하나, `--provenance`, 고정 npm registry route,
  lifecycle script 비활성화를 사용합니다. Token은 auth/publish step에만 노출합니다.
- 같은 tag rerun은 이미 publish된 version과 같은 이름의 release asset을 byte-for-byte 확인하고
  빠진 mutation만 완료합니다.
- GitHub Release는 모든 npm package가 성공한 뒤 생성합니다. 일부 npm publish 후 실패했다면 외부
  원인을 해결하고 같은 tag를 다시 실행합니다.
- npm과 GitHub publication은 byte-atomic하지 않습니다. 대신 identity-pinned, immutable,
  retryable, convergent하게 설계합니다.

Pinned Rust toolchain의 clean checkout에서 package하고 exact source SHA와 archive checksum을
보존하며, publish된 asset을 한 번 내려받아 최종 검증합니다.
