/*
 * 프로젝트의 이슈 규약과 공개 정책이다 (#2414, #2416). 설정 파일을 읽고 검증한 뒤의 값이다.
 *
 * 필수 게시 검증과 Vault 의 사용자 잠금 해제는 이 값에 없다. 두 규칙은 항상 켜져 있고 설정으로 끌 수 없다.
 */
use serde::{Deserialize, Serialize};

/// 민감 구간의 종류이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSensitiveSpanClass {
    /// 토큰, 비밀번호 같은 비밀값이다.
    Secret,
    /// 사람, 조직, 호스트의 이름이다.
    Name,
    /// 파일 경로이다.
    Path,
    /// 로그의 구간이다.
    Log,
}

impl IssueSensitiveSpanClass {
    pub const ALL: [Self; 4] = [Self::Secret, Self::Name, Self::Path, Self::Log];
}

/// 민감 구간 하나에 하는 처리이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSensitiveSpanAction {
    /// 값을 버린다. 복원할 수 없다.
    Remove,
    /// 표식으로 가린다. 평문은 Vault 에 두고 사용자가 복원한다.
    RestorableMask,
    /// 구간마다 사용자에게 묻는다. 답이 없으면 내보내지 않는다.
    UserConfirm,
}

/// 글이 나가는 대상이다. 대상마다 공개 정책이 따로 있다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueDisclosureTarget {
    /// 공급자(예: GitHub)에 게시한다.
    Publish,
    /// AI 에 전달한다.
    AiHandoff,
}

/// 대상 하나의 공개 정책이다. 민감 구간의 종류마다 처리가 하나 있다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueDisclosurePolicy {
    pub secret: IssueSensitiveSpanAction,
    pub name: IssueSensitiveSpanAction,
    pub path: IssueSensitiveSpanAction,
    pub log: IssueSensitiveSpanAction,
}

impl IssueDisclosurePolicy {
    /// 설정이 없을 때의 값이다. 비밀값은 버리고 나머지는 복원 가능하게 가린다.
    pub fn builtin() -> Self {
        Self {
            secret: IssueSensitiveSpanAction::Remove,
            name: IssueSensitiveSpanAction::RestorableMask,
            path: IssueSensitiveSpanAction::RestorableMask,
            log: IssueSensitiveSpanAction::RestorableMask,
        }
    }

    pub fn action_for(&self, class: IssueSensitiveSpanClass) -> IssueSensitiveSpanAction {
        match class {
            IssueSensitiveSpanClass::Secret => self.secret,
            IssueSensitiveSpanClass::Name => self.name,
            IssueSensitiveSpanClass::Path => self.path,
            IssueSensitiveSpanClass::Log => self.log,
        }
    }
}

/// 프로젝트의 이슈 작성 규약이다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueConvention {
    /// 적용할 이슈 템플릿이다. 저장소 뿌리에서 본 상대 경로이다. 없으면 템플릿을 쓰지 않는다.
    pub template: Option<String>,
    /// 템플릿과 별도로 프로젝트가 요구하는 섹션의 제목이다.
    pub required_sections: Vec<String>,
}

/// 프로젝트 하나에 적용하는 이슈 정책이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuePolicy {
    pub convention: IssueConvention,
    pub publish: IssueDisclosurePolicy,
    pub ai_handoff: IssueDisclosurePolicy,
}

impl IssuePolicy {
    /// 설정 파일이 하나도 없을 때의 정책이다.
    pub fn builtin() -> Self {
        Self {
            convention: IssueConvention::default(),
            publish: IssueDisclosurePolicy::builtin(),
            ai_handoff: IssueDisclosurePolicy::builtin(),
        }
    }

    pub fn disclosure_for(&self, target: IssueDisclosureTarget) -> &IssueDisclosurePolicy {
        match target {
            IssueDisclosureTarget::Publish => &self.publish,
            IssueDisclosureTarget::AiHandoff => &self.ai_handoff,
        }
    }
}
