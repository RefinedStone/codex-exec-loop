// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::fs;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::path::{Path, PathBuf};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::thread;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::time::{Duration, Instant, SystemTime};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::super::ensure_directory_exists;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{derive_default_pool_root, detect_canonical_repo_root};

// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const POOL_ALLOCATION_LOCK_DIR: &str = ".allocation-lock";
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const POOL_ALLOCATION_LOCK_OWNER_FILE: &str = "owner";
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const POOL_ALLOCATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const POOL_ALLOCATION_LOCK_RETRY: Duration = Duration::from_millis(25);
// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const POOL_ALLOCATION_LOCK_STALE_AFTER: Duration = Duration::from_secs(300);

/*
학습 주석: allocation lock은 여러 turn submission이 동시에 빈 slot을 잡으려 할 때 같은
slot을 중복 배정하지 않게 하는 파일시스템 락입니다. lock directory 생성은 대부분의
파일시스템에서 원자적이므로, 성공한 프로세스 하나만 lease 선택과 branch 생성 구간에
들어갈 수 있습니다.

`Drop`에서 release를 호출하므로, acquire 후 중간에 에러가 나도 스코프를 벗어나며 락이
해제됩니다. owner token을 함께 저장하는 이유는 stale lock 제거와 잘못된 owner의 release를
구분하기 위해서입니다.
*/
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(in crate::application::service::parallel_mode) struct PoolAllocationLock {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    lock_path: PathBuf,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    owner_token: String,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl Drop for PoolAllocationLock {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn drop(&mut self) {
        release_pool_allocation_lock(&self.lock_path, &self.owner_token);
    }
}

/*
학습 주석: public acquire 함수는 먼저 canonical repo root와 default pool root를 찾고,
pool root 디렉터리를 보장한 뒤 실제 lock acquire로 들어갑니다. 호출자가 workspace 하위
디렉터리에서 시작해도 같은 canonical root를 기준으로 같은 pool lock을 사용해야 병렬
slot 배정이 하나의 임계구역으로 묶입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(in crate::application::service::parallel_mode) fn acquire_pool_allocation_lock(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    planning_authority: &dyn PlanningAuthorityPort,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    workspace_dir: &str,
) -> Result<PoolAllocationLock, String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .ok_or_else(|| "canonical root inspection failed".to_string())?;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    ensure_directory_exists(&pool_root)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map_err(|error| format!("pool root creation failed before allocation lock: {error}"))?;
    acquire_pool_allocation_lock_at(&pool_root)
}

/*
학습 주석: 실제 lock 획득 루프는 `.allocation-lock` 디렉터리 생성을 시도합니다. 이미 있으면
stale lock인지 확인한 뒤 짧게 sleep하고 재시도합니다. timeout을 둔 이유는 다른 프로세스가
정상적으로 slot을 배정 중일 때 무한정 TUI turn submission이 멈추지 않게 하기 위해서입니다.

owner 파일 쓰기에 실패하면 방금 만든 lock directory를 지우고 실패합니다. owner token 없는
lock은 누가 소유하는지 검증할 수 없어서 release 안전성이 떨어지기 때문입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn acquire_pool_allocation_lock_at(pool_root: &Path) -> Result<PoolAllocationLock, String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let lock_path = pool_root.join(POOL_ALLOCATION_LOCK_DIR);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let deadline = Instant::now() + POOL_ALLOCATION_LOCK_TIMEOUT;
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let owner_token = pool_allocation_lock_owner_token();

    // 학습 주석: `loop`는 명시적으로 `break`될 때까지 계속 실행되는 반복문입니다.
    loop {
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match fs::create_dir(&lock_path) {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(()) => {
                // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                if let Err(error) = fs::write(
                    lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE),
                    &owner_token,
                ) {
                    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                    let _ = fs::remove_dir_all(&lock_path);
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return Err(format!(
                        "pool allocation lock owner could not be written at `{}`: {error}",
                        lock_path.display()
                    ));
                }
                // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                return Ok(PoolAllocationLock {
                    lock_path,
                    owner_token,
                });
            }
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                remove_stale_pool_allocation_lock(&lock_path);
                // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                if Instant::now() >= deadline {
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return Err(format!(
                        "pool allocation lock is busy at `{}`",
                        lock_path.display()
                    ));
                }
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                thread::sleep(POOL_ALLOCATION_LOCK_RETRY);
            }
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) => {
                // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                return Err(format!(
                    "pool allocation lock could not be acquired at `{}`: {error}",
                    lock_path.display()
                ));
            }
        }
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn pool_allocation_lock_owner_token() -> String {
    /*
    학습 주석: owner token은 lock directory를 만든 실행 주체를 최소 정보로 식별합니다. pid는
    stale lock을 정리할 때 아직 살아 있는 프로세스인지 확인하는 단서이고, created_at_ms는
    사람이 pool root를 열어 봤을 때 언제 생긴 lock인지 판단하는 운영 단서입니다. token 전체를
    release 시 비교하므로, 같은 pid가 재사용되더라도 이전 permit이 새 lock을 삭제할 위험을
    줄입니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let created_at = SystemTime::now()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .duration_since(SystemTime::UNIX_EPOCH)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|duration| duration.as_millis())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or_default();
    format!("pid={}\ncreated_at_ms={created_at}\n", std::process::id())
}

/*
학습 주석: release는 owner 파일의 내용이 현재 permit의 owner token과 같을 때만 lock directory를
지웁니다. acquire timeout 중 stale lock 제거가 일어났거나 다른 프로세스가 새 lock을 잡은
상태에서 이전 permit이 drop될 수 있으므로, token 확인 없이 삭제하면 남의 lock을 풀 수
있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn release_pool_allocation_lock(lock_path: &Path, owner_token: &str) {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let owner_path = lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Ok(current_owner) = fs::read_to_string(&owner_path) else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return;
    };
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if current_owner == owner_token {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = fs::remove_dir_all(lock_path);
    }
}

/*
학습 주석: stale lock 제거는 오래된 lock directory가 있고, owner pid가 없거나 죽은 것으로
확인될 때만 실행됩니다. 수정 시간이 짧으면 정상 작업 중일 수 있어 건드리지 않고, pid 상태가
Unknown이면 보수적으로 유지합니다. slot 중복 배정보다 잠시 busy로 남는 편이 안전하기
때문입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn remove_stale_pool_allocation_lock(lock_path: &Path) {
    /*
    학습 주석: stale 제거는 allocation lock에서 가장 보수적이어야 하는 경로입니다. 여기서 실수로
    살아 있는 lock을 지우면 두 agent가 같은 idle slot을 동시에 lease할 수 있습니다. 그래서
    directory 수정 시간이 충분히 오래됐는지 먼저 확인하고, 그 다음 owner pid가 없거나 명확히
    죽었다고 확인되는 경우에만 directory를 지웁니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Ok(metadata) = fs::metadata(lock_path) else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return;
    };
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Ok(modified_at) = metadata.modified() else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return;
    };
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Ok(age) = SystemTime::now().duration_since(modified_at) else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return;
    };
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if age >= POOL_ALLOCATION_LOCK_STALE_AFTER {
        /*
        학습 주석: owner 파일을 읽을 수 없으면 `None`으로 이어지고 stale 제거 대상이 됩니다. 오래된
        lock에 owner가 없다는 것은 acquire 도중 owner write 전에 죽었거나 파일이 손상된 상태라,
        새 lease 배정을 영원히 막기보다 lock을 회수하는 쪽이 낫습니다. 하지만 owner pid가 있고
        liveness가 Alive 또는 Unknown이면 lock을 보존합니다.
        */
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let owner_path = lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !matches!(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            fs::read_to_string(owner_path)
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .ok()
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .and_then(|owner| pool_allocation_lock_owner_pid(&owner))
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .map(pool_allocation_lock_owner_liveness),
            None | Some(PoolAllocationLockOwnerLiveness::Dead)
        ) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = fs::remove_dir_all(lock_path);
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn pool_allocation_lock_owner_pid(owner_token: &str) -> Option<u32> {
    /*
    학습 주석: owner token은 사람이 읽기 쉬운 key=value 줄 목록입니다. pid parsing은 그중
    `pid=` 줄만 골라 process liveness check로 넘기는 좁은 helper입니다. 형식이 깨졌거나 숫자로
    파싱되지 않으면 None으로 두어 stale cleanup이 "소유자를 확인할 수 없는 오래된 lock"으로
    처리하게 합니다.
    */
    owner_token
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .lines()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .find_map(|line| line.strip_prefix("pid=")?.parse::<u32>().ok())
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: `enum`은 가능한 상태나 명령을 정해진 선택지로 제한해 패턴 매칭으로 안전하게 처리하게 해줍니다.
enum PoolAllocationLockOwnerLiveness {
    /*
    학습 주석: Alive는 lock을 유지해야 한다는 강한 신호이고, Dead는 오래된 lock을 회수해도 되는
    신호입니다. Unknown은 보수적 안전 상태로, process lookup 실패나 권한 문제처럼 "죽었다고
    증명하지 못한" 경우입니다. remove_stale 경로는 Unknown을 Dead처럼 취급하지 않습니다.
    */
    Alive,
    Dead,
    Unknown,
}

/*
학습 주석: owner liveness는 플랫폼별 process table을 아주 얕게 확인합니다. Unix에서는
`kill -0`, Windows에서는 `tasklist`를 사용하고, 둘 다 실패하면 Unknown으로 둡니다.
Unknown을 Dead로 취급하지 않는 이유는 권한 문제나 플랫폼 차이로 살아 있는 프로세스를
잘못 죽은 것으로 판단해 lock을 훔치는 일을 피하기 위해서입니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn pool_allocation_lock_owner_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    platform_process_liveness(pid)
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(unix)]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn platform_process_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    /*
    학습 주석: Unix의 `kill -0`은 실제 signal을 보내지 않고 process 존재/접근 가능 여부만
    검사합니다. 성공은 pid가 살아 있거나 접근 가능하다는 의미로 Alive이고, non-zero status는
    process가 없거나 접근할 수 없다는 뜻입니다. 이 구현은 allocation lock recovery의 보조
    판단일 뿐이라, command 실행 자체가 실패하면 Unknown으로 둡니다.
    */
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match std::process::Command::new("kill")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .args(["-0", &pid.to_string()])
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .status()
    {
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(status) if status.success() => PoolAllocationLockOwnerLiveness::Alive,
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(_) => PoolAllocationLockOwnerLiveness::Dead,
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Err(_) => PoolAllocationLockOwnerLiveness::Unknown,
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(windows)]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn platform_process_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    /*
    학습 주석: Windows에는 `kill -0`과 같은 portable primitive가 없으므로 `tasklist` 필터로
    pid가 현재 process table에 있는지 확인합니다. 출력 형식은 locale이나 Windows 버전에 따라
    달라질 수 있어, 명령 실패는 Unknown으로 보수 처리하고, 성공 출력에 pid field가 있을 때만
    Alive로 판단합니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let filter = format!("PID eq {pid}");
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match std::process::Command::new("tasklist")
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .args(["/FI", filter.as_str(), "/NH"])
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .output()
    {
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(output) if output.status.success() => {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let stdout = String::from_utf8_lossy(&output.stdout);
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if stdout
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .split_whitespace()
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .any(|field| field.trim() == pid.to_string())
            {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                PoolAllocationLockOwnerLiveness::Alive
            } else {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                PoolAllocationLockOwnerLiveness::Dead
            }
        }
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(_) => PoolAllocationLockOwnerLiveness::Dead,
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Err(_) => PoolAllocationLockOwnerLiveness::Unknown,
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(not(any(unix, windows)))]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn platform_process_liveness(_pid: u32) -> PoolAllocationLockOwnerLiveness {
    /*
    학습 주석: 지원하지 않는 platform에서는 process liveness를 안전하게 증명할 방법이 없으므로
    Unknown을 반환합니다. 이 값은 stale cleanup에서 lock 보존으로 이어져, 자동 회수보다 중복
    slot 배정 방지를 우선합니다.
    */
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    PoolAllocationLockOwnerLiveness::Unknown
}
