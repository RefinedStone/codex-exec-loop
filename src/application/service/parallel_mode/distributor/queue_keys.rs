// 학습 주석: distributor claim token은 wall-clock timestamp를 suffix로 넣어 매번 다른 owner 값을 만듭니다.
// `UNIX_EPOCH` 기준 duration을 쓰면 문자열에 넣기 쉬운 단조 증가 성격의 숫자 표현을 얻을 수 있습니다.
use std::time::{SystemTime, UNIX_EPOCH};

/*
학습 주석: distributor claim owner token은 queue head lock의 소유자를 구분하는 값입니다. process id,
queue item id, high-resolution timestamp를 함께 넣어 같은 프로세스가 같은 queue item을 재시도해도
토큰이 달라지게 합니다. release 시 token 일치 여부를 확인하므로 다른 process의 claim을 실수로
해제하지 않습니다.
*/
// 학습 주석: 이 함수는 queue head를 claim할 때 store에 기록할 owner token을 만듭니다. token 안에
// process id와 sanitized queue item id를 넣어 로그/진단에서 어느 실행이 무엇을 잡았는지 추적하게 합니다.
pub(super) fn distributor_claim_owner_token(queue_item_id: &str) -> String {
    // 학습 주석: timestamp suffix는 같은 process가 같은 queue item을 빠르게 재시도해도 이전 claim과 새
    // claim을 구분하게 합니다. system clock 오류 시에도 default 0으로 떨어져 token 생성 자체는 실패하지 않습니다.
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "distributor-queue-head-{}-{}-{unique_suffix}",
        // 학습 주석: process id는 같은 machine에서 병렬로 뜬 distributor 실행을 구분하는 값입니다.
        std::process::id(),
        // 학습 주석: queue item id는 외부 입력에서 올 수 있으므로 token/storage key에 넣기 전에 안전한
        // runtime record key 문자 집합으로 줄입니다.
        sanitize_runtime_record_key(queue_item_id)
    )
}

/*
학습 주석: runtime record key는 파일명, lock token, store key에 들어가므로 path separator나
공백 같은 문자를 그대로 두면 안 됩니다. ASCII alphanumeric, dash, underscore만 보존하고 나머지는
underscore로 바꿔 storage boundary에서 안전한 identifier로 만듭니다.
*/
// 학습 주석: 이 sanitizer는 queue item id를 filesystem/store boundary에서 안전한 key로 바꾸는 작은
// application helper입니다. 원래 id의 구분 가능성은 최대한 유지하되 위험 문자는 `_`로 흡수합니다.
pub(super) fn sanitize_runtime_record_key(value: &str) -> String {
    // 학습 주석: 결과를 누적하는 mutable string입니다. 입력 길이만큼 순회하며 허용 문자만 그대로 보존합니다.
    let mut key = String::new();
    // 학습 주석: char 단위 순회라 Unicode 입력도 panic 없이 처리됩니다. ASCII 허용 집합 밖 문자는 모두
    // `_`가 되어 path separator, whitespace, emoji 같은 문자가 storage key로 새지 않습니다.
    for ch in value.chars() {
        // 학습 주석: dash와 underscore는 사람이 읽기 좋은 id 구분자라 보존합니다. 나머지 안전한 문자는
        // ASCII alphanumeric으로 제한해 shell/path/store에서 해석될 여지를 줄입니다.
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            key.push(ch);
        } else {
            key.push('_');
        }
    }
    key
}
