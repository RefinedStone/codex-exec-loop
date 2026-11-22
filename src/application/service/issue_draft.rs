mod creation;
mod readiness;

use crate::application::port::inbound::issue_draft_port::*;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_creation::IssueCreationService;
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::inbound::issue_draft_port::{
    IssueDraftApproval, IssueDraftField, IssueDraftIntent, IssueDraftMarker, IssueDraftOpened,
    IssueDraftPort, IssueDraftPreview, IssueDraftPreviewRequest, IssueDraftPublishRequest,
    IssueDraftPublished, IssueDraftRequiredInput, IssueDraftSpan, IssueDraftSpanActions,
    IssueDraftTarget, IssueDraftTemplate, IssueDraftText, IssueDraftUnsupportedInput,
    IssueDraftValidationError,
};
use crate::application::port::outbound::issue_code_validator_runner_port::IssueCodeValidatorRunnerPort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotResolveRequest, IssueSnapshotResponse,
};
use crate::application::service::evidence_vault_restore::restorable_fields;
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;
use crate::application::service::issue_code_validator::gate_issue_publish_with_code_validators;
use crate::application::service::issue_masking::{
    IssueMaskedSpan, IssueMaskingBlocked, IssueSensitiveSpan, IssueTextField, mask_issue_text,
};
use crate::application::service::issue_policy::{
    IssuePolicyService, issue_policy_version as policy_version,
};
use crate::application::service::issue_publish::{
    IssueApprovedPublishRequest, IssuePublishRefusal,
};
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishOutcome, IssueJournaledPublishRequest, IssueJournaledPublishService,
};
use crate::application::service::issue_template::IssueTemplateService;
use crate::application::service::issue_validation::{IssuePublishBlocked, gate_issue_publish};
use crate::domain::issue_policy::{IssueDisclosureTarget, IssuePolicy};
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};
use crate::domain::issue_template::{IssueTemplateInputKind, IssueTemplateResolution};
use crate::domain::issue_validation::{
    IssueFieldFormat, IssueFieldRequirement, IssueRequirements, IssueValidationError,
    IssueValidationErrorKind, IssueValidationField, IssueValidationPassed,
};
use crate::domain::issue_work::{
    IssueFieldHash, IssueLocator, IssueProjectScope, IssuePublishApproval, IssueReference,
    IssueWriteApproval, IssueWriteOperation,
};

/*
 * Desktop 의 이슈 초안 화면이 쓰는 서비스이다 (#2426). 앞의 조각을 잇기만 한다: 정책(#2416), 템플릿(#2417),
 * 검증(#2418), 마스킹(#2419), 게시 의도 기록 뒤의 게시(#2424). 규칙을 새로 만들지 않는다.
 *
 * 승인은 클라이언트가 미리보기에서 받은 값(공유본 해시, 정책 판, 템플릿 판)이다. 게시할 때 로컬 글을 다시 가리고
 * 정책과 템플릿을 다시 읽는다. 그 결과가 승인과 다르면 게시 서비스가 거절한다.
 *
 * 미리보기는 필수 규약 관문만 지난다. 게시는 코드 검증기가 있는 관문(#2431)을 지난다: 검증기는 프로그램을
 * 실행하므로 글자를 칠 때마다 돌리지 않는다.
 *
 * 복원 가능 구간은 게시 전에 Vault 에 저장한다 (#2429). 키는 잠금 해제 서비스의 `sealing_key` 이다. 키를 얻지
 * 못하거나 저장하지 못하면 게시는 `Held` 이고 사용자가 공유본만 게시할지 정한다. 게시가 `Confirmed` 가 되면 복원
 * 링크를 확정한다. 확정하지 못한 링크는 다음 `open` 또는 `recheck` 가 다시 확정한다.
 */
pub struct IssueDraftService {
    policy: IssuePolicyService,
    template: IssueTemplateService,
    snapshot: Arc<dyn IssueSnapshotPort>,
    publisher: Arc<IssueJournaledPublishService>,
    creator: Option<Arc<IssueCreationService>>,
    automation_policy: Option<Arc<IssueAutomationPolicyService>>,
    code_validators: Arc<dyn IssueCodeValidatorRunnerPort>,
    vault_unlock: Arc<EvidenceVaultUnlockService>,
}

/// 프로젝트의 지금 규약이다.
struct Conventions {
    policy: IssuePolicy,
    policy_version: String,
    template: IssueTemplateResolution,
}

impl IssueDraftService {
    pub fn new(
        policy: IssuePolicyService,
        template: IssueTemplateService,
        snapshot: Arc<dyn IssueSnapshotPort>,
        publisher: impl Into<Arc<IssueJournaledPublishService>>,
        code_validators: Arc<dyn IssueCodeValidatorRunnerPort>,
        vault_unlock: Arc<EvidenceVaultUnlockService>,
    ) -> Self {
        Self {
            policy,
            template,
            snapshot,
            publisher: publisher.into(),
            creator: None,
            automation_policy: None,
            code_validators,
            vault_unlock,
        }
    }

    pub fn with_creation(mut self, creator: Arc<IssueCreationService>) -> Self {
        self.creator = Some(creator);
        self
    }

    pub fn with_automation_policy(mut self, policy: Arc<IssueAutomationPolicyService>) -> Self {
        self.automation_policy = Some(policy);
        self
    }

    fn creator(&self) -> Result<&IssueCreationService> {
        self.creator
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("new-issue creation is unavailable"))
    }

    fn conventions(&self, cwd: &str) -> Result<Conventions> {
        let policy = self.policy.resolve(cwd)?;
        let template = self
            .template
            .resolve(cwd, policy.convention.template.as_deref());
        Ok(Conventions {
            policy_version: policy_version(&policy),
            policy,
            template,
        })
    }

    fn workspace_dir(&self, cwd: &str) -> Result<String> {
        match &self.creator {
            Some(creator) => creator.canonical_workspace(cwd),
            None => Ok(cwd.to_string()),
        }
    }

    fn scoped_target(&self, mut target: IssueDraftTarget) -> Result<IssueDraftTarget> {
        target.cwd = self.workspace_dir(&target.cwd)?;
        Ok(target)
    }

    fn resolve(&self, target: &IssueDraftTarget) -> Result<IssueSnapshotResponse> {
        self.snapshot.resolve(IssueSnapshotResolveRequest {
            locator: IssueLocator {
                owner: target.owner.clone(),
                repository: target.repository.clone(),
                number: target.number,
            },
            project_scope: IssueProjectScope {
                workspace_dir: target.cwd.clone(),
            },
        })
    }

    /// 이 이슈의 가장 최근 의도이다. 결과를 모르는 의도는 먼저 원격과 맞춘다.
    fn latest_intent(&self, cwd: &str, issue: &IssueReference) -> Result<Option<IssueDraftIntent>> {
        Ok(self
            .publisher
            .reconcile(cwd)?
            .into_iter()
            .rfind(|record| record.issue.same_remote_issue(issue))
            .map(|record| intent(&self.with_restore_link(record))))
    }

    /// 확인된 게시의 복원 링크를 확정한다. 확정하지 못하면 기록을 그대로 준다: 게시는 이미 확인되었고 다음
    /// `open` 또는 `recheck` 가 다시 확정한다.
    fn with_restore_link(&self, record: IssuePublishIntentRecord) -> IssuePublishIntentRecord {
        if record.state != IssuePublishIntentState::Confirmed
            || record.restore_link_finalized
            || record.pending_vault_fields.is_empty()
        {
            return record;
        }
        let Ok(key) = self.vault_unlock.sealing_key() else {
            return record;
        };
        self.publisher
            .finalize_restore_link(
                &record.issue.project_scope.workspace_dir,
                &record.intent_id,
                key,
            )
            .unwrap_or(record)
    }
}

impl IssueDraftPort for IssueDraftService {
    fn prepare_review_draft(
        &self,
        request: IssueDraftPreviewRequest,
        approval: IssueDraftApproval,
    ) -> Result<PreparedIssueDraftReview> {
        let target = self.scoped_target(request.target)?;
        let conventions = self.conventions(&target.cwd)?;
        let (preview, passed) = evaluate(&conventions, &request.text, None);
        let (
            IssueDraftPreview::Ready {
                approval: current, ..
            },
            Some((passed, _)),
        ) = (preview, passed)
        else {
            anyhow::bail!("issue draft did not pass masking and validation");
        };
        anyhow::ensure!(
            approval == current,
            "issue draft approval no longer matches"
        );
        let model = super::issue_review::model_request(&passed);
        Ok(PreparedIssueDraftReview {
            workspace: target.cwd,
            draft_json: model.draft_json,
        })
    }

    fn creation_open(&self, target: IssueCreationTarget) -> Result<IssueCreationOpened> {
        creation::open(self, target)
    }

    fn creation_preview(
        &self,
        request: IssueCreationPreviewRequest,
    ) -> Result<IssueCreationPreview> {
        creation::preview(self, request)
    }

    fn creation_publish(
        &self,
        request: IssueCreationPublishRequest,
    ) -> Result<IssueCreationPublished> {
        creation::publish(self, request)
    }

    fn creation_recheck(&self, request: IssueCreationIntentRequest) -> Result<IssueCreationIntent> {
        creation::recheck(self, request)
    }

    fn creation_confirm(
        &self,
        request: IssueCreationConfirmRequest,
    ) -> Result<IssueCreationIntent> {
        creation::confirm(self, request)
    }

    fn creation_restart(
        &self,
        request: IssueCreationRestartRequest,
    ) -> Result<IssueCreationOpened> {
        creation::restart(self, request)
    }

    fn open(&self, target: IssueDraftTarget) -> Result<IssueDraftOpened> {
        let target = self.scoped_target(target)?;
        let conventions = self.conventions(&target.cwd)?;
        let remote = self.resolve(&target)?;
        let publish = conventions
            .policy
            .disclosure_for(IssueDisclosureTarget::Publish);
        Ok(IssueDraftOpened {
            title: remote.raw_fields.expose_title().to_string(),
            body: remote.raw_fields.expose_body().to_string(),
            edit_basis: remote.comparison_basis.field_hash,
            template: template_view(&conventions.template),
            required_sections: conventions.policy.convention.required_sections.clone(),
            span_actions: IssueDraftSpanActions {
                secret: publish.secret,
                name: publish.name,
                path: publish.path,
                log: publish.log,
            },
            intent: self.latest_intent(&target.cwd, &remote.issue)?,
        })
    }

    fn preview(&self, request: IssueDraftPreviewRequest) -> Result<IssueDraftPreview> {
        let target = self.scoped_target(request.target)?;
        let conventions = self.conventions(&target.cwd)?;
        let mut preview = evaluate(&conventions, &request.text, None).0;
        readiness::attach(
            &mut preview,
            readiness::inspect(
                self.automation_policy.as_deref(),
                &target.cwd,
                &conventions,
                &request.text,
                &request.expected_labels,
            ),
        );
        Ok(preview)
    }

    fn publish(&self, request: IssueDraftPublishRequest) -> Result<IssueDraftPublished> {
        let target = self.scoped_target(request.target)?;
        let conventions = self.conventions(&target.cwd)?;
        let (preview, passed) = evaluate(
            &conventions,
            &request.text,
            Some(self.code_validators.as_ref()),
        );
        let Some((validated, masked_spans)) = passed else {
            return Ok(IssueDraftPublished::Blocked { preview });
        };
        let issue = self.resolve(&target)?.issue;
        let restorable = restorable_fields(&masked_spans);
        // 가릴 구간이 없거나 공유본만 게시하면 키 저장소를 부르지 않는다.
        let vault_key = (!restorable.is_empty() && !request.shared_copy_only)
            .then(|| self.vault_unlock.sealing_key().ok())
            .flatten();
        let outcome = self.publisher.publish(IssueJournaledPublishRequest {
            publish: IssueApprovedPublishRequest {
                issue: issue.clone(),
                validated,
                approval: IssuePublishApproval {
                    write: IssueWriteApproval {
                        issue,
                        operation: IssueWriteOperation::Body,
                    },
                    shared_copy_hash: request.approval.shared_copy_hash,
                    policy_version: request.approval.policy_version,
                    template_version: request.approval.template_version,
                },
                edit_basis: request.edit_basis,
                policy_version: conventions.policy_version,
                template_version: conventions.template.version().map(str::to_string),
            },
            restorable,
            vault_key,
            shared_copy_only: request.shared_copy_only,
        })?;
        Ok(match outcome {
            IssueJournaledPublishOutcome::Confirmed(record) => IssueDraftPublished::Confirmed {
                intent: intent(&self.with_restore_link(record)),
            },
            IssueJournaledPublishOutcome::NeedsConfirmation(record)
            | IssueJournaledPublishOutcome::AlreadyPending(record) => {
                IssueDraftPublished::NeedsConfirmation {
                    intent: intent(&record),
                }
            }
            IssueJournaledPublishOutcome::Held(record) => IssueDraftPublished::Held {
                intent: intent(&record),
            },
            IssueJournaledPublishOutcome::Conflict { .. } => IssueDraftPublished::Conflict,
            IssueJournaledPublishOutcome::Refused(refusal) => IssueDraftPublished::Refused {
                reason: match refusal {
                    IssuePublishRefusal::Target => "target",
                    IssuePublishRefusal::SharedCopy => "shared_copy",
                    IssuePublishRefusal::PolicyVersion => "policy_version",
                    IssuePublishRefusal::TemplateVersion => "template_version",
                },
            },
        })
    }

    fn recheck(&self, target: IssueDraftTarget) -> Result<Option<IssueDraftIntent>> {
        let target = self.scoped_target(target)?;
        let issue = self.resolve(&target)?.issue;
        self.latest_intent(&target.cwd, &issue)
    }

    fn abandon(&self, cwd: &str, intent_id: &str) -> Result<IssueDraftIntent> {
        Ok(intent(
            &self
                .publisher
                .abandon(&self.workspace_dir(cwd)?, intent_id)?,
        ))
    }
}

fn intent(record: &IssuePublishIntentRecord) -> IssueDraftIntent {
    IssueDraftIntent {
        intent_id: record.intent_id.clone(),
        state: record.state,
    }
}

/// 로컬 글을 가리고 게시 관문으로 검사한다. 통과하면 관문을 지난 글과 가림 구간도 준다.
/// `code_validators` 가 있으면 코드 검증기도 실행한다.
fn evaluate(
    conventions: &Conventions,
    text: &IssueDraftText,
    code_validators: Option<&dyn IssueCodeValidatorRunnerPort>,
) -> (
    IssueDraftPreview,
    Option<(IssueValidationPassed, Vec<IssueMaskedSpan>)>,
) {
    let mut spans = Vec::new();
    for (span_index, span) in text.spans.iter().enumerate() {
        match sensitive_span(text, span) {
            Some(span) => spans.push(span),
            None => {
                return (
                    IssueDraftPreview::InvalidSpan {
                        span_index,
                        readiness: None,
                    },
                    None,
                );
            }
        }
    }
    let disclosure = conventions
        .policy
        .disclosure_for(IssueDisclosureTarget::Publish);
    let masked = match mask_issue_text(&text.title, &text.body, &spans, disclosure) {
        Ok(masked) => masked,
        Err(IssueMaskingBlocked::NeedsUserConfirmation { span_indexes }) => {
            return (
                IssueDraftPreview::NeedsConfirmation {
                    span_indexes,
                    readiness: None,
                },
                None,
            );
        }
        Err(IssueMaskingBlocked::InvalidSpan { span_index }) => {
            return (
                IssueDraftPreview::InvalidSpan {
                    span_index,
                    readiness: None,
                },
                None,
            );
        }
    };
    let requirements = requirements(conventions);
    let gate = match code_validators {
        Some(runner) => gate_issue_publish_with_code_validators(
            &masked.shared_copy,
            requirements.as_ref(),
            &conventions.policy,
            runner,
        ),
        None => gate_issue_publish(&masked.shared_copy, requirements.as_ref()),
    };
    let (errors, validator_absent) = match &gate {
        Ok(_) => (Vec::new(), false),
        Err(IssuePublishBlocked::ValidatorAbsent) => (Vec::new(), true),
        Err(IssuePublishBlocked::Invalid(errors)) => {
            (errors.iter().map(validation_error).collect(), false)
        }
        Err(IssuePublishBlocked::CodeValidator(results)) => (
            results
                .iter()
                .map(|result| IssueDraftValidationError {
                    field: "code_validator",
                    label: Some(result.id.clone()),
                    kind: "failed",
                })
                .collect(),
            false,
        ),
    };
    let preview = IssueDraftPreview::Ready {
        markers: masked
            .masked_spans
            .iter()
            .map(|span| IssueDraftMarker {
                marker: span.marker.clone(),
                field: match span.field {
                    IssueTextField::Title => IssueDraftField::Title,
                    IssueTextField::Body => IssueDraftField::Body,
                },
            })
            .collect(),
        errors,
        validator_absent,
        publishable: gate.is_ok(),
        approval: IssueDraftApproval {
            shared_copy_hash: IssueFieldHash::of_fields(
                &masked.shared_copy.title,
                &masked.shared_copy.body,
            ),
            policy_version: conventions.policy_version.clone(),
            template_version: conventions.template.version().map(str::to_string),
        },
        shared_copy: masked.shared_copy,
        readiness: None,
    };
    (
        preview,
        gate.ok().map(|passed| (passed, masked.masked_spans)),
    )
}

/// 클라이언트의 구간(UTF-16 위치)을 마스킹의 구간(바이트 위치)으로 옮긴다. 글 밖이나 문자 중간이면 `None` 이다.
fn sensitive_span(text: &IssueDraftText, span: &IssueDraftSpan) -> Option<IssueSensitiveSpan> {
    let (field, value) = match span.field {
        IssueDraftField::Title => (IssueTextField::Title, &text.title),
        IssueDraftField::Body => (IssueTextField::Body, &text.body),
    };
    Some(IssueSensitiveSpan {
        field,
        range: byte_offset(value, span.start)?..byte_offset(value, span.end)?,
        class: span.class,
        user_decision: span.decision,
    })
}

fn byte_offset(text: &str, utf16_offset: usize) -> Option<usize> {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units == utf16_offset {
            return Some(index);
        }
        units += character.len_utf16();
    }
    (units == utf16_offset).then_some(text.len())
}

/// 검증기의 입력이다. 템플릿을 읽지 못했으면 `None` 이다: 검증기가 없고 게시를 막는다.
fn requirements(conventions: &Conventions) -> Option<IssueRequirements> {
    let template_fields = match &conventions.template {
        IssueTemplateResolution::NotConfigured => Vec::new(),
        IssueTemplateResolution::Resolved(convention)
            if !convention.unsupported_required_inputs.is_empty() =>
        {
            return None;
        }
        IssueTemplateResolution::Resolved(convention) => convention
            .required_inputs
            .iter()
            .map(|input| IssueFieldRequirement {
                label: input.label.clone(),
                required: true,
                format: match &input.kind {
                    IssueTemplateInputKind::Input => Some(IssueFieldFormat::SingleLine),
                    IssueTemplateInputKind::Dropdown { options } => {
                        Some(IssueFieldFormat::OneOf(options.clone()))
                    }
                    // shortcut: 체크 상자는 그 섹션에 값이 있는지만 본다, 검증 규칙에 체크 형식이 생기면 그것을 쓴다.
                    IssueTemplateInputKind::Section
                    | IssueTemplateInputKind::Textarea
                    | IssueTemplateInputKind::Checkbox { .. } => None,
                },
            })
            .collect(),
        IssueTemplateResolution::Missing { .. }
        | IssueTemplateResolution::Unreadable { .. }
        | IssueTemplateResolution::Invalid { .. } => return None,
    };
    Some(IssueRequirements {
        template_fields,
        required_sections: conventions.policy.convention.required_sections.clone(),
    })
}

fn validation_error(error: &IssueValidationError) -> IssueDraftValidationError {
    let (field, label) = match &error.field {
        IssueValidationField::Title => ("title", None),
        IssueValidationField::TemplateField(label) => ("template_field", Some(label.clone())),
        IssueValidationField::RequiredSection(title) => ("required_section", Some(title.clone())),
    };
    IssueDraftValidationError {
        field,
        label,
        kind: match error.kind {
            IssueValidationErrorKind::Missing => "missing",
            IssueValidationErrorKind::Empty => "empty",
            IssueValidationErrorKind::InvalidFormat => "invalid_format",
        },
    }
}

fn template_view(template: &IssueTemplateResolution) -> IssueDraftTemplate {
    let view = |status, path: Option<&String>, reason: Option<&String>| IssueDraftTemplate {
        status,
        path: path.cloned(),
        reason: reason.cloned(),
        required_inputs: Vec::new(),
        unsupported_inputs: Vec::new(),
    };
    match template {
        IssueTemplateResolution::NotConfigured => view("not_configured", None, None),
        IssueTemplateResolution::Missing { path } => view("missing", Some(path), None),
        IssueTemplateResolution::Unreadable { path, reason } => {
            view("unreadable", Some(path), Some(reason))
        }
        IssueTemplateResolution::Invalid { path, reason, .. } => {
            view("invalid", Some(path), Some(reason))
        }
        IssueTemplateResolution::Resolved(convention) => IssueDraftTemplate {
            required_inputs: convention
                .required_inputs
                .iter()
                .map(|input| {
                    let (kind, options) = match &input.kind {
                        IssueTemplateInputKind::Section => ("section", Vec::new()),
                        IssueTemplateInputKind::Input => ("input", Vec::new()),
                        IssueTemplateInputKind::Textarea => ("textarea", Vec::new()),
                        IssueTemplateInputKind::Dropdown { options } => {
                            ("dropdown", options.clone())
                        }
                        IssueTemplateInputKind::Checkbox { option } => {
                            ("checkbox", vec![option.clone()])
                        }
                    };
                    IssueDraftRequiredInput {
                        label: input.label.clone(),
                        kind,
                        options,
                    }
                })
                .collect(),
            unsupported_inputs: convention
                .unsupported_required_inputs
                .iter()
                .map(|input| IssueDraftUnsupportedInput {
                    label: input.label.clone(),
                    reason: input.reason.clone(),
                })
                .collect(),
            ..view("resolved", Some(&convention.path), None)
        },
    }
}

#[cfg(test)]
mod tests {
    mod creation;
    mod readiness;

    use std::sync::Mutex;

    use anyhow::bail;

    use super::*;
    use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
    use crate::application::port::outbound::evidence_vault_port::{
        EvidenceVaultStorePort, VaultPlaintext, VaultRecordLoadRequest, VaultRecordSaveRequest,
        VaultStoreError,
    };
    use crate::application::port::outbound::issue_code_validator_runner_port::{
        IssueCodeValidatorRunOutcome, IssueCodeValidatorRunRequest,
    };
    use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
    use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
    use crate::application::port::outbound::issue_snapshot_port::{
        IssueRawFields, IssueSnapshotReadRequest,
    };
    use crate::application::port::outbound::issue_template_source_port::IssueTemplateSourcePort;
    use crate::application::port::outbound::issue_update_port::{
        IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
    };
    use crate::application::port::outbound::vault_key_store_port::VaultKeyStoreError;
    use crate::domain::issue_policy::IssueSensitiveSpanClass;
    use crate::domain::issue_work::{IssueComparisonBasis, IssueProvider, IssueSharedCopy};

    const CWD: &str = "/work/project";
    const TEMPLATE_PATH: &str = ".github/ISSUE_TEMPLATE/bug.md";
    // 시험용 가짜 값이다. 실제 비밀이 아니다.
    const FAKE_SECRET: &str = "synthetic-secret-0000";

    /// 프로젝트의 정책 파일과 템플릿 파일이다. 시험 중에 바꿀 수 있다.
    struct FakeProjectFiles {
        policy: Mutex<String>,
        template: Mutex<String>,
    }

    impl IssuePolicySourcePort for FakeProjectFiles {
        fn load_user_policy_toml(&self) -> Result<Option<String>> {
            Ok(None)
        }

        fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
            Ok(Some(self.policy.lock().unwrap().clone()))
        }
    }

    impl IssueTemplateSourcePort for FakeProjectFiles {
        fn load_template(&self, _: &str, _: &str) -> Result<Option<String>> {
            Ok(Some(self.template.lock().unwrap().clone()))
        }
    }

    /// 메모리에 원격 이슈 한 건을 가진 가짜이다. 네트워크가 없다.
    struct FakeRemoteIssue {
        fields: Mutex<IssueSharedCopy>,
        writes: Mutex<usize>,
        lose_answer: bool,
    }

    impl FakeRemoteIssue {
        fn snapshot(&self, issue: IssueReference) -> IssueSnapshotResponse {
            let fields = self.fields.lock().unwrap().clone();
            let raw_fields = IssueRawFields::new(fields.title, fields.body);
            IssueSnapshotResponse {
                issue,
                comparison_basis: IssueComparisonBasis {
                    field_hash: raw_fields.field_hash(),
                    provider_updated_at: "2026-11-19T07:19:56Z".to_string(),
                },
                raw_fields,
            }
        }
    }

    impl IssueSnapshotPort for FakeRemoteIssue {
        fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
            Ok(self.snapshot(IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_1".to_string(),
                issue_id: format!("I_{}", request.locator.number),
                project_scope: request.project_scope,
            }))
        }

        fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
            Ok(self.snapshot(request.issue))
        }
    }

    impl IssueUpdatePort for FakeRemoteIssue {
        fn update_title_and_body(
            &self,
            request: IssueUpdateRequest,
        ) -> Result<IssueUpdateResponse> {
            request.ensure_authorized()?;
            *self.writes.lock().unwrap() += 1;
            if self.lose_answer {
                // 쓰기가 되었는지 모르는 경우이다: 원격은 바뀌지 않았고 답도 없다.
                bail!("the connection closed before the answer")
            }
            *self.fields.lock().unwrap() = request.shared_copy.clone();
            Ok(IssueUpdateResponse {
                issue: request.issue,
                published_copy: request.shared_copy,
            })
        }
    }

    #[derive(Default)]
    struct FakeJournal {
        records: Mutex<Vec<IssuePublishIntentRecord>>,
    }

    impl IssuePublishIntentJournalPort for FakeJournal {
        fn save(&self, _: &str, record: &IssuePublishIntentRecord) -> Result<()> {
            let mut records = self.records.lock().unwrap();
            records.retain(|existing| existing.intent_id != record.intent_id);
            records.push(record.clone());
            Ok(())
        }

        fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
            Ok(self.records.lock().unwrap().clone())
        }
    }

    /// 시험의 키 저장소는 키를 주지 않는다. Vault 를 부르면 시험이 실패한다.
    struct UnusedVault;

    impl EvidenceVaultStorePort for UnusedVault {
        fn begin_binding(
        &self,
        _request: VaultRecordLoadRequest,
        ) -> std::result::Result<Box<dyn crate::application::port::outbound::evidence_vault_port::VaultBindingTransaction + '_>, VaultStoreError>{
            unreachable!("this test does not use Vault binding transactions")
        }

        fn save(&self, _: VaultRecordSaveRequest) -> std::result::Result<(), VaultStoreError> {
            unreachable!("no vault key is wired")
        }

        fn load(
            &self,
            _: VaultRecordLoadRequest,
        ) -> std::result::Result<VaultPlaintext, VaultStoreError> {
            unreachable!("no vault key is wired")
        }
    }

    /// 시험의 정책은 코드 검증기를 고르지 않는다. 실행하면 시험이 실패한다.
    struct NoCodeValidators;

    impl IssueCodeValidatorRunnerPort for NoCodeValidators {
        fn run(&self, _: IssueCodeValidatorRunRequest) -> IssueCodeValidatorRunOutcome {
            unreachable!("the policy selects no code validator")
        }
    }

    struct Fixture {
        files: Arc<FakeProjectFiles>,
        remote: Arc<FakeRemoteIssue>,
        service: IssueDraftService,
    }

    fn fixture(lose_answer: bool) -> Fixture {
        let files = Arc::new(FakeProjectFiles {
            policy: Mutex::new(format!(
                "[convention]\ntemplate = \"{TEMPLATE_PATH}\"\nrequired_sections = [\"Impact\"]\n"
            )),
            template: Mutex::new("## Steps\n\n## Expected\n".to_string()),
        });
        let remote = Arc::new(FakeRemoteIssue {
            fields: Mutex::new(IssueSharedCopy {
                title: "Old title".to_string(),
                body: "Old body".to_string(),
            }),
            writes: Mutex::new(0),
            lose_answer,
        });
        let key_store = FakeVaultKeyStoreAdapter::new();
        key_store.fail_with(Some(VaultKeyStoreError::Unsupported));
        let service = IssueDraftService::new(
            IssuePolicyService::new(files.clone()),
            IssueTemplateService::new(files.clone()),
            remote.clone(),
            IssueJournaledPublishService::new(
                remote.clone(),
                remote.clone(),
                Arc::new(FakeJournal::default()),
                Arc::new(UnusedVault),
            ),
            Arc::new(NoCodeValidators),
            Arc::new(EvidenceVaultUnlockService::new(Arc::new(key_store))),
        );
        Fixture {
            files,
            remote,
            service,
        }
    }

    fn target() -> IssueDraftTarget {
        IssueDraftTarget {
            cwd: CWD.to_string(),
            owner: "sample".to_string(),
            repository: "widgets".to_string(),
            number: 7,
        }
    }

    fn valid_body() -> String {
        format!("## Steps\nRun with {FAKE_SECRET}\n\n## Expected\nNo crash\n\n## Impact\nLogin\n")
    }

    /// 본문의 가짜 비밀값을 `class` 의 구간으로 지정한 글이다.
    fn text_with_span(class: Option<IssueSensitiveSpanClass>) -> IssueDraftText {
        let body = valid_body();
        let start = body.find(FAKE_SECRET).unwrap();
        IssueDraftText {
            title: "New title".to_string(),
            spans: vec![IssueDraftSpan {
                field: IssueDraftField::Body,
                start,
                end: start + FAKE_SECRET.len(),
                class,
                decision: None,
            }],
            body,
        }
    }

    fn ready(preview: IssueDraftPreview) -> (IssueSharedCopy, IssueDraftApproval, bool) {
        match preview {
            IssueDraftPreview::Ready {
                shared_copy,
                approval,
                publishable,
                ..
            } => (shared_copy, approval, publishable),
            other => panic!("the preview is not ready: {other:?}"),
        }
    }

    fn publish_request(
        fixture: &Fixture,
        text: IssueDraftText,
        approval: IssueDraftApproval,
    ) -> IssueDraftPublishRequest {
        IssueDraftPublishRequest {
            target: target(),
            text,
            approval,
            edit_basis: fixture.service.open(target()).unwrap().edit_basis,
            shared_copy_only: false,
        }
    }

    #[test]
    fn open_gives_the_remote_text_its_basis_and_the_requirements() {
        let fixture = fixture(false);

        let opened = fixture.service.open(target()).unwrap();

        assert_eq!(opened.title, "Old title");
        assert_eq!(
            opened.edit_basis,
            IssueFieldHash::of_fields("Old title", "Old body")
        );
        assert_eq!(opened.template.status, "resolved");
        let labels: Vec<_> = opened
            .template
            .required_inputs
            .iter()
            .map(|input| input.label.as_str())
            .collect();
        assert_eq!(labels, ["Steps", "Expected"]);
        assert_eq!(opened.required_sections, ["Impact"]);
        assert_eq!(opened.intent, None);
    }

    #[test]
    fn preview_removes_a_secret_and_reports_each_missing_requirement() {
        let fixture = fixture(false);
        let mut text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
        text.body = text.body.replace("## Impact\nLogin\n", "");

        let preview = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                expected_labels: Vec::new(),
                target: target(),
                text,
            })
            .unwrap();

        let IssueDraftPreview::Ready {
            shared_copy,
            markers,
            errors,
            publishable,
            ..
        } = preview
        else {
            panic!("the preview is not ready")
        };
        assert!(!shared_copy.body.contains(FAKE_SECRET));
        assert!(shared_copy.body.contains("[removed]"));
        assert!(markers.is_empty());
        assert_eq!(
            errors,
            [IssueDraftValidationError {
                field: "required_section",
                label: Some("Impact".to_string()),
                kind: "missing",
            }]
        );
        assert!(!publishable);
    }

    #[test]
    fn codex_thread_review_receives_only_the_approved_masked_draft() {
        let fixture = fixture(false);
        let mut request = IssueDraftPreviewRequest {
            target: target(),
            text: text_with_span(Some(IssueSensitiveSpanClass::Secret)),
            expected_labels: Vec::new(),
        };
        let IssueDraftPreview::Ready { approval, .. } =
            fixture.service.preview(request.clone()).unwrap()
        else {
            panic!("expected reviewable draft");
        };
        let prepared = fixture
            .service
            .prepare_review_draft(request.clone(), approval.clone())
            .unwrap();
        assert!(!prepared.draft_json.contains(FAKE_SECRET));
        assert!(prepared.draft_json.contains("[removed]"));
        request.text.title.push_str(" changed");
        assert!(
            fixture
                .service
                .prepare_review_draft(request, approval)
                .is_err()
        );
    }

    #[test]
    fn preview_lists_the_markers_that_stay_and_asks_about_an_unclassified_span() {
        let fixture = fixture(false);

        let masked = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                expected_labels: Vec::new(),
                target: target(),
                text: text_with_span(Some(IssueSensitiveSpanClass::Log)),
            })
            .unwrap();
        let IssueDraftPreview::Ready {
            shared_copy,
            markers,
            ..
        } = masked
        else {
            panic!("the preview is not ready")
        };
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].field, IssueDraftField::Body);
        assert!(shared_copy.body.contains(&markers[0].marker));
        assert!(!shared_copy.body.contains(FAKE_SECRET));

        let unclassified = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                expected_labels: Vec::new(),
                target: target(),
                text: text_with_span(None),
            })
            .unwrap();
        assert!(matches!(
            unclassified,
            IssueDraftPreview::NeedsConfirmation { span_indexes, .. } if span_indexes == vec![0]
        ));
    }

    #[test]
    fn span_positions_are_utf16_units_of_the_client_text() {
        // "가" 는 UTF-16 한 단위, UTF-8 세 바이트이다. "😀" 는 UTF-16 두 단위이다.
        assert_eq!(byte_offset("가😀b", 1), Some(3));
        assert_eq!(byte_offset("가😀b", 3), Some(7));
        assert_eq!(byte_offset("가😀b", 4), Some(8));
        assert_eq!(byte_offset("가😀b", 2), None);
        assert_eq!(byte_offset("가😀b", 5), None);
    }

    #[test]
    fn an_approved_preview_publishes_and_the_publish_is_confirmed() {
        let fixture = fixture(false);
        let text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
        let (shared_copy, approval, publishable) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );
        assert!(publishable);

        let published = fixture
            .service
            .publish(publish_request(&fixture, text, approval))
            .unwrap();

        assert!(matches!(
            &published,
            IssueDraftPublished::Confirmed { intent }
                if intent.state == IssuePublishIntentState::Confirmed
        ));
        assert_eq!(*fixture.remote.fields.lock().unwrap(), shared_copy);
        let reopened = fixture.service.open(target()).unwrap();
        assert_eq!(
            reopened.intent.map(|intent| intent.state),
            Some(IssuePublishIntentState::Confirmed)
        );
    }

    #[test]
    fn an_approval_of_other_text_or_of_an_older_policy_does_not_publish() {
        let fixture = fixture(false);
        let text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
        let (_, approval, _) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );

        let mut edited = text.clone();
        edited.title = "Another title".to_string();
        let other_text = fixture
            .service
            .publish(publish_request(&fixture, edited, approval.clone()))
            .unwrap();
        assert_eq!(
            other_text,
            IssueDraftPublished::Refused {
                reason: "shared_copy"
            }
        );

        fixture
            .files
            .policy
            .lock()
            .unwrap()
            .push_str("\n[publish.spans]\nlog = \"remove\"\n");
        let older_policy = fixture
            .service
            .publish(publish_request(&fixture, text.clone(), approval.clone()))
            .unwrap();
        assert_eq!(
            older_policy,
            IssueDraftPublished::Refused {
                reason: "policy_version"
            }
        );

        // 정책은 그대로 두고 템플릿만 바꾼다.
        let (_, approval, _) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );
        fixture
            .files
            .template
            .lock()
            .unwrap()
            .push_str("\n<!-- note -->\n");
        let older_template = fixture
            .service
            .publish(publish_request(&fixture, text, approval))
            .unwrap();
        assert_eq!(
            older_template,
            IssueDraftPublished::Refused {
                reason: "template_version"
            }
        );
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
    }

    #[test]
    fn an_invalid_draft_is_blocked_without_a_remote_write() {
        let fixture = fixture(false);
        let mut text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
        let (_, approval, _) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );
        text.title = " ".to_string();

        let published = fixture
            .service
            .publish(publish_request(&fixture, text, approval))
            .unwrap();

        assert!(matches!(published, IssueDraftPublished::Blocked { .. }));
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
    }

    #[test]
    fn an_external_edit_after_open_is_a_conflict_without_a_write() {
        let fixture = fixture(false);
        let text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
        let (_, approval, _) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );
        let request = publish_request(&fixture, text, approval);
        fixture.remote.fields.lock().unwrap().body = "Edited by someone else".to_string();

        let published = fixture.service.publish(request).unwrap();

        assert_eq!(published, IssueDraftPublished::Conflict);
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
    }

    #[test]
    fn a_lost_answer_needs_confirmation_blocks_a_second_write_and_can_be_abandoned() {
        let fixture = fixture(true);
        let text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
        let (_, approval, _) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );

        let first = fixture
            .service
            .publish(publish_request(&fixture, text.clone(), approval.clone()))
            .unwrap();
        let IssueDraftPublished::NeedsConfirmation { intent } = first else {
            panic!("the first publish must need confirmation: {first:?}")
        };
        let second = fixture
            .service
            .publish(publish_request(&fixture, text, approval))
            .unwrap();
        assert_eq!(
            second,
            IssueDraftPublished::NeedsConfirmation {
                intent: intent.clone()
            }
        );
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 1);
        assert_eq!(
            fixture.service.recheck(target()).unwrap(),
            Some(intent.clone())
        );

        let abandoned = fixture.service.abandon(CWD, &intent.intent_id).unwrap();
        assert_eq!(abandoned.state, IssuePublishIntentState::Abandoned);
    }

    #[test]
    fn a_restorable_mask_is_held_until_the_user_publishes_the_shared_copy_only() {
        let fixture = fixture(false);
        let text = text_with_span(Some(IssueSensitiveSpanClass::Log));
        let (shared_copy, approval, _) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap(),
        );

        let mut request = publish_request(&fixture, text, approval);
        let held = fixture.service.publish(request.clone()).unwrap();
        assert!(matches!(held, IssueDraftPublished::Held { .. }));
        assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);

        request.shared_copy_only = true;
        let published = fixture.service.publish(request).unwrap();
        assert!(matches!(published, IssueDraftPublished::Confirmed { .. }));
        assert_eq!(*fixture.remote.fields.lock().unwrap(), shared_copy);
    }
    #[test]
    fn unsupported_required_form_inputs_and_invalid_forms_block_every_write() {
        for form in [
            "body:\n  - type: dropdown\n    attributes:\n      label: Systems\n      multiple: true\n      options: [Windows, macOS]\n    validations:\n      required: true\n",
            "body:\n  - type: upload\n    attributes:\n      label: Attachment\n    validations:\n      required: true\n",
            "body:\n  - type: checkboxes\n    attributes:\n      label: Checks\n      options:\n        - label: I checked\n          required: true\n",
            "body: [unterminated",
        ] {
            let fixture = fixture(false);
            let updated_policy = fixture
                .files
                .policy
                .lock()
                .unwrap()
                .replace(TEMPLATE_PATH, ".github/ISSUE_TEMPLATE/bug.yml");
            *fixture.files.policy.lock().unwrap() = updated_policy;
            *fixture.files.template.lock().unwrap() = form.to_string();
            let text = text_with_span(Some(IssueSensitiveSpanClass::Secret));
            let preview = fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    expected_labels: Vec::new(),
                    target: target(),
                    text: text.clone(),
                })
                .unwrap();
            assert!(matches!(
                &preview,
                IssueDraftPreview::Ready {
                    validator_absent: true,
                    publishable: false,
                    ..
                }
            ));
            let (_, approval, _) = ready(preview);
            let result = fixture
                .service
                .publish(publish_request(&fixture, text, approval))
                .unwrap();
            assert!(matches!(result, IssueDraftPublished::Blocked { .. }));
            assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
        }
    }
}
