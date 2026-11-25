use serde::Serialize;

use crate::domain::issue_automation::policy::IssueAutomationPolicy;

use super::super::provenance::PolicyProvenance;
use super::{
    IssueAutomationPolicyBlocked, IssueAutomationPolicyOrigin, IssueAutomationPolicyOriginScope,
    IssueAutomationPolicyResolution, IssueAutomationPolicyScope, IssueAutomationPolicySetting,
};

pub(super) fn from_policy(
    policy: &IssueAutomationPolicy,
    origins: &PolicyProvenance,
) -> Result<Vec<IssueAutomationPolicySetting>, IssueAutomationPolicyBlocked> {
    let mut settings = Settings {
        rows: Vec::new(),
        origins,
    };
    settings.add("schema_version", "정책 스키마 버전", &policy.schema_version)?;
    settings.add(
        "binding.github.repository",
        "GitHub 저장소",
        &policy.binding.github.repository,
    )?;
    settings.add(
        "binding.github.mode",
        "요청 자율 수준의 효과값",
        &policy.binding.github.mode,
    )?;
    if let Some(jira) = &policy.binding.jira {
        settings.add("binding.jira.site_url", "Jira Cloud 사이트", &jira.site_url)?;
        settings.add(
            "binding.jira.cloud_id",
            "Jira Cloud 안정 ID",
            &jira.cloud_id,
        )?;
        settings.add(
            "binding.jira.project_id",
            "Jira 프로젝트 안정 ID",
            &jira.project_id,
        )?;
        settings.add(
            "binding.jira.expected_account_id",
            "Jira 계정 안정 ID",
            &jira.expected_account_id,
        )?;
        settings.add("binding.jira.jql", "Jira 발견 필터", &jira.jql)?;
        settings.add(
            "binding.jira.mode",
            "Jira 요청 자율 수준의 효과값",
            &jira.mode,
        )?;
        settings.add(
            "binding.jira.close_transition",
            "사용자가 허용한 Jira 완료 전이",
            &jira.close_transition,
        )?;
    }
    if let Some(linear) = &policy.binding.linear {
        settings.add(
            "binding.linear.team_id",
            "Linear 팀 안정 ID",
            &linear.team_id,
        )?;
        settings.add(
            "binding.linear.expected_viewer_id",
            "Linear 계정 안정 ID",
            &linear.expected_viewer_id,
        )?;
        settings.add(
            "binding.linear.mode",
            "Linear 요청 자율 수준의 효과값",
            &linear.mode,
        )?;
        settings.add(
            "binding.linear.close_state",
            "사용자가 허용한 Linear 완료 상태 변경",
            &linear.close_state,
        )?;
    }
    settings.add(
        "authority.autonomy_cap",
        "자율 수준 상한",
        &policy.authority.autonomy_cap,
    )?;
    settings.add("labels.auto", "자동 요청 라벨", &policy.labels.auto)?;
    settings.add("labels.gated", "반자동 요청 라벨", &policy.labels.gated)?;
    settings.add("labels.manual", "수동 요청 라벨", &policy.labels.manual)?;
    settings.add("labels.hold", "보류 라벨", &policy.labels.hold)?;
    settings.add("select.mode", "이슈 선택 방식", &policy.select.mode)?;
    settings.add("select.labels_any", "선택 라벨", &policy.select.labels_any)?;
    settings.add(
        "select.trusted_actors",
        "신뢰하는 행위자의 안정 ID",
        &policy.select.trusted_actors,
    )?;
    settings.add(
        "classify.rule",
        "위험 등급 규칙 순서",
        &policy.classify.rule,
    )?;
    for (mode, mode_label, risks) in [
        ("propose", "제안", &policy.gates.propose),
        ("gated", "반자동", &policy.gates.gated),
        ("auto", "자동", &policy.gates.auto),
    ] {
        for (risk, risk_label, gates) in [
            ("low", "낮음", &risks.low),
            ("normal", "보통", &risks.normal),
            ("high", "높음", &risks.high),
        ] {
            for (gate, gate_label, action) in [
                ("start", "시작", gates.start),
                ("spec", "명세", gates.spec),
                ("plan", "계획", gates.plan),
                ("delivery", "전달", gates.delivery),
                ("merge", "통합", gates.merge),
            ] {
                settings.add(
                    &format!("gates.{mode}.{risk}.{gate}"),
                    &format!("{mode_label} · {risk_label} · {gate_label} 관문"),
                    &action,
                )?;
            }
        }
    }
    settings.add(
        "timeouts.start",
        "시작 제한 시간",
        &format!("{}s", policy.timeouts.start_seconds),
    )?;
    settings.add(
        "timeouts.remind_before",
        "시작 전 알림 시간",
        &format!("{}s", policy.timeouts.remind_before_seconds),
    )?;
    settings.add(
        "budget.max_running",
        "동시 실행 상한",
        &policy.budget.max_running,
    )?;
    settings.add(
        "budget.max_started_per_day",
        "하루 시작 상한",
        &policy.budget.max_started_per_day,
    )?;
    settings.add(
        "budget.max_attempts",
        "시도 횟수 상한",
        &policy.budget.max_attempts,
    )?;
    settings.add(
        "feedback.comment",
        "댓글 쓰기 허용",
        &policy.feedback.comment,
    )?;
    settings.add(
        "arbiter.min_confidence",
        "중재자 최소 확신 (만분율)",
        &policy.arbiter.min_confidence,
    )?;
    settings.add(
        "arbiter.max_running",
        "동시 중재자 호출 상한",
        &policy.arbiter.max_running,
    )?;
    settings.add(
        "arbiter.max_started_per_day",
        "하루 중재자 호출 상한",
        &policy.arbiter.max_started_per_day,
    )?;
    settings.add(
        "arbiter.timeout_seconds",
        "중재자 호출 제한 시간 (초)",
        &policy.arbiter.timeout_seconds,
    )?;
    settings.add("feedback.labels", "라벨 쓰기 허용", &policy.feedback.labels)?;
    settings.add(
        "feedback.transition",
        "상태 전이 쓰기 허용",
        &policy.feedback.transition,
    )?;
    settings.add(
        "feedback.telegram",
        "Telegram 알림 허용",
        &policy.feedback.telegram,
    )?;
    settings.add("close.when", "외부 이슈를 닫는 조건", &policy.close.when)?;
    settings.add("protection.paths", "보호 경로", &policy.protection.paths)?;
    settings.add(
        "human_required.rules",
        "사람이 필요한 내장 조건",
        &policy.human_required.rules,
    )?;
    settings.add(
        "human_required.additional",
        "사람이 필요한 추가 조건",
        &policy.human_required.additional,
    )?;
    Ok(settings.rows)
}

struct Settings<'a> {
    rows: Vec<IssueAutomationPolicySetting>,
    origins: &'a PolicyProvenance,
}

impl Settings<'_> {
    fn add<T: Serialize>(
        &mut self,
        key: &str,
        label: &str,
        value: &T,
    ) -> Result<(), IssueAutomationPolicyBlocked> {
        // 검증한 효과값만 표시 문자열로 만든다. 입력 TOML, 주석, 실패 원문은 들어오지 않는다.
        let value =
            serde_json::to_string_pretty(value).map_err(|_| IssueAutomationPolicyBlocked {
                scope: IssueAutomationPolicyScope::Effective,
                field: "policy",
                reason: "canonical_encoding_failed",
            })?;
        let origin = self.origins.for_field(key);
        self.rows.push(IssueAutomationPolicySetting {
            key: key.to_owned(),
            label: label.to_owned(),
            value,
            origins: origin
                .scopes
                .into_iter()
                .map(|scope| IssueAutomationPolicyOrigin {
                    scope,
                    label: scope_label(scope).to_owned(),
                })
                .collect(),
            resolution: origin.resolution,
            resolution_summary: resolution_summary(origin.resolution).to_owned(),
        });
        Ok(())
    }
}

fn scope_label(scope: IssueAutomationPolicyOriginScope) -> &'static str {
    match scope {
        IssueAutomationPolicyOriginScope::Builtin => "내장 기본값",
        IssueAutomationPolicyOriginScope::User => "사용자 정책",
        IssueAutomationPolicyOriginScope::Project => "저장소 정책",
    }
}

fn resolution_summary(resolution: IssueAutomationPolicyResolution) -> &'static str {
    match resolution {
        IssueAutomationPolicyResolution::Default => "내장 기본값을 사용합니다.",
        IssueAutomationPolicyResolution::Derived => {
            "최종 자동·반자동·수동 라벨 이름에서 선택 기본값을 만듭니다."
        }
        IssueAutomationPolicyResolution::Override => "가장 높은 우선순위의 명시한 값을 사용합니다.",
        IssueAutomationPolicyResolution::Minimum => {
            "내장·사용자 허용값과 저장소 제한값 중 작은 값을 사용합니다."
        }
        IssueAutomationPolicyResolution::Maximum => {
            "사용자 최소값과 저장소 제한값 중 큰 값을 사용합니다."
        }
        IssueAutomationPolicyResolution::And => {
            "내장·사용자 허용값과 저장소 설정이 모두 허용한 경우에만 켭니다."
        }
        IssueAutomationPolicyResolution::Intersection => {
            "내장·사용자 신뢰 목록과 저장소 목록에 함께 있는 행위자만 유지합니다."
        }
        IssueAutomationPolicyResolution::Union => {
            "내장 값에 사용자·저장소의 항목을 합치고 중복을 제거합니다."
        }
        IssueAutomationPolicyResolution::Cap => "요청 수준을 유효한 자율 상한 이하로 제한합니다.",
    }
}
