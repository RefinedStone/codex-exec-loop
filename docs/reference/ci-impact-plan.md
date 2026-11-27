# 공통 crate 영향 계획

관련: [#2904](https://github.com/RefinedStone/codex-exec-loop/issues/2904)

소유권: [crate 경계](ci-crate-boundaries.md), [registry](../../scripts/ci-crate-ownership.json)

구현: [ci-impact-plan.mjs](../../scripts/ci-impact-plan.mjs)

## 역할과 안전 조건

`ci-impact-plan.mjs`는 base/head의 Cargo metadata, 파일 소유권 registry, 변경 목록을
받아 결정적인 JSON 검증 계획을 만든다. Cargo/Git를 실행하거나 checkout, Cargo.lock,
작업 파일을 변경하지 않는다. 입력 수집과 실제 실행은 별도의 호출자 책임이다.

현재 root의 consumer 시험 소유권은 `full`이다. root가 컴파일 또는 행동 영향 집합에
포함되면 `mode: "full"`, `root-consumer-requires-full`을 반환해 기존 전체 gate를
유지한다. root의 고비용 통합시험을 임의로 생략하지 않는다.
독립 leaf만 바뀌고 어느 snapshot에서도 root가 소비하지 않는다는 근거가 있을 때
선택 계획을 만들 수 있다.

v1은 선택된 package마다 전체 시험을 유지한다. 계약 필터는 추가로 보존하는 의무이며
다른 시험을 제외하는 근거가 아니다. 소유권 registry에 `consumerCoverage: "contracts"`
같은 검증되지 않은 값을 추가하면 full로 전환한다.
전체 CI 속도가 개선됐다는 주장은 이 계획기 구현에 포함하지 않는다.

## 입력 API

```javascript
import {
  createValidationPlan,
  explainPlan,
  parseGitNameStatusZ,
  verifyValidationPlanHash,
} from "./scripts/ci-impact-plan.mjs";
```

`createValidationPlan(request)`의 입력 형식은 다음과 같다.

| 필드 | 형식과 의미 |
| --- | --- |
| schemaVersion | 정수 1 |
| base, head | 각각 `{sha, metadata, ownership}` |
| sha | snapshot을 읽은 정확한 commit SHA. head 값이 결과의 sourceSha가 됨 |
| metadata | Cargo format-version 1의 전체 출력. resolve 포함 |
| ownership | 같은 snapshot의 ci-crate-ownership.json |
| changes | `[{status, path, oldPath?}]`; status는 A/M/D/R/C/T/U |
| localChangeId | 로컬 변경 내용 식별자 또는 clean일 때 null. 경로 목록만 hash해서는 안 됨 |
| configuration | platforms, features, profile, targetTriples |

configuration 기본값은 다음과 같다.

```json
{
  "platforms": ["linux", "macos", "windows"],
  "features": ["all", "default"],
  "profile": "test",
  "targetTriples": []
}
```

지원 feature 모드는 `all`, `default`다. 다른 mode, 알 수 없는 configuration 필드,
잘못된 profile/target은 명확한 오류가 된다. 배열은 정렬하고 중복을 제거한다.

입력 수집자는 각 snapshot에서 다음 명령으로 metadata를 얻는다.

```text
cargo metadata --format-version=1 --locked --all-features
```

이미 필요한 registry 자료가 있다면 `--offline`을 추가할 수 있다.
`--no-deps`로 만든 출력은 전체 graph가 아니므로 full 전환 대상이다.
플랫폼 조건을 빠뜨리지 않도록 `--filter-platform`을 사용하지 않는다.
base를 읽으려고 사용자의 현재 checkout을 과거 commit으로 바꾸지 않는다.

Cargo의 package ID는 snapshot 내부에서만 비교하는 불투명 키로 처리한다.
출력의 공통 packageKey는 `<repository 상대 manifest>::<package name>`이다.
예를 들어 `crates/issue-core/Cargo.toml::akra-issue-core`다.
두 snapshot에서 manifest가 이동했으면 고유한 package name 대응도 추적해
이전 graph와 새 graph의 소비자를 함께 선택한다.
실제 상대경로를 만들 때 Windows와 POSIX 경로 표기 차이를 정규화하며,
`..`, 절대경로, NUL이 포함된 변경 경로를 선택 계획에 사용하지 않는다.

변경 목록은 `git diff --name-status -z --find-renames`의 출력을
`parseGitNameStatusZ`에 전달해 만들 수 있다. newline 기반 파싱과 달리 공백·탭이 있는
이름과 rename 양쪽 경로를 보존한다. 로컬 수집자는 commit 변경뿐 아니라
staged/unstaged/untracked 변경과 내용 식별자도 합쳐야 한다.

## 영향 전파

컴파일 영향은 변경 경로의 base/head 소유자를 시작점으로 삼는다.
base/head 양쪽의 normal/dev/build 역의존을 합쳐 소비자를 선택한다.
optional feature와 Windows/macOS 조건부 edge의 조건은 graph hash에 기록하며,
조건을 추측해서 edge를 제거하지 않는다. 활성 feature 전체를 확인할 수 없으면 full이다.

Cargo resolve의 `deps[].pkg`와 `dep_kinds`, manifest 선언의 path/rename/optional/target을
함께 사용한다. 선언된 workspace path edge도 보존하므로 플랫폼 필터에 의해 빠진
resolve edge로 소비자가 사라지지 않는다. package 삭제, 의존 제거, 경로 이동에서도
base의 소비자를 잃지 않는다. 순환 graph는 full로 처리하며 탐색을 중단한다.

행동 시험 영향은 컴파일 영향과 별도로 계산한다.
선택된 package와 Cargo 소비자의 전체 시험을 포함하고, Cargo 밖의 계약도 전파한다.
registry의 기존 계약 형식에 다음 필드를 선택적으로 추가할 수 있다.

| 계약 필드 | 의미 |
| --- | --- |
| providers | 이 계약이 검증하는 package name 목록. Cargo 의존이 없어도 행동 영향 전파 |
| inputs | 계약 fixture/schema/프로토콜 파일의 정확한 경로 또는 끝이 /인 prefix |

예를 들어 별도 플러그인 계약이 leaf DTO를 검증한다면 leaf 변경은 compilePackages에
leaf를, testPackages에는 leaf와 계약 소유자를 포함할 수 있다.
testPackages가 compilePackages보다 넓을 수 있으며 실행기는 시험에 필요한 대상도
빌드해야 한다. 계약 필터의 OS/features/isolation은 contractTests에 보존한다.

일반 소스 prefix뿐 아니라 registry의 externalInputs도 소유권으로 처리한다.
Askama `templates/admin/*.html`은 기존 Rust+Node 영향을 유지한다.
DTO·직렬화·이벤트·IPC·플러그인 fixture 및 생성 입력의 소유자가 없으면 full이다.

## 출력 계약

| 필드 | 의미 |
| --- | --- |
| schemaVersion | 계획 형식 버전 1 |
| sourceSha, baseSha, localChangeId | 사용한 source/base/로컬 내용 식별 |
| configuration | 정규화된 OS, feature mode, profile, target triple |
| mode | full 또는 selective |
| fullReasons | 전체 검증이 필요한 이유. 빈 목록이면 selective |
| compilePackages | 컴파일 영향 package와 target 집합 |
| testPackages | 행동 시험 package와 target 집합. coverage는 full |
| contractTests | 보존할 package/target/filter/OS/features/isolation 계약 |
| nodeRequired, docsOnly | 기존 Node/문서 영향 유지 |
| removedPackages | 사라진 이전 package와 대응되는 새 packageKey |
| excludedPackages | 제외된 현재 package 및 제외 이유 |
| ownership, graphs | 각 schema version, base/head revision 또는 hash |
| explanations | 경로별 소유권 판정 근거 |
| planHash | planHash 자신을 제외한 출력 전체의 canonical SHA-256 |

compilePackages와 testPackages의 요소는 다음 형식이다.
`coverage`는 testPackages에만 있다.

```json
{
  "packageKey": "crates/issue-core/Cargo.toml::akra-issue-core",
  "name": "akra-issue-core",
  "manifest": "crates/issue-core/Cargo.toml",
  "targets": [
    {
      "name": "akra_issue_core",
      "kind": "lib",
      "requiredFeatures": [],
      "doctest": true,
      "test": true
    }
  ],
  "coverage": "full",
  "reasons": ["head:input:crates/issue-core/src/issue_validation.rs"]
}
```

target의 kind와 name을 함께 식별한다. 이름이 같은 lib와 bin을 합치지 않는다.
custom-build는 Cargo가 선택 대상의 빌드에 자동 포함하므로 직접 실행 target 목록에서 제외한다.
doctest와 requiredFeatures도 버리지 않는다.

sourceSha/baseSha/localChangeId, configuration, 소유권 및 graph hash는 모두 planHash에
들어간다. timestamp, 임시 checkout 절대경로, Cargo ID 문자열의 내부 표기를 계획에 넣지 않는다.
같은 의미의 metadata/registry 배열 순서나 Windows/POSIX 경로 구분자로 선택 결과가
달라지지 않는 fixture가 있다.

`verifyValidationPlanHash(plan)`는 저장된 payload와 hash의 일치 여부만 검사한다.
최신 checkout과 동일한지 또는 입력이 신뢰할 만한지는 증명하지 않는다.
실행자는 실행 직전에 source와 로컬 입력을 다시 수집해 계획을 재생성하고 비교해야 한다.

## full 및 오류

다음 경우에는 선택 성공으로 처리하지 않고 명시적인 full 계획을 만든다.

- base snapshot/resolve 누락, 불완전한 optional feature graph
- 등록되지 않은 workspace member, planned owner, 불완전한 consumer 소유권
- root와 모든 member의 Cargo.toml/Cargo.lock, toolchain/빌드 정책 변경, 미등록 경로·생성 입력
- 알 수 없는 dependency kind, 순환, 누락된 resolve node
- 비어 있거나 잘못된 변경 목록, 충돌 중인 경로
- root 소비자 영향 또는 실행할 시험 target이 확인되지 않은 package

head member/target 목록 자체가 없거나 사용할 수 없으면 full 명령의 대상도 확정할 수
없으므로 오류로 종료한다. 빈 package 목록을 정상 검증 성공으로 내보내지 않는다.
문서 또는 알려진 Node 전용 변경은 docsOnly/nodeRequired로 그 목적을 명시한다.

## CLI와 검증

```text
node scripts/ci-impact-plan.mjs --input request.json --explain
node scripts/ci-impact-plan.mjs --input -
node --test scripts/ci-impact-plan.test.mjs
```

CLI는 항상 JSON을 stdout으로 내보낸다. `--explain`의 사람이 읽는 근거는 stderr로
출력하므로 JSON을 별도 저장할 수 있다. 잘못된 입력은 stderr 설명과 nonzero exit를 반환한다.

정책 fixture는 추가/삭제/rename/package 이동, base의 삭제된 의존, normal/dev/build,
optional·OS 조건, 순환, 불투명 package ID, 동명 target, 불완전 metadata, 공용 계약,
미등록 입력, 혼합 Rust/Node, hash와 CLI 읽기 전용 동작을 검증한다.
전체 CI나 실제 모델 턴을 실행해서 계획기를 시험하지 않는다.

2026-11-27 검증에서 `4983131b3a519504562494c86d339ce30f9cacbb`의 실제 workspace
metadata에 합성 issue-core 변경을 입력했다. 결과는
`root-consumer-requires-full` 한 가지 사유의 full 계획이며
root 15 targets, issue-core 1 target, 기존 4개 계약을 포함했다.
실제 소스 변경이나 성능 측정 결과를 뜻하지 않는다. Cargo.lock과 manifest는 변경되지 않았다.

Cargo 형식의 정의는 [공식 cargo metadata 문서](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)를 따른다.
