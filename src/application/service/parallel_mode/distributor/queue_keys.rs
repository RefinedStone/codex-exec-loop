// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::time::{SystemTime, UNIX_EPOCH};

/*
학습 주석: distributor claim owner token은 queue head lock의 소유자를 구분하는 값입니다. process id,
queue item id, high-resolution timestamp를 함께 넣어 같은 프로세스가 같은 queue item을 재시도해도
토큰이 달라지게 합니다. release 시 token 일치 여부를 확인하므로 다른 process의 claim을 실수로
해제하지 않습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn distributor_claim_owner_token(queue_item_id: &str) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let unique_suffix = SystemTime::now()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .duration_since(UNIX_EPOCH)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or_default()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .as_nanos();
    format!(
        "distributor-queue-head-{}-{}-{unique_suffix}",
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        std::process::id(),
        sanitize_runtime_record_key(queue_item_id)
    )
}

/*
학습 주석: runtime record key는 파일명, lock token, store key에 들어가므로 path separator나
공백 같은 문자를 그대로 두면 안 됩니다. ASCII alphanumeric, dash, underscore만 보존하고 나머지는
underscore로 바꿔 storage boundary에서 안전한 identifier로 만듭니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn sanitize_runtime_record_key(value: &str) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut key = String::new();
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for ch in value.chars() {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            key.push(ch);
        } else {
            key.push('_');
        }
    }
    key
}
