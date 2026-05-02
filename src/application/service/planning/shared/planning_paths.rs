// `Component`는 path를 root, parent(`..`), normal segment 같은 의미 단위로 나눠 보게 해 준다.
// 문자열 검사와 함께 쓰면 Windows/Unix 표기 차이를 지나도 workspace 탈출 시도를 더 안정적으로 걸러낸다.
use std::path::{Component, Path};

// planning service는 direction detail 문서나 queue-idle prompt 경로를 markdown/DB record 안에
// 저장한다. 이 validator는 그런 저장 경로가 지정된 prefix 아래의 상대 `.md` 파일인지 확인해,
// adapter/runtime이 임의 경로를 열지 않게 하는 application-layer path guard이다.
pub(crate) fn is_valid_planning_markdown_path(path: &str, required_prefix: &str) -> bool {
    // 입력은 사용자가 쓰거나 기존 markdown에서 읽은 문자열이므로 양끝 공백과 Windows separator를
    // 먼저 정규화한다. 아래 검사는 `/` 기준으로 통일된 상대 경로를 대상으로 한다.
    let normalized = path.trim().replace('\\', "/");
    // 빈 경로, 절대 경로, parent traversal은 모두 planning workspace 밖 파일을 가리킬 수 있어 즉시
    // 거부한다. 문자열 검사와 `Path::components` 검사를 함께 둬 `..`, `a/../b`, separator 변형을
    // 이중으로 막는다.
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized.contains("/..")
        || Path::new(&normalized)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        // 여기서 `false`를 반환하면 caller는 해당 markdown reference를 신뢰하지 않고 오류나 진단으로
        // 처리한다. file IO까지 내려가기 전에 방어하는 application-layer guard이다.
        return false;
    }

    // required_prefix는 caller가 허용한 planning 하위 디렉터리이다. prefix가 맞지 않으면 같은
    // workspace 안이라도 이 validation context에서는 잘못된 reference이다.
    let Some(suffix) = normalized.strip_prefix(required_prefix) else {
        // prefix mismatch는 잘못된 종류의 planning file을 참조했다는 뜻이라 즉시 거부한다.
        return false;
    };

    // prefix만 같은 문자열도 통과하지 않도록 suffix가 `/`로 시작하고 실제 파일명이 뒤따라야 한다.
    // 마지막 `.md` 조건은 planning references가 markdown 문서만 가리키는 service 계약이다.
    suffix.starts_with('/') && suffix.len() > 1 && normalized.ends_with(".md")
}
