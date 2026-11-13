/*
 * planning 의 변경이 아무것도 바꾸지 않고 거절된 이유이다 (plan 18 K4-9d,
 * 설계 `docs/plan/18-k4-9d-planning-mutations.md` §3.3).
 *
 * 서비스와 저장소 adapter 는 이 두 경우를 글자 오류가 아니라 이 타입으로 돌려준다. 저장 명령의 실행기가 오류 사슬에서
 * 이 타입을 찾아 커널의 거절(이유 코드)로 바꾼다. 그 밖의 오류는 실패이다. 글(`message`)은 사람이 읽는다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningMutationRefusal {
    /// 보낸 revision 이 지금의 revision 과 다르다. 화면을 다시 읽고 다시 해야 한다.
    RevisionStale { message: String },
    /// 다른 변경이나 병렬 작업이 그 planning 데이터를 쥐고 있다. 조금 뒤에 다시 하면 된다.
    Locked { message: String },
}

impl PlanningMutationRefusal {
    pub fn revision_stale(message: impl Into<String>) -> Self {
        Self::RevisionStale {
            message: message.into(),
        }
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self::Locked {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::RevisionStale { message } | Self::Locked { message } => message,
        }
    }

    /// 오류 사슬에서 거절을 찾는다. 서비스가 `context` 를 덧붙여도 찾는다.
    pub fn find_in(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for PlanningMutationRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for PlanningMutationRefusal {}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn a_refusal_keeps_its_text_and_is_found_under_added_context() {
        let refusal = PlanningMutationRefusal::revision_stale("planning changed; reload and retry");
        let error = Err::<(), _>(anyhow::Error::from(refusal.clone()))
            .context("failed to update the task")
            .unwrap_err();
        assert_eq!(PlanningMutationRefusal::find_in(&error), Some(&refusal));
        // The text for a person is the same as before the type existed.
        assert_eq!(refusal.to_string(), "planning changed; reload and retry");
        assert!(PlanningMutationRefusal::find_in(&anyhow::anyhow!("another failure")).is_none());
    }
}
