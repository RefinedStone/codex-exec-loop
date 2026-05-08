use chrono::{DateTime, Utc};

use super::TaskActor;

const TASK_ID_HASH_CHARS: usize = 12;

#[derive(Debug, Default, Clone)]
// task id policy는 create/intake 경로가 공유하는 deterministic id shape와 collision suffix 규칙이다.
pub struct PlanningTaskIdPolicy;

impl PlanningTaskIdPolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn build_task_id(
        &self,
        actor: TaskActor,
        generated_at: DateTime<Utc>,
        stable_text: &str,
        collision_suffix: Option<u32>,
    ) -> String {
        let timestamp = generated_at.format("%Y%m%dT%H%M%SZ");
        let base = format!(
            "task-{}-{timestamp}-{}",
            actor_slug(actor),
            stable_short_hash(stable_text)
        );
        match collision_suffix {
            Some(suffix) => format!("{base}-{suffix}"),
            None => base,
        }
    }

    pub fn next_collision_suffix(&self, suffix: Option<u32>) -> Option<u32> {
        Some(suffix.unwrap_or(0) + 1)
    }
}

fn actor_slug(actor: TaskActor) -> &'static str {
    match actor {
        TaskActor::User => "user",
        TaskActor::Worker => "worker",
        TaskActor::System => "system",
    }
}

fn stable_short_hash(value: &str) -> String {
    // ID suffix 가독성을 위한 deterministic FNV 축약값이다. 보안용 digest가 아니며, 충돌은
    // application의 authority snapshot retry가 numeric suffix로 해결한다.
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..TASK_ID_HASH_CHARS].to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{PlanningTaskIdPolicy, TASK_ID_HASH_CHARS};
    use crate::domain::planning::TaskActor;

    #[test]
    fn builds_stable_actor_timestamp_and_content_id() {
        let generated_at = Utc.with_ymd_and_hms(2026, 4, 24, 1, 2, 3).unwrap();
        let policy = PlanningTaskIdPolicy::new();

        let first = policy.build_task_id(
            TaskActor::Worker,
            generated_at,
            "Write review response",
            None,
        );
        let second = policy.build_task_id(
            TaskActor::Worker,
            generated_at,
            "Write review response",
            None,
        );

        assert_eq!(first, second);
        assert!(first.starts_with("task-worker-20260424T010203Z-"));
        assert_eq!(
            first.rsplit('-').next().expect("hash suffix").len(),
            TASK_ID_HASH_CHARS
        );
    }

    #[test]
    fn appends_collision_suffix_only_after_collision() {
        let generated_at = Utc.with_ymd_and_hms(2026, 4, 24, 1, 2, 3).unwrap();
        let policy = PlanningTaskIdPolicy::new();

        let base = policy.build_task_id(TaskActor::User, generated_at, "Ship task", None);
        let collision = policy.build_task_id(TaskActor::User, generated_at, "Ship task", Some(2));

        assert!(!base.ends_with("-2"));
        assert_eq!(collision, format!("{base}-2"));
    }

    #[test]
    fn advances_collision_suffix_from_empty_to_one() {
        let policy = PlanningTaskIdPolicy::new();

        assert_eq!(policy.next_collision_suffix(None), Some(1));
        assert_eq!(policy.next_collision_suffix(Some(1)), Some(2));
    }
}
