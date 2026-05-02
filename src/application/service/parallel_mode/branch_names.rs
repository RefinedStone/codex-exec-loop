use std::collections::BTreeSet;

use super::readiness::{command_succeeds, run_command};
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
pub(super) fn allocate_agent_branch_name(
    repo_root: &str,
    slot_id: &str,
    task_slug: &str,
    task_id: &str,
    task_title: &str,
) -> String {
    // 학습 주석: task_slug는 계획 시스템이 준 짧은 이름이라 가장 읽기 좋고, 없으면 id와
    // title을 차례로 축약합니다. 이 순서가 lease 파일, PR branch, pool board의 표시명을 맞춥니다.
    let sanitized_slug = sanitize_task_slug(task_slug)
        .or_else(|| sanitize_task_slug(task_id))
        .or_else(|| sanitize_task_slug(task_title))
        .unwrap_or_else(|| "task".to_string());
    // 학습 주석: remote branch 목록은 루프 밖에서 한 번만 읽습니다. allocation 중 같은 프로세스가
    // 만든 local branch 충돌은 `branch_exists`가 잡고, 이미 원격에 있던 이름은 이 set이 잡습니다.
    let remote_branch_names = remote_agent_branch_names(repo_root, slot_id);
    let mut collision_index = 1usize;
    loop {
        let candidate = build_agent_branch_name(slot_id, &sanitized_slug, collision_index);
        if agent_branch_name_is_available(repo_root, &candidate, &remote_branch_names) {
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
fn agent_branch_name_is_available(
    repo_root: &str,
    branch_name: &str,
    remote_branch_names: &BTreeSet<String>,
) -> bool {
    !branch_exists(repo_root, branch_name) && !remote_branch_names.contains(branch_name)
}

/*
학습 주석: branch slug가 너무 길면 GitHub UI와 git ref 조작이 불편해집니다. 이 함수는 전체
slug 길이를 제한하되, 충돌 번호 suffix가 들어갈 공간을 먼저 빼고 남은 길이만 slug에 할당합니다.
그래야 `-2`, `-3` 같은 collision suffix가 붙어도 최대 길이를 넘지 않습니다.
*/
fn build_agent_branch_name(slot_id: &str, sanitized_slug: &str, collision_index: usize) -> String {
    // 학습 주석: 첫 번째 후보는 suffix 없이 사람이 읽기 좋은 이름을 유지하고, 충돌이
    // 확인된 뒤에만 번호를 붙여 기존 branch와 구분합니다.
    let collision_suffix = if collision_index > 1 {
        format!("-{collision_index}")
    } else {
        String::new()
    };
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
fn bounded_agent_branch_slug(slug: &str, max_len: usize) -> String {
    if slug.len() <= max_len {
        return slug.to_string();
    }

    let hash = short_branch_slug_hash(slug);
    // 학습 주석: limit이 극단적으로 작아도 caller가 빈 문자열이나 panic 대신 deterministic
    // suffix를 받게 해 branch construction 경로를 단순하게 유지합니다.
    if max_len <= hash.len() {
        return hash[..max_len].to_string();
    }

    // 학습 주석: prefix는 UTF-8 경계에서 자르고 끝 dash를 제거합니다. 그렇지 않으면
    // `<prefix>-<hash>` 조합이 보기 나쁘거나 잘못된 byte slice가 됩니다.
    let prefix_len = max_len.saturating_sub(hash.len() + 1);
    let prefix = truncate_to_char_boundary(slug, prefix_len).trim_end_matches('-');
    if prefix.is_empty() {
        return hash[..max_len.min(hash.len())].to_string();
    }

    format!("{prefix}-{hash}")
}

pub(super) fn short_branch_slug_hash(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    // 학습 주석: branch slug hash는 보안 식별자가 아니라 긴 task title을 잘랐을 때
    // 사람이 읽는 prefix 뒤에 붙일 안정적인 충돌 완화 suffix입니다.
    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..AGENT_BRANCH_TRUNCATION_HASH_LEN].to_string()
}

fn truncate_to_char_boundary(value: &str, max_len: usize) -> &str {
    if value.len() <= max_len {
        return value;
    }

    // 학습 주석: byte length limit을 쓰지만 Rust string slice는 UTF-8 경계에서만 잘라야 합니다.
    // sanitization 뒤에는 대부분 ASCII지만, 이 helper는 재사용 지점이 늘어도 안전하게 동작합니다.
    let mut boundary = 0usize;
    for (index, character) in value.char_indices() {
        let next_boundary = index + character.len_utf8();
        if next_boundary > max_len {
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
pub(super) fn sanitize_task_slug(input: &str) -> Option<String> {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for ch in input.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            previous_was_dash = false;
            continue;
        }
        // 학습 주석: separator는 slug 중간에만 하나씩 남겨 `foo--bar`나 `-foo` 같은
        // git에는 가능하지만 사람이 읽기 불편한 branch segment를 만들지 않습니다.
        if !previous_was_dash && !slug.is_empty() {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

pub(super) fn branch_exists(repo_root: &str, branch_name: &str) -> bool {
    // 학습 주석: local branch existence는 `git branch --list` 대신 exact ref 확인으로 봅니다.
    // prefix가 같은 다른 agent branch가 있어도 현재 후보만 충돌로 처리해야 하기 때문입니다.
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
fn remote_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    let mut branch_names = remote_tracking_agent_branch_names(repo_root, slot_id);
    branch_names.extend(remote_live_agent_branch_names(repo_root, slot_id));
    branch_names
}

fn remote_tracking_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    // 학습 주석: tracking ref는 `refs/remotes/origin/...` 형태라 실제 branch name으로
    // 비교하려면 remote prefix를 제거하고 `akra-agent/<slot>/...` 형태로 되돌려야 합니다.
    let refs_prefix =
        format!("refs/remotes/{DEFAULT_PUSH_REMOTE_NAME}/{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
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
    .map(|output| {
        output
            .lines()
            .filter_map(|line| line.strip_prefix(&refs_prefix))
            .map(|suffix| format!("{branch_prefix}{suffix}"))
            .collect()
    })
    .unwrap_or_default()
}

fn remote_live_agent_branch_names(repo_root: &str, slot_id: &str) -> BTreeSet<String> {
    let refs_prefix = "refs/heads/";
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    // 학습 주석: ls-remote에는 full ref glob을 넘기고, 결과는 full ref로 돌아오므로 아래에서
    // `refs/heads/`만 제거해 local branch name과 같은 좌표계로 맞춥니다.
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
    .map(|output| {
        output
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .filter_map(|remote_ref| remote_ref.strip_prefix(refs_prefix))
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}
