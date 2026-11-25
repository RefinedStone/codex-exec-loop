# 32. Akra ADE 소스 이용정책 감사와 결정안

- 상태: **결정됨 (2026-11-26)**, 기여 조건만 미결정. 적용은 L2(#2586)가 한다.
- 관련: [L1 #2585](https://github.com/RefinedStone/codex-exec-loop/issues/2585),
  [L2 #2586](https://github.com/RefinedStone/codex-exec-loop/issues/2586),
  상위 [#2584](https://github.com/RefinedStone/codex-exec-loop/issues/2584)
- 조사 기준: `origin/prerelease@48b6d49a3`, 2026-11-26
- 이 문서는 사실 감사와 결정 선택지다. 법률 자문이 아니다. 권리 판단이 불명확한 항목은
  전문 검토 대상으로 표시한다.

## 1. 이미 정해진 방향

사용자 결정(#2584)을 그대로 옮긴다.

- 소스를 공개 상태로 유지한다.
- 권리자의 사전 허락 없는 수정을 제한한다.
- 추후 유료 판매와 비공개 개발 선택지를 남긴다.
- 이 정책은 OSI 의미의 오픈소스가 아니다. 공식 문서에서는 **소스 공개형(source-available)**으로
  설명한다.

## 2. 감사표

상태 값: **확인됨** / **확인 못 함** / **해당 없음**. 검색 결과가 없다는 것을 미배포의 증명으로
쓰지 않는다.

### 2.1 기존 라이선스 선언과 배포본

| 대상 | 식별자 | 라이선스 선언 | 상태 |
| --- | --- | --- | --- |
| Git tag `v1.0.0` ~ `v1.3.5` (19개) | 각 tag의 tree | LICENSE 파일 없음. 루트 `package.json`, `npm/package.json`, `Cargo.toml`에 license 필드 없음 | 확인됨 (tag마다 `git ls-tree`와 manifest 확인) |
| GitHub Release `codex-exec-loop-native v1.0.0` ~ `v1.3.5` | 최신 `v1.3.5`, 2026-06-20 | archive 구성 스크립트에 LICENSE·고지 파일 포함 단계 없음 | 스크립트 기준 확인됨. 게시된 archive 내부는 열어 보지 않음 |
| npm `@refinedstone/akra` | 1.2.6, 1.2.7, 1.2.8, 1.2.9, 1.3.0, 1.3.1, 1.3.3, 1.3.5 (+ 플랫폼 접미사 버전) | registry의 `license` 필드 비어 있음 | 1.3.5는 확인됨. 나머지는 범위 조회 결과가 비어 있음 |
| 루트 `package.json` `"license": "ISC"` | 최초 커밋 `92690fe50` (2026-07-06, `chore(test): add playwright scaffold`) | ISC | 확인됨. `prerelease`에만 있고 `main`과 모든 릴리스 tag에는 없음. tag `start-revision`(2026-07-11)에도 있음 |
| npm 배포 대상 | `.github/workflows/release-native-assets.yml`, `npm/scripts/stage-npm-packages.mjs` | staging이 만드는 `@refinedstone/akra` manifest에 license 필드 없음 | 확인됨. workflow는 `npm/` staging 결과만 배포하고 루트 manifest는 배포하지 않는다 |
| crates.io | `Cargo.toml` | license 필드 없음 | workflow와 스크립트에 `cargo publish` 단계 없음. registry는 조회하지 않음 |
| Desktop | `desktop/package.json` | `"private": true`, `0.0.0-development` | 해당 없음 (배포판 없음) |

**해석 메모 (결정 아님)**

- ISC 값은 Playwright scaffold를 만들 때 `npm init` 기본값으로 들어간 것으로 보인다. 이 추정은
  커밋 내용에서 나온 것이며 의도를 확인하지 않았다.
- 그래도 2026-07-06 이후 공개된 `prerelease` 스냅샷에는 ISC 선언이 있다. 이 기간에 코드를 받은
  사람이 ISC 권리를 주장할 수 있는지는 **전문 검토 대상**이다. 새 조건은 이 권리를 소급해서
  회수한다고 쓰지 않는다.
- 릴리스 tag와 npm 배포본에는 라이선스 선언이 없다. 선언이 없는 공개 코드의 이용 범위도 전문
  검토 대상이다.

### 2.2 기여와 작성자

| 대상 | 결과 | 상태 |
| --- | --- | --- |
| 병합된 PR 작성자 | 조회한 1,000건 모두 `RefinedStone` | 확인됨 (조회 상한 1,000건) |
| 커밋 작성자 | `RefinedStone` 계정 식별자, 자동화 시험용 식별자(`Akra Test`, `akra@localhost`) | 확인됨 |
| `Co-authored-by` trailer | AI 도구 이름(Claude 계열, `sisyphus-dev-ai`) | 확인됨. AI 도구가 만든 코드의 권리 귀속은 **전문 검토 대상** |
| 외부 기여자 | 발견하지 못함 | 확인 못 함 (fork 이력과 PR 밖 기여는 조사하지 않음) |

### 2.3 제3자 구성요소

| 구성요소 | 위치 | 라이선스 | 고지 상태 |
| --- | --- | --- | --- |
| Galmuri 11 폰트 v2.40.3 | `assets/admin/fonts/` | SIL OFL 1.1 | 원문 `LICENSE-Galmuri.txt`와 README 있음 |
| PixiJS | `assets/admin/game/akra-diorama.js`, `assets/admin/game/scripts/promote-build.mjs` | MIT | 파일 머리에 저작권·허락 문구 있음 |
| Codex app-server protocol 생성물 | `schema/`, `desktop/src/shared/protocol/generated/` | upstream `openai/codex` 기준 | 생성물에 필요한 고지는 **확인 못 함** |
| Rust 의존성 | `Cargo.lock` 477개 package | 모두 허용형 계열: MIT/Apache-2.0 이중 (약 290), MIT (118), Unicode-3.0 (18), CDLA-Permissive-2.0 (3), ISC, Zlib, BSD, Unlicense, MIT-0, WTFPL, BSL-1.0 선택 | 배포 archive에 제3자 고지 파일 없음. **L2·R1에서 고쳐야 할 공백** |
| Desktop npm 의존성 | `desktop/package-lock.json` | — | 해당 없음 (Desktop 배포판 없음) |

Rust 수치는 `cargo metadata` 전체 lock 기준이다. 실제 배포 바이너리에 들어가는 package만 따로
세지 않았다. LGPL은 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` 같은 선택형으로만 나타난다.

## 3. 이용 행위별 권한표 (결정안)

**고정**은 1장의 사용자 결정이나 GitHub 약관에서 나온다. **결정 필요**는 사용자가 고른다.

| 행위 | 기본안 | 근거 | 상태 |
| --- | --- | --- | --- |
| 열람, GitHub 안에서 fork | 허용 | 공개 저장소에서는 GitHub 약관이 이 행위를 허용한다 | 고정 |
| 원본 그대로 빌드·실행 (개인) | 허용 | 소스 공개 방향 | 결정 필요 (A) |
| 원본 그대로 업무용 사용 | 허용 | #2585: Noncommercial 조건을 자동으로 채택하지 않는다 | 결정 필요 (A) |
| 소스 수정, 파생물 제작 | 사전 서면 허락 필요 | 사용자 결정 | 고정 |
| 사용자 설정·플러그인·생성 결과물 | 제품 소스 수정으로 보지 않음. 자유 | 제품 사용과 소스 수정을 구분한다 | 결정 필요 (B) |
| 원본 재배포 | 사전 허락 필요 | 유료 판매 선택지 유지 | 결정 필요 (C) |
| 수정본 재배포·판매 | 금지. 별도 계약으로만 허용 | 사용자 결정 | 고정 |
| PR 기여 | 기여 목적의 수정만 허용. 기여 조건 동의 필요 | 수정 제한과 PR 수용을 함께 유지 | 결정 필요 (D) |
| 이름·로고 (Akra, Akra ADE) | 사전 허락 필요 | 브랜드 보호 | 고정 |

### 결정 선택지

**(A) 원본 빌드·실행 범위**

1. 개인·업무용 모두 허용 (권장). 수정 제한만으로 유료화 선택지는 남는다.
2. 개인·평가용만 허용, 업무용은 별도 계약. 유료 판매에 유리하다. 하지만 #2585가 경계한
   Noncommercial 조건과 비슷해진다.

**(B) 사용자 확장 범위**

1. 설정, 저장소 확장(`kernel-extensions`), 스킬, 생성 결과물은 자유 (권장).
2. 확장 배포에도 허락 필요.

**(C) 원본 재배포**

1. 수정하지 않은 원본과 이용조건 원문을 함께 배포할 때만 허용, 판매는 금지.
2. 모든 재배포에 사전 허락 필요 (권장. 공식 배포 경로를 하나로 유지한다).

**(D) 기여 조건**

1. CLA: 기여자가 권리자에게 재라이선스 권한을 부여한다 (권장. 유료·비공개 선택지를 지킨다).
2. 저작권 양도 계약.
3. 외부 PR을 받지 않고 이슈 제보만 받는다.

DCO만으로는 저작권 양도나 포괄적 재라이선스 권한을 얻지 못한다. 그래서 DCO는 선택지에 넣지
않았다.

## 4. 문안 방식 선택지

| 방식 | 맞는 점 | 맞지 않는 점 |
| --- | --- | --- |
| 자체 문안 "Akra ADE Source License" | 3장의 권한표를 그대로 표현할 수 있다 | 문안 검토 비용이 든다. SPDX 식별자가 없으므로 manifest는 `SEE LICENSE IN LICENSE`를 쓴다 |
| PolyForm Strict 1.0.0 | 수정·배포 금지가 이미 들어 있다 | 비상업 목적만 허용한다. (A)-1과 충돌한다 |
| PolyForm Internal Use 1.0.0 | 업무용 사용을 허용한다 | 내부 목적의 수정을 허용한다. "수정 제한"과 다르다 |
| Business Source License 1.1 | 소스 공개형으로 널리 쓰인다 | 수정을 허용하고 일정 기간 뒤 오픈소스로 바뀐다. 결정 방향과 다르다 |

권장: **자체 문안**. 기존 표준 문안 중 3장 기본안과 그대로 맞는 것을 찾지 못했다. 문안은
전문 검토를 거친 뒤 L2에서 적용한다.

## 5. 적용 범위와 고지 방법 (L2 입력)

- 새 조건의 시작점은 L2를 병합하는 커밋과 그 뒤의 첫 릴리스 버전으로 정한다.
- 시작점 이전의 tag, 릴리스, npm 버전, `prerelease` 스냅샷은 다시 쓰지 않는다. 그 기간의 권리는
  2.1의 해석 메모대로 전문 검토 결과를 따른다.
- 루트 `package.json`의 `"license": "ISC"`는 L2 전까지 바꾸지 않는다.
- L2에서 할 일:
  - 루트 `LICENSE` 파일 추가와 각 manifest의 license 참조 정렬
  - `THIRD_PARTY_NOTICES` 작성 (2.3 표 기준)
  - 배포 archive와 npm package에 두 파일을 포함 (R1에서 산출물로 확인)
  - `CONTRIBUTING.md`에 (D)의 결과 반영
  - 허락 요청 경로: GitHub 이슈 템플릿 또는 연락 주소 (결정 필요)

## 6. 사용자 결정 (2026-11-26)

| 항목 | 결정 |
| --- | --- |
| (A) 원본 빌드·실행 | 개인·업무용 모두 무료로 허용 |
| (B) 사용자 확장 | 자유롭게 사용. 제품 소스를 담지 않은 설정·확장·스킬·생성 결과물은 사용자 것이다 |
| 소스 수정 | 자기 비상업적 목적의 수정만 허용. 수정본의 배포와 상업적 이용은 금지 |
| (C) 원본 재배포 | 사전 서면 허락 필요 (GitHub fork 제외) |
| (D) 기여 조건 | **미결정**. 정할 때까지 외부 PR을 받지 않고 이슈 제보만 받는다 |
| 문안 방식 | 자체 문안 `Akra ADE Source License 1.0` |
| 권리자 | akra dev studio (개인사업자) |
| 허락 요청 경로 | 이 저장소의 GitHub 이슈 |
| 전문 검토 | 결정하지 않음. 문안은 검토 전 초안으로 적용한다 |

해석 메모: "수정 배포 절대 금지"와 "수정본 상업적 이용 금지"를 함께 적용했다. 그래서 회사 안에서
수정본을 업무에 쓰는 것도 상업적 이용으로 보고 금지한다.

## 7. 결정 전 확인 항목 (기록)

1. (A) ~ (D)의 선택
2. 문안 방식 (자체 문안 권장)
3. 2.1의 ISC 기간과 무선언 배포본에 대한 전문 검토 여부
4. 허락 요청 경로

이 네 가지가 정해지면 L1을 완료하고 L2를 시작할 수 있다.
