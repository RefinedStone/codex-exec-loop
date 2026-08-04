use std::collections::BTreeSet;
use std::ffi::OsString;

use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;

use super::readiness::command_succeeds_with_runtime;
use super::{
    AGENT_BRANCH_TRUNCATION_HASH_LEN, AKRA_AGENT_BRANCH_PREFIX, MAX_AGENT_BRANCH_SLUG_LEN,
    try_push_remote_name_with_runtime,
};

/*
agent branch 이름은 slot lease의 git identity이다. 같은 task가 같은 slot에서
재시도되거나, 이전 원격 branch가 남아 있어도 충돌하지 않아야 하므로 local branch와 remote
tracking/live remote branch를 모두 확인한다. slug 후보는 task_slug, task_id, task_title
순서로 고르며, 모두 비면 `task` fallback을 쓴다.

반환 형식은 `akra-agent/<slot_id>/<slug>-<lease-instance>`이다. slot id와 lease generation의
64-bit prefix를 함께 넣으면 GitHub/local git log에서 소유 slot을 읽을 수 있고, 서로 다른 clone이
동시에 같은 task slug를 배정해도 같은 source branch를 장시간 공유하지 않는다.
*/
#[allow(clippy::too_many_arguments)]
pub(super) fn allocate_agent_branch_name(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    slot_id: &str,
    task_slug: &str,
    task_id: &str,
    task_title: &str,
    branch_instance_id: &str,
    live_remote_branch_names: &[String],
) -> Result<String, String> {
    if branch_instance_id.len() != 16
        || !branch_instance_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(
            "agent branch instance id must be exactly 16 lowercase hexadecimal characters"
                .to_string(),
        );
    }
    // task_slug는 계획 시스템이 준 짧은 이름이라 가장 읽기 좋고, 없으면 id와
    // title을 차례로 축약한다. 이 순서가 lease 파일, PR branch, pool board의 표시명을 맞춘다.
    let sanitized_slug = sanitize_task_slug(task_slug)
        .or_else(|| sanitize_task_slug(task_id))
        .or_else(|| sanitize_task_slug(task_title))
        .unwrap_or_else(|| "task".to_string());
    // remote branch 목록은 루프 밖에서 한 번만 읽는다. allocation 중 같은 프로세스가
    // 만든 local branch 충돌은 `branch_exists`가 잡고, 이미 원격에 있던 이름은 이 set이 잡는다.
    let remote_branch_names =
        remote_agent_branch_names(runtime, repo_root, slot_id, live_remote_branch_names)?;
    let mut collision_index = 1usize;
    loop {
        let candidate = build_agent_branch_name(
            slot_id,
            &sanitized_slug,
            branch_instance_id,
            collision_index,
        );
        if agent_branch_name_is_available(runtime, repo_root, &candidate, &remote_branch_names) {
            return Ok(candidate);
        }
        collision_index = collision_index
            .checked_add(1)
            .ok_or_else(|| "agent branch collision counter overflowed".to_string())?;
    }
}

/*
branch name availability는 local ref와 remote ref를 함께 본다. local만 보면
이미 origin에 남아 있는 branch와 같은 이름을 새로 만들 수 있고, push 단계에서 충돌한다.
remote 정보를 미리 반영하면 lease 획득 시점에 안정적인 branch 이름을 선택할 수 있다.
*/
fn agent_branch_name_is_available(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    branch_name: &str,
    remote_branch_names: &BTreeSet<String>,
) -> bool {
    !branch_exists(runtime, repo_root, branch_name) && !remote_branch_names.contains(branch_name)
}

/*
branch slug가 너무 길면 GitHub UI와 git ref 조작이 불편해진다. 이 함수는 전체
slug 길이를 제한하되, 충돌 번호 suffix가 들어갈 공간을 먼저 빼고 남은 길이만 slug에 할당한다.
그래야 `-2`, `-3` 같은 collision suffix가 붙어도 최대 길이를 넘지 않는다.
*/
fn build_agent_branch_name(
    slot_id: &str,
    sanitized_slug: &str,
    branch_instance_id: &str,
    collision_index: usize,
) -> String {
    // 첫 번째 후보는 suffix 없이 사람이 읽기 좋은 이름을 유지하고, 충돌이
    // 확인된 뒤에만 번호를 붙여 기존 branch와 구분한다.
    let collision_suffix = if collision_index > 1 {
        format!("-{collision_index}")
    } else {
        String::new()
    };
    let instance_suffix = format!("-{branch_instance_id}");
    let bounded_slug = bounded_agent_branch_slug(
        sanitized_slug,
        MAX_AGENT_BRANCH_SLUG_LEN
            .saturating_sub(instance_suffix.len())
            .saturating_sub(collision_suffix.len()),
    );
    format!(
        "{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/{bounded_slug}{instance_suffix}{collision_suffix}"
    )
}

/*
긴 slug를 자를 때는 앞부분만 남기면 서로 다른 긴 task title이 같은 branch 이름으로
충돌할 수 있다. 그래서 prefix 뒤에 stable hash를 붙인다. hash가 들어가면 사람이 읽을 수
있는 앞부분과 충돌 방지 식별자를 동시에 확보한다.
*/
fn bounded_agent_branch_slug(slug: &str, max_len: usize) -> String {
    if slug.len() <= max_len {
        return slug.to_string();
    }

    let hash = short_branch_slug_hash(slug);
    // limit이 극단적으로 작아도 caller가 빈 문자열이나 panic 대신 deterministic
    // suffix를 받게 해 branch construction 경로를 단순하게 유지한다.
    if max_len <= hash.len() {
        return hash[..max_len].to_string();
    }

    // prefix는 UTF-8 경계에서 자르고 끝 dash를 제거한다. 그렇지 않으면
    // `<prefix>-<hash>` 조합이 보기 나쁘거나 잘못된 byte slice가 된다.
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

    // branch slug hash는 보안 식별자가 아니라 긴 task title을 잘랐을 때
    // 사람이 읽는 prefix 뒤에 붙일 안정적인 충돌 완화 suffix이다.
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

    // byte length limit을 쓰지만 Rust string slice는 UTF-8 경계에서만 잘라야 한다.
    // sanitization 뒤에는 대부분 ASCII지만, 이 helper는 재사용 지점이 늘어도 안전하게 동작한다.
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
task slug는 git ref path에 들어가므로 ASCII alphanumeric과 dash만 남긴다. 공백,
구두점, 한글 같은 문자는 dash separator로 축약하고, 연속 dash와 끝 dash를 제거한다.
slug가 완전히 비면 caller가 task_id나 title fallback을 시도한다.
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
        // separator는 slug 중간에만 하나씩 남겨 `foo--bar`나 `-foo` 같은
        // git에는 가능하지만 사람이 읽기 불편한 branch segment를 만들지 않는다.
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

pub(super) fn branch_exists(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    branch_name: &str,
) -> bool {
    // local branch existence는 `git branch --list` 대신 exact ref 확인으로 본다.
    // prefix가 같은 다른 agent branch가 있어도 현재 후보만 충돌로 처리해야 하기 때문이다.
    command_succeeds_with_runtime(
        runtime,
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
remote branch lookup은 두 경로를 합친다. remote tracking refs는 이미 fetch된
origin 상태이고, live ls-remote는 아직 fetch되지 않은 원격 branch까지 확인한다. 둘을 합쳐야
오래된 local tracking 정보와 최신 remote reality 사이의 틈에서 branch 이름 충돌이 생기지 않는다.
*/
fn remote_agent_branch_names(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    slot_id: &str,
    live_remote_branch_names: &[String],
) -> Result<BTreeSet<String>, String> {
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    let mut branch_names = remote_tracking_agent_branch_names(runtime, repo_root, slot_id)?;
    for branch_name in live_remote_branch_names {
        if !branch_name.starts_with(&branch_prefix) {
            return Err(format!(
                "frozen remote branch listing returned an out-of-scope branch `{branch_name}`"
            ));
        }
        branch_names.insert(branch_name.clone());
    }
    Ok(branch_names)
}

fn remote_tracking_agent_branch_names(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
    slot_id: &str,
) -> Result<BTreeSet<String>, String> {
    // tracking ref는 `refs/remotes/<push-remote>/...` 형태라 실제 branch name으로
    // 비교하려면 remote prefix를 제거하고 `akra-agent/<slot>/...` 형태로 되돌려야 한다.
    let push_remote = try_push_remote_name_with_runtime(runtime, repo_root)?;
    let refs_prefix = format!("refs/remotes/{push_remote}/{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    let branch_prefix = format!("{AKRA_AGENT_BRANCH_PREFIX}/{slot_id}/");
    let args = [
        OsString::from("-C"),
        OsString::from(repo_root),
        OsString::from("for-each-ref"),
        OsString::from("--format=%(refname)"),
        OsString::from(refs_prefix.as_str()),
    ];
    let output = runtime.run_git_command(&args, None).map_err(|error| {
        format!("configured push remote refs could not be inspected safely: {error}")
    })?;
    if !output.succeeded() {
        return Err("configured push remote refs could not be inspected safely".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix(&refs_prefix))
        .map(|suffix| format!("{branch_prefix}{suffix}"))
        .collect())
}
