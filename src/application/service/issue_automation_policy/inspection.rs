use crate::application::port::outbound::issue_automation_policy_source_port::{
    IssueAutomationPolicySourceDocument, IssueAutomationPolicySourceRead,
    IssueAutomationPolicySourceSnapshot,
};

use super::{
    IssueAutomationPolicyBlocked, IssueAutomationPolicyFile, IssueAutomationPolicyFileScope,
    IssueAutomationPolicyFileStatus, IssueAutomationPolicyInspection,
    IssueAutomationPolicyInspectionBlocked, IssueAutomationPolicyOrigin,
    IssueAutomationPolicyOriginScope, IssueAutomationPolicyResolution, IssueAutomationPolicyScope,
    IssueAutomationPolicySetting, resolve_with_provenance,
};

mod settings;

pub(super) fn inspect(
    workspace_dir: &str,
    snapshot: IssueAutomationPolicySourceSnapshot,
) -> IssueAutomationPolicyInspection {
    let files = vec![
        file_view(IssueAutomationPolicyFileScope::User, &snapshot.user),
        file_view(IssueAutomationPolicyFileScope::Project, &snapshot.project),
    ];
    let resolved = (|| {
        let user = contents(&snapshot.user, IssueAutomationPolicyScope::User)?;
        let project = contents(&snapshot.project, IssueAutomationPolicyScope::Project)?;
        let (resolved, origins) = resolve_with_provenance(user, project)?;
        let settings = settings::from_policy(&resolved.policy, &origins)?;
        Ok::<_, IssueAutomationPolicyBlocked>((resolved.version, settings))
    })();
    match resolved {
        Ok((version, settings)) => IssueAutomationPolicyInspection::Ready {
            workspace: workspace_dir.to_owned(),
            version,
            settings,
            files,
        },
        Err(error) => IssueAutomationPolicyInspection::PolicyBlocked {
            workspace: workspace_dir.to_owned(),
            blocked: blocked_view(error),
            files,
        },
    }
}

fn contents(
    document: &IssueAutomationPolicySourceDocument,
    scope: IssueAutomationPolicyScope,
) -> Result<Option<&str>, IssueAutomationPolicyBlocked> {
    match &document.read {
        IssueAutomationPolicySourceRead::Missing => Ok(None),
        IssueAutomationPolicySourceRead::Contents(contents) => Ok(Some(contents)),
        IssueAutomationPolicySourceRead::Unavailable => {
            Err(IssueAutomationPolicyBlocked::unreadable(scope))
        }
    }
}

fn file_view(
    scope: IssueAutomationPolicyFileScope,
    document: &IssueAutomationPolicySourceDocument,
) -> IssueAutomationPolicyFile {
    let status = match &document.read {
        IssueAutomationPolicySourceRead::Missing => IssueAutomationPolicyFileStatus::Missing,
        IssueAutomationPolicySourceRead::Contents(_) => IssueAutomationPolicyFileStatus::Readable,
        IssueAutomationPolicySourceRead::Unavailable => {
            IssueAutomationPolicyFileStatus::Unavailable
        }
    };
    // 상대 경로나 손실 변환된 경로를 파일 열기의 근거로 쓰지 않는다.
    let path = document
        .location
        .as_ref()
        .filter(|path| path.is_absolute())
        .and_then(|path| path.to_str())
        .map(str::to_owned);
    let open_target =
        (path.is_some() && status != IssueAutomationPolicyFileStatus::Unavailable).then_some(scope);
    IssueAutomationPolicyFile {
        scope,
        label: match scope {
            IssueAutomationPolicyFileScope::User => "사용자 자동화 정책",
            IssueAutomationPolicyFileScope::Project => "저장소 자동화 정책",
        }
        .to_owned(),
        path,
        status,
        open_target,
    }
}

fn blocked_view(error: IssueAutomationPolicyBlocked) -> IssueAutomationPolicyInspectionBlocked {
    let summary = match error.reason {
        "source_unavailable" => "자동화 정책 파일을 안전하게 읽지 못했습니다.",
        "invalid_toml_or_shape" => "자동화 정책의 TOML 문법이나 설정 구조가 올바르지 않습니다.",
        "unsupported_version" => "지원하지 않는 자동화 정책 버전입니다.",
        "must_precede_start" => "시작 전 알림 시간은 시작 제한 시간보다 짧아야 합니다.",
        "integration_requires_transition" => {
            "통합 뒤 닫기를 사용하려면 상태 전이 쓰기도 허용해야 합니다."
        }
        "action_not_allowed_for_gate_or_mode" => {
            "해당 자율 수준이나 관문에서 사용할 수 없는 동작입니다."
        }
        "canonical_encoding_failed" => "자동화 정책의 효과값을 읽기 전용 표시로 만들지 못했습니다.",
        _ => "자동화 정책 값이 지원하는 형식이나 제한을 만족하지 않습니다.",
    };
    let (next_summary, file_scopes) = match error.scope {
        IssueAutomationPolicyScope::User => (
            if error.reason == "source_unavailable" {
                "사용자 자동화 정책의 위치와 파일 접근 조건을 확인한 뒤 다시 불러오세요."
            } else {
                "사용자 자동화 정책 파일의 해당 설정을 수정한 뒤 다시 불러오세요."
            },
            vec![IssueAutomationPolicyFileScope::User],
        ),
        IssueAutomationPolicyScope::Project => (
            if error.reason == "source_unavailable" {
                "저장소 자동화 정책의 위치와 파일 접근 조건을 확인한 뒤 다시 불러오세요."
            } else {
                "저장소 자동화 정책 파일의 해당 설정을 수정한 뒤 다시 불러오세요."
            },
            vec![IssueAutomationPolicyFileScope::Project],
        ),
        IssueAutomationPolicyScope::Effective => (
            "사용자·저장소 자동화 정책의 해당 설정과 상속된 값을 함께 확인한 뒤 다시 불러오세요.",
            vec![
                IssueAutomationPolicyFileScope::User,
                IssueAutomationPolicyFileScope::Project,
            ],
        ),
    };
    IssueAutomationPolicyInspectionBlocked {
        scope: error.scope,
        field: error.field,
        reason: error.reason,
        summary: summary.to_owned(),
        next_summary: next_summary.to_owned(),
        file_scopes,
    }
}

#[cfg(test)]
mod tests;
