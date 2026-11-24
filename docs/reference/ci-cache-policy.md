# Rust CI 캐시 정책

관련: #2690 단계 3. 실제 CI 캐시 API를 조회하거나 cold/warm 빌드를 실행하지 않고
정책과 계측 경로를 추가했다. 아래 정책이 실행 시간을 얼마나 줄이는지는 아직
측정되지 않았다. 기존 캐시를 삭제하거나 로컬 `cargo clean`을 실행하지 않는다.

## 다운로드와 컴파일 결과 분리

| 종류 | 저장 경로 | 키 경계 | fallback |
|---|---|---|---|
| Cargo 다운로드 | Cargo home의 registry/index, registry/cache, git/db | OS, 아키텍처, lockfile hash | 같은 OS·아키텍처의 다운로드 |
| Rust target | 기존 CARGO_TARGET_DIR 또는 target | OS, 아키텍처, lane, target 방식, features, profile, compiler, manifest/config hash, 빌드 환경 hash, target directory | 위 경계 전체가 같은 이전 source SHA |

`scripts/ci-cache.mjs plan`이 실제 `rustc --version --verbose` 결과와 workflow의
manifest·lockfile hash를 결합한다. implicit host 빌드와 `--target <triple>` 빌드는
다른 경계다. Linux 기본 feature 테스트, all-feature Clippy, portable native,
smoke는 각각 독립 lane이다. features/profile이 달라진 산출물을 broad
`OS-rust-` prefix로 되살리지 않는다.

컴파일에 영향을 주는 RUSTFLAGS, Rustdoc flags, CARGO_PROFILE_*, target linker/flags,
CARGO_INCREMENTAL, CARGO_BUILD_TARGET은 hash에 반영한다. 환경변수 원문은 artifact에
저장하지 않는다. manifest hash에는 Cargo manifests/lockfiles, rust-toolchain,
repository .cargo/config가 포함된다. Cargo가 fingerprint와 lockfile을 다시
검증하므로 cache hit가 검사를 생략할 근거가 되지 않는다.

Registry는 컴파일 산출물이 아니므로 lane과 feature가 달라도 공유한다.
정확한 lock 키가 맞아도 처음 저장한 job에 없던 optional crate는 Cargo가 추가로
내려받을 수 있다. registry hit를 의존성 다운로드가 전혀 없다는 뜻으로 해석하지 않는다.

## 갱신과 저장 주체

Target의 키 끝에는 source SHA를 붙인다. 같은 컴파일 조건에서 source가 바뀌면
이전 snapshot을 fallback으로 복원한 후, 성공한 새 source의 결과를 새 키로
저장할 수 있다. 동일한 immutable key의 오래된 결과만 계속 복원하는 구조를 피한다.
같은 source의 변경 없는 재실행은 exact snapshot을 재사용한다.

저장은 성공한 `main`·`prerelease` **push job**에서만 한다. PR·수동 실행·그 외
branch push는 restore-only이다. 따라서 PR merge ref마다 큰 target archive를 만들지
않으며, PR 대상이 기본 브랜치 main이 아닌 prerelease일 때도 해당 base branch push가
캐시를 공급한다. 임시 integration branch에 PR들을 모으는 작업은 캐시 저장 주체가
아니다. GitHub의 branch scope를 우회하거나 PR 캐시를 base branch에 승격하지 않는다.

Source가 바뀐 성공 push마다 lane별 target 키가 생길 수 있다. 키 수·압축 용량은
최종 Actions 자료와 서비스의 캐시 사용량으로 관찰해야 하며 이번 구현에서는 삭제 API,
임의 TTL purge, 다른 worktree의 target 공유를 추가하지 않는다. 자동 서비스 eviction
뒤 cache miss가 생기면 원래 Cargo 명령이 그대로 cold build를 수행한다.

`actions/cache/restore`와 `actions/cache/save`는 기존 저장소 pin을 사용한다.
Restore의 `fail-on-cache-miss`는 false이며 cache action 장애는 검증 명령 실행을
막지 않는다. 저장은 검증 성공 후에만 시도한다. 캐시 오류를 무시하는 설정은 cache
action에만 적용하며 Rust·Clippy·architecture·portable 검사에는 적용하지 않는다.

## 남기는 증거

`artifacts/ci-performance/`에 기존 CI 측정 결과와 함께 저장한다.

- `cache-policy.json`: 키, fallback, source SHA, 컴파일 경계, 저장 허용 여부.
- `cache-snapshots.jsonl`: 복원 직후와 job 완료 시점의 primary/matched key,
  exact/fallback/miss/unavailable, restore action 결과, save action 결과, 파일 수와 byte 수.
- `cache-summary.md`: 간단한 표. GitHub job summary에도 추가한다.

파일 크기는 symlink/junction을 따라가지 않고 계산한 **로컬 비압축 파일의 논리 크기**다.
압축 archive 크기·네트워크 전송량·GitHub 청구 비용과 같지 않다. 크기 순회 자체의
시간도 `sizeScanMilliseconds`에 기록한다. 저장 action의 success는 action 종료 상태이며
서비스가 새 archive를 실제로 보관했다는 별도 검증 결과로 표현하지 않는다.

복원·저장 step 시간과 workflow/job 시간은 마지막에 수집할 Actions jobs 자료에서
비교한다. `ci-performance.mjs`의 Cargo compiled-unit 관측 및 timings와 함께 보아
key hit, 재컴파일량, 복원/저장 비용을 구분한다. 이번 작업에서 CI 상태·원격 캐시 API를
중간 조회하지 않았다. 최종 검증 이후 동일 runner·compiler·features·profile·target별
cold/cache-reuse/unchanged/source-change 표본을 축적해야 개선 폭과 키 수명을 판단할 수 있다.

## 로컬 정책 검증

```bash
node --test scripts/ci-cache.test.mjs
```

Fixture는 cache boundary 변화, source snapshot 갱신, 저장 허용 이벤트, symlink를
따라가지 않는 byte 계측, exact/fallback/miss/error 구분을 검사한다. 네트워크 캐시나
실제 Rust 빌드는 사용하지 않는다.

## 근거

- [사용 중인 cache action의 restore 입력·출력](https://github.com/actions/cache/blob/caa296126883cff596d87d8935842f9db880ef25/restore/action.yml)
- [사용 중인 cache action의 save 입력](https://github.com/actions/cache/blob/caa296126883cff596d87d8935842f9db880ef25/save/action.yml)
- [Cache scope·version·immutable key 설명](https://github.com/actions/cache/blob/caa296126883cff596d87d8935842f9db880ef25/README.md)
- [GitHub cache 접근 범위](https://docs.github.com/en/actions/using-workflows/caching-dependencies-to-speed-up-workflows#restrictions-for-accessing-a-cache)

## 계측 증거의 현재성

복원된 target에 이전 Cargo timings HTML이 남을 수 있으므로 검증 전에 cargo-timings 디렉터리만 정리합니다.
컴파일 산출물과 dependency/incremental cache는 보존합니다. symlink/junction으로 연결된 target은 순회하지 않습니다.
이후 artifact의 timings는 이번 실행에서 생성된 자료이며, 단계별 JSON과 함께 확인합니다.
