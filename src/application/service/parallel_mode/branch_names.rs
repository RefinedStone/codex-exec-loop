// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::collections::BTreeSet;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::readiness::{command_succeeds, run_command};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    AGENT_BRANCH_TRUNCATION_HASH_LEN, AKRA_AGENT_BRANCH_PREFIX, DEFAULT_PUSH_REMOTE_NAME,
    MAX_AGENT_BRANCH_SLUG_LEN,
};

/*
학습 주석: agent branch 이름은 slot lease의 git identity입니다. 같은 task가 같은 slot에서
재시도되거나, 이전 원격 branch가 남아 있어도 충돌하지 않아야 하므로 local branch와 remote
tracking/live remote branch를 모두 확인합니다. slug 후보는 task_slug, task_id, task_title
순서로 고르며, 모두 비면 `task` fallback을 씁니다.

반환 형식은 `akra-agent/<slot_id>/<slug>`입니다. slot id를 branch path에 넣으면 어떤 pool
slot이 만든 branch인지 GitHub와 local git log에서 바로 추적할 수 있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn allocate_agent_branch_name(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    slot_id: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    task_slug: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    task_id: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    task_title: &str,
) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let sanitized_slug = sanitize_task_slug(task_slug)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| sanitize_task_slug(task_id))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| sanitize_task_slug(task_title))
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .unwrap_or_else(|| "task".to_string());
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let remote_branch_names = remote_agent_branch_names(repo_root, slot_id);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut collision_index = 1usize;
    // 학습 주석: `loop`는 명시적으로 `break`될 때까지 계속 실행되는 반복문입니다.
    loop {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let candidate = build_agent_branch_name(slot_id, &sanitized_slug, collision_index);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if agent_branch_name_is_available(repo_root, &candidate, &remote_branch_names) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return candidate;
        }
        collision_index += 1;
    }
}

/*
학습 주석: branch name availability는 local ref와 remote ref를 함께 봅니다. local만 보면
이미 origin에 남아 있는 branch와 같은 이름을 새로 만들 수 있고, push 단계에서 충돌합니다.
remote 정보를 미리 반영하면 lease 획득 시점에 안정적인 branch 이름을 선택할 수 있습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn agent_branch_name_is_available(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    branch_name: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    remote_branch_names: &BTreeSet<String>,
) -> bool {
    !branch_exists(repo_root, branch_name) && !remote_branch_names.contains(branch_name)
}

/*
학습 주석: branch slug가 너무 길면 GitHub UI와 git ref 조작이 불편해집니다. 이 함수는 전체
slug 길이를 제한하되, 충돌 번호 suffix가 들어갈 공간을 먼저 빼고 남은 길이만 slug에 할당합니다.
그래야 `-2`, `-3` 같은 collision suffix가 붙어도 최대 길이를 넘지 않습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn build_agent_branch_name(slot_id: &str, sanitized_slug: &str, collision_index: usize) -> String {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let collision_suffix = if collision_index > 1 {
        format!("-{collision_index}")
    } else {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        String::new()
    };
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let bounded_slug = bounded_agent_branch_slug(
        sanitized_slug,
        MAX_AGENT_BRANCH_SLUG_LEN.saturating_sub(collision_suffix.len()),
    );
    format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/{bounded_slug}{collision_suffix}")
}

/*
학습 주석: 긴 slug를 자를 때는 앞부분만 남기면 서로 다른 긴 task title이 같은 branch 이름으로
충돌할 수 있습니다. 그래서 prefix 뒤에 stable hash를 붙입니다. hash가 들어가면 사람이 읽을 수
있는 앞부분과 충돌 방지 식별자를 동시에 확보합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn bounded_agent_branch_slug(slug: &str, max_len: usize) -> String {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if slug.len() <= max_len {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return slug.to_string();
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let hash = short_branch_slug_hash(slug);
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if max_len <= hash.len() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return hash[..max_len].to_string();
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let prefix_len = max_len.saturating_sub(hash.len() + 1);
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let prefix = truncate_to_char_boundary(slug, prefix_len).trim_end_matches('-');
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if prefix.is_empty() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return hash[..max_len.min(hash.len())].to_string();
    }

    format!("{prefix}-{hash}")
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn short_branch_slug_hash(input: &str) -> String {
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    // 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
    const FNV_PRIME: u64 = 0x100000001b3;

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut hash = FNV_OFFSET_BASIS;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..AGENT_BRANCH_TRUNCATION_HASH_LEN].to_string()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn truncate_to_char_boundary(value: &str, max_len: usize) -> &str {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if value.len() <= max_len {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return value;
    }

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut boundary = 0usize;
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for (index, character) in value.char_indices() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let next_boundary = index + character.len_utf8();
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if next_boundary > max_len {
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            break;
        }
        boundary = next_boundary;
    }

    &value[..boundary]
}

/*
학습 주석: task slug는 git ref path에 들어가므로 ASCII alphanumeric과 dash만 남깁니다. 공백,
구두점, 한글 같은 문자는 dash separator로 축약하고, 연속 dash와 끝 dash를 제거합니다.
slug가 완전히 비면 caller가 task_id나 title fallback을 시도합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn sanitize_task_slug(input: &str) -> Option<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut slug = String::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut previous_was_dash = false;

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for ch in input.chars() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let normalized = ch.to_ascii_lowercase();
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            previous_was_dash = false;
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !previous_was_dash && !slug.is_empty() {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn branch_exists(repo_root: &str, branch_name: &str) -> bool {
    command_succeeds(
        "git",
        [
            "-C",
            repo_root,
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ],
    )
}

/*
학습 주석: remote branch lookup은 두 경로를 합칩니다. remote tracking refs는 이미 fetch된
origin 상태이고, live ls-remote는 아직 fetch되지 않은 원격 branch까지 확인합니다. 둘을 합쳐야
오래된 local tracking 정보와 최신 remote reality 사이의 틈에서 branch 이름 충돌이 생기지 않습니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn remote_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut branch_names = remote_tracking_agent_branch_names(repo_root, slot_id);
    branch_names.extend(remote_live_agent_branch_names(repo_root, slot_id));
    branch_names
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn remote_tracking_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let refs_prefix =
        format!("refs/remotes/{DEFAULT_PUSH_REMOTE_NAME}/{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    run_command(
        "git",
        [
            "-C",
            repo_root,
            "for-each-ref",
            "--format=%(refname)",
            refs_prefix.as_str(),
        ],
        None,
    )
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .map(|output| {
        output
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .lines()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .filter_map(|line| line.strip_prefix(&refs_prefix))
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(|suffix| format!("{branch_prefix}{suffix}"))
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .collect()
    })
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .unwrap_or_default()
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn remote_live_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let refs_prefix = "refs/heads/";
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let pattern = format!("refs/heads/{branch_prefix}*");
    run_command(
        "git",
        [
            "-C",
            repo_root,
            "ls-remote",
            "--heads",
            DEFAULT_PUSH_REMOTE_NAME,
            pattern.as_str(),
        ],
        None,
    )
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .map(|output| {
        output
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .lines()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .filter_map(|line| line.split_whitespace().nth(1))
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .filter_map(|remote_ref| remote_ref.strip_prefix(refs_prefix))
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(str::to_string)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .collect()
    })
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .unwrap_or_default()
}
