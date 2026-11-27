# 변경 없는 Rust 입력의 캐시 재사용 조사

관련: [#2866](https://github.com/RefinedStone/codex-exec-loop/issues/2866),
[#2867](https://github.com/RefinedStone/codex-exec-loop/issues/2867),
[#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911).

기준 소스: `caf1da09aa561d1510b3271cc663b8e5bf3a0690`.
조사일: 2026-11-27. 이 문서는 구현 및 재현 근거다. 전체 Akra CI의 개선율이나
#2866의 실제 CI 인수 완료를 선언하지 않는다.

## 이미 확인된 것과 남은 원인 확인

[기존 65건 분석](ci-build-cost.md)은 다음을 구분했다.

- 의존 crate는 캐시에서 재사용됐다. fallback 복원을 캐시 전체 실패로 볼 수 없다.
- Rust 입력을 수정하지 않은
  [push 45563935880](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45563935880)도
  본 crate를 다시 빌드했다. 기존 로그에는 Cargo의 dirty 사유가 없어 원인을 확정하지 못했다.
- [최신 통합 실행 45936932223](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223)의
  Windows job은 60분 제한을 넘었다. 이는 #2864의 느린 시험 실행과 별개로 다루며,
  컴파일 캐시 수정만으로 그 시간 초과가 해결됐다고 판단하지 않는다.

Cargo 1.95.0의 같은 경로·같은 명령·같은 컴파일 조건에서, 소스 바이트는 그대로 두고
checkout처럼 mtime만 새로 만들면 실제 `cargo -v` 출력에 다음 사유가 나타났다.

~~~text
Dirty akra-cache-inputs-proof ...: the file src/lib.rs has changed (... after last build ...)
~~~

같은 파일의 내용 해시를 확인하고 이전 입력 mtime를 복원하면 `Fresh`가 됐다.
실제 Rust 내용, `include_str!` 입력, build script의 환경 입력을 바꾸면 다시 `Dirty`가
되고 새 값을 검증하는 시험이 통과했다. 이는 **mtime 원인과 보정의 실제 Cargo 재현**이며,
기존 원격 실행의 유일한 원인을 소급 확정하는 증거는 아니다.

Cargo의 [checksum-freshness](https://doc.rust-lang.org/cargo/reference/unstable.html#checksum-freshness)는
아직 unstable 기능이다. 이번 구현은 stable toolchain과 Cargo의 자체 fingerprint 검사를 유지한다.
build script의 [입력 변경 감지](https://doc.rust-lang.org/cargo/reference/build-scripts.html#change-detection)도
그대로 수행된다.

## 구현 계약

`scripts/ci-cache-inputs.mjs prepare`는 target 복원 뒤, 첫 Cargo 명령 전에 실행한다.

1. 같은 checkout 절대 경로와 기존 캐시 컴파일 경계가 일치하는 sealed snapshot만 읽는다.
2. Git이 추적하는 일반 파일의 SHA-256, 길이, 파일 모드를 비교한다.
3. **같은 경로의 내용과 모드가 동일한 파일만** 저장된 mtime로 되돌린다.
   변경·추가·이동된 파일은 새 checkout mtime를 유지한다.
4. symlink/junction 경로와 hardlink는 제외한다. 열린 파일의 신원을 확인한 descriptor로
   시간을 바꾸며 내용, 권한, Cargo 산출물·dep-info·fingerprint·디렉터리 시간은 바꾸지 않는다.
5. 이번 빌드 전 입력을 pending snapshot으로 기록한다. 손상·누락·다른 경계의 snapshot은
   정상적인 Cargo 검증과 재빌드로 돌아간다.

성공한 trusted push에서 target을 저장하기 직전 `seal`을 실행한다.
빌드 전후의 내용·mtime·ctime·파일 신원·모드가 모두 같아야 sealed가 된다.
검증 도중 소스가 바뀌었다가 원래 내용과 mtime로 돌아와도 ctime/신원 변화로 무효화한다.
pending 또는 무효화된 snapshot은 다음 실행에서 재사용하지 않는다.
seal이 성공했을 때만 target 저장을 허용하므로, 보정 도중 오류나 입력 변경이 생겨도
낡은 sealed 정보를 새 산출물과 함께 게시하지 않는다.

PR restore-only, 성공한 main/prerelease push만 저장하는 정책은 유지한다.
이전 캐시에는 snapshot이 없으므로 첫 실행은 기존처럼 Cargo가 재빌드한다.
성공한 새 push가 sealed snapshot을 포함한 target을 저장한 뒤부터 복원 효과를 관찰할 수 있다.
캐시가 없거나 보정이 비활성화돼도 원래 Cargo 명령과 시험은 계속 실행된다.

## 검증

~~~text
node --test scripts/ci-cache-inputs.test.mjs scripts/ci-cache.test.mjs scripts/ci-performance.test.mjs
node scripts/check_ci_cache_inputs.mjs
~~~

실제 Cargo 검증은 registry 의존성이 없는 임시 crate와 별도 target만 사용한다.
기존 개발 target과 캐시를 지우거나 바꾸지 않는다. 임시 fixture만 끝에서 제거한다.

Windows 11 / x86_64 / Rust·Cargo 1.95.0에서 다음 8조건이 통과했다.

| 조건 | Cargo 관측 | 확인한 행동 |
|---|---|---|
| 빈 target | Compiling | 캐시 없이 컴파일·시험 성공 |
| 변경 없는 반복 | Fresh | 원래 검증은 그대로 실행 |
| 같은 바이트, 새 checkout mtime, 보정 없음 | Dirty / Compiling | mtime만으로 재빌드 재현 |
| 같은 바이트, 검증된 mtime 복원 | Fresh | 재컴파일 없이 동일 시험 실행 |
| target을 PAX tar로 저장·복원하고 동일 소스를 새로 checkout | Fresh | 복원된 산출물과 동일 입력의 재사용 |
| Rust 소스 내용 변경 | Dirty / Compiling | 바뀐 반환값을 실행해서 확인 |
| `include_str!` 파일 내용 변경 | Dirty / Compiling | 바뀐 포함 데이터를 실행해서 확인 |
| build script 환경 입력 변경 | Dirty / Compiling | 바뀐 생성 상수를 실행해서 확인 |

[구조화된 재현 결과](ci-cache-inputs-2026-11-27.json)를 함께 보관한다.
작은 fixture의 시간은 Akra의 성능 표본으로 계산하지 않는다.

안전 조건의 Node 시험은 내용이 같은 파일, 동일 길이의 내용 변경, 새 파일/이름 변경,
컴파일 조건/checkout 경로 변경, 손상/미봉인 snapshot, symlink/junction/hardlink,
검증 중 내용 변경 후 원복을 다룬다.

## 실제 CI 인수에서 확인할 것

CI의 `CARGO_TERM_VERBOSE=true`와 `steps.jsonl`의 `cargo.dirtyReasons`, `dirtyUnits`,
`checkingUnits`, `freshUnits`로 다음 실행의 원인을 남긴다.
`cache-inputs-prepare.json`과 `cache-inputs-seal.json`에는 보정된 파일 수, 제외 사유,
입력 변경 여부가 남는다. 대상·feature·profile이 다른 자료를 합치지 않는다.

- 동일 Rust 입력의 두 trusted push 사이에 sealed snapshot이 실제 복원됐는가?
- 본 crate의 `Compiling`/`Checking`이 0인가? 남았다면 Cargo가 기록한 다른 사유는 무엇인가?
- 본 crate와 workspace member, build script, 환경 입력의 재빌드를 각각 구분했는가?
- 입력 해시 확인 비용을 포함해 컴파일 구간이 줄었는가?
- 전체 시험은 여전히 실행됐으며 검증 범위와 결과가 같은가?

이번 로컬 fixture로 원격 runner의 checkout 경로, SDK/환경 변경, cache archive 복원,
전체 Akra CI의 전후 시간을 확인한 것으로 취급하지 않는다.
이 인수 자료가 없으면 #2866의 전체 완료와 CI 개선율은 보류한다.
