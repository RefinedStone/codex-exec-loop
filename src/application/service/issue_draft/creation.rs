//! Creation UI orchestration reuses the existing masking, policy and validation gates.
use super::*;
use crate::application::port::inbound::issue_draft_port::*;
use crate::application::service::issue_creation::{
    IssueCreationApprovedRequest, IssueCreationOutcome,
};
use crate::domain::issue_creation::{
    IssueCreationApproval, IssueCreationIntentRecord, IssueCreationIntentState,
};

pub(super) fn open(
    service: &IssueDraftService,
    target: IssueCreationTarget,
) -> Result<IssueCreationOpened> {
    let record = service.creator()?.open(&target)?;
    opened(service, record)
}

pub(super) fn preview(
    service: &IssueDraftService,
    request: IssueCreationPreviewRequest,
) -> Result<IssueCreationPreview> {
    let record = service
        .creator()?
        .intent(&request.target, &request.intent_id)?;
    let conventions = service.conventions(&record.repository.project_scope.workspace_dir)?;
    let text = frozen_text(&record).unwrap_or(request.text);
    let (preview, _) = evaluate(&conventions, &text, None);
    Ok(creation_preview(&record, preview))
}

pub(super) fn publish(
    service: &IssueDraftService,
    request: IssueCreationPublishRequest,
) -> Result<IssueCreationPublished> {
    let creator = service.creator()?;
    let record = creator.intent(&request.target, &request.intent_id)?;
    if record.state == IssueCreationIntentState::Abandoned {
        return Ok(IssueCreationPublished::Refused {
            reason: "intent_abandoned",
        });
    }
    // Once sent, a publish click only recovers the durable outcome. Policy changes
    // cannot turn an already completed remote creation into a new mutation.
    if record.state.may_have_been_sent() {
        if record
            .approved
            .as_ref()
            .is_none_or(|a| a.approval != request.approval)
        {
            return Ok(IssueCreationPublished::Refused {
                reason: "approval_mismatch",
            });
        }
        return Ok(published(creator.recheck(
            &request.target,
            &request.intent_id,
            service.vault_unlock.sealing_key().ok(),
        )?));
    }
    let conventions = service.conventions(&record.repository.project_scope.workspace_dir)?;
    let text = frozen_text(&record).unwrap_or(request.text);
    let (preview, passed) = evaluate(&conventions, &text, Some(service.code_validators.as_ref()));
    let preview = creation_preview(&record, preview);
    let Some((validated, masked_spans)) = passed else {
        return Ok(IssueCreationPublished::Blocked { preview });
    };
    if preview.approval.as_ref() != Some(&request.approval) {
        return Ok(IssueCreationPublished::Refused {
            reason: "approval_mismatch",
        });
    }
    let restorable = if record.state == IssueCreationIntentState::Prepared {
        restorable_fields(&masked_spans)
    } else {
        Vec::new()
    };
    let vault_key = ((!restorable.is_empty() || !record.pending_vault_fields.is_empty())
        && !request.shared_copy_only)
        .then(|| service.vault_unlock.sealing_key().ok())
        .flatten();
    match creator.publish(IssueCreationApprovedRequest {
        target: request.target,
        intent_id: request.intent_id,
        validated,
        approval: request.approval,
        policy_version: conventions.policy_version,
        template_version: conventions.template.version().map(str::to_string),
        restorable,
        vault_key,
        shared_copy_only: request.shared_copy_only,
    })? {
        IssueCreationOutcome::Intent(record) => Ok(published(*record)),
        IssueCreationOutcome::Refused(reason) => Ok(IssueCreationPublished::Refused { reason }),
    }
}

pub(super) fn recheck(
    service: &IssueDraftService,
    request: IssueCreationIntentRequest,
) -> Result<IssueCreationIntent> {
    Ok(creation_intent(&service.creator()?.recheck(
        &request.target,
        &request.intent_id,
        service.vault_unlock.sealing_key().ok(),
    )?))
}

pub(super) fn confirm(
    service: &IssueDraftService,
    request: IssueCreationConfirmRequest,
) -> Result<IssueCreationIntent> {
    Ok(creation_intent(&service.creator()?.confirm(
        &request.target,
        &request.intent_id,
        &request.url,
        service.vault_unlock.sealing_key().ok(),
    )?))
}

pub(super) fn restart(
    service: &IssueDraftService,
    request: IssueCreationRestartRequest,
) -> Result<IssueCreationOpened> {
    let record = service.creator()?.restart(&request)?;
    opened(service, record)
}

fn opened(
    service: &IssueDraftService,
    record: IssueCreationIntentRecord,
) -> Result<IssueCreationOpened> {
    if record.state.may_have_been_sent() || record.state == IssueCreationIntentState::Abandoned {
        return Ok(recovery_view(record));
    }
    let conventions = service.conventions(&record.repository.project_scope.workspace_dir)?;
    let template = template_view(&conventions.template);
    let text = frozen_text(&record).unwrap_or_else(|| IssueDraftText {
        title: String::new(),
        body: scaffold(&template, &conventions.policy.convention.required_sections),
        spans: Vec::new(),
    });
    let publish = conventions
        .policy
        .disclosure_for(IssueDisclosureTarget::Publish);
    Ok(IssueCreationOpened {
        repository: record.repository.clone(),
        intent: creation_intent(&record),
        title: text.title,
        body: text.body,
        template,
        required_sections: conventions.policy.convention.required_sections.clone(),
        span_actions: IssueDraftSpanActions {
            secret: publish.secret,
            name: publish.name,
            path: publish.path,
            log: publish.log,
        },
    })
}

/// Recovering a write does not depend on today's editable draft policy. A broken
/// policy must block a new write, not conceal a durable pending/completed outcome.
fn recovery_view(record: IssueCreationIntentRecord) -> IssueCreationOpened {
    use crate::domain::issue_policy::IssueSensitiveSpanAction::UserConfirm;

    let text = frozen_text(&record).unwrap_or_else(|| IssueDraftText {
        title: String::new(),
        body: String::new(),
        spans: Vec::new(),
    });
    IssueCreationOpened {
        repository: record.repository.clone(),
        intent: creation_intent(&record),
        title: text.title,
        body: text.body,
        template: IssueDraftTemplate {
            status: "recovery_only",
            path: None,
            reason: None,
            required_inputs: Vec::new(),
            unsupported_inputs: Vec::new(),
        },
        required_sections: Vec::new(),
        // No editor is available in these states, and no implicit disclosure policy
        // is substituted for an unreadable policy.
        span_actions: IssueDraftSpanActions {
            secret: UserConfirm,
            name: UserConfirm,
            path: UserConfirm,
            log: UserConfirm,
        },
    }
}

fn creation_preview(
    record: &IssueCreationIntentRecord,
    preview: IssueDraftPreview,
) -> IssueCreationPreview {
    let approval = match &preview {
        IssueDraftPreview::Ready {
            approval,
            publishable: true,
            ..
        } if matches!(
            record.state,
            IssueCreationIntentState::Prepared
                | IssueCreationIntentState::Held
                | IssueCreationIntentState::Recorded
        ) =>
        {
            let current = IssueCreationApproval {
                repository: record.repository.clone(),
                intent_id: record.intent_id.clone(),
                shared_copy_hash: approval.shared_copy_hash.clone(),
                policy_version: approval.policy_version.clone(),
                template_version: approval.template_version.clone(),
            };
            if record
                .approved
                .as_ref()
                .is_none_or(|frozen| frozen.approval == current)
            {
                Some(current)
            } else {
                None
            }
        }
        _ => None,
    };
    IssueCreationPreview { preview, approval }
}

fn frozen_text(record: &IssueCreationIntentRecord) -> Option<IssueDraftText> {
    record.approved.as_ref().map(|approved| IssueDraftText {
        title: approved.shared_copy.title.clone(),
        body: approved.shared_copy.body.clone(),
        spans: Vec::new(),
    })
}

fn creation_intent(record: &IssueCreationIntentRecord) -> IssueCreationIntent {
    IssueCreationIntent {
        intent_id: record.intent_id.clone(),
        state: record.state,
        created: record.created.clone(),
        shared_copy: record.approved.as_ref().map(|a| a.shared_copy.clone()),
        restore_link_required: !record.pending_vault_fields.is_empty(),
        restore_link_finalized: record.restore_link_finalized,
        predecessor_intent_id: record.predecessor_intent_id.clone(),
    }
}

fn published(record: IssueCreationIntentRecord) -> IssueCreationPublished {
    let intent = creation_intent(&record);
    match record.state {
        IssueCreationIntentState::Confirmed => IssueCreationPublished::Confirmed { intent },
        IssueCreationIntentState::Sent | IssueCreationIntentState::NeedsConfirmation => {
            IssueCreationPublished::NeedsConfirmation { intent }
        }
        IssueCreationIntentState::Abandoned => IssueCreationPublished::Refused {
            reason: "intent_abandoned",
        },
        _ => IssueCreationPublished::Held { intent },
    }
}

/// The existing template contract exposes required inputs, not arbitrary template
/// source. Seed only those sections; do not invent optional text or checked consent.
fn scaffold(template: &IssueDraftTemplate, required_sections: &[String]) -> String {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for input in &template.required_inputs {
        let index = sections
            .iter()
            .position(|(label, _)| label == &input.label)
            .unwrap_or_else(|| {
                sections.push((input.label.clone(), Vec::new()));
                sections.len() - 1
            });
        if input.kind == "checkbox" {
            sections[index]
                .1
                .extend(input.options.iter().map(|option| format!("- [ ] {option}")));
        }
    }
    for label in required_sections {
        if !sections.iter().any(|(existing, _)| existing == label) {
            sections.push((label.clone(), Vec::new()));
        }
    }
    sections
        .into_iter()
        .map(|(label, options)| format!("### {label}\n\n{}\n", options.join("\n")))
        .collect::<Vec<_>>()
        .join("\n")
}
