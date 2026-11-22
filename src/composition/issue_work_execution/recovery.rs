//! Only a currently observed terminal may register a fresh validation successor.
use anyhow::{Result, bail};

use super::{AkraIssueWorkExecutionAdapter, observation};
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkRecoveryOutcome, IssueWorkRecoveryReceipt, IssueWorkRecoveryRequest,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningValidationRecoveryKind, PlanningValidationRecoveryOutcome,
    PlanningValidationRecoveryRequest,
};
use crate::domain::issue_automation::execution::{
    IssueExecutionPhase, IssueExecutionTerminalClass,
};
use crate::domain::parallel_mode::PrValidationRecordKey;

fn blocked(request: &IssueWorkRecoveryRequest, reason: &str) -> IssueWorkRecoveryReceipt {
    IssueWorkRecoveryReceipt {
        command_id: request.command_id.clone(),
        terminal_record_id: request.terminal.record_id.clone(),
        outcome: IssueWorkRecoveryOutcome::Blocked {
            reason: reason.to_owned(),
        },
    }
}

pub(super) fn recover(
    adapter: &AkraIssueWorkExecutionAdapter,
    workspace: &str,
    request: &IssueWorkRecoveryRequest,
) -> Result<IssueWorkRecoveryReceipt> {
    request.validate()?;
    let kind = match request.terminal.classification {
        IssueExecutionTerminalClass::ObserveAgain => PlanningValidationRecoveryKind::ObserveAgain,
        IssueExecutionTerminalClass::QueueRecovery => PlanningValidationRecoveryKind::QueueRecovery,
        _ => return Ok(blocked(request, "terminal_requires_human_action")),
    };
    // Read revision before observation. Any task/lineage change during the read invalidates
    // new registration atomically; it cannot be attached to a later task generation.
    let Some(planning) = adapter.repository.load_task_authority_snapshot(workspace)? else {
        return Ok(blocked(request, "planning_authority_missing"));
    };
    let Some(order) = adapter
        .repository
        .load_work_order(workspace, &request.work_order_id)?
    else {
        return Ok(blocked(request, "work_order_missing"));
    };
    // Even when the current projection is unavailable, a completed immutable receipt can
    // still be read below. Such an observation never authorizes a new recovery.
    let current = observation::observe(adapter, workspace, &request.work_order_id)
        .ok()
        .flatten();
    let is_current_terminal = current.as_ref().is_some_and(|current| {
        matches!(
            current.phase,
            IssueExecutionPhase::Terminal | IssueExecutionPhase::Integrated
        ) && (current.integration.is_none()
            || request.terminal.classification == IssueExecutionTerminalClass::ObserveAgain)
            && current.current_terminal_id.as_deref() == Some(request.terminal.record_id.as_str())
    });
    let mut original = None;
    for key in adapter
        .repository
        .work_order_validation_record_keys(workspace, &request.work_order_id)?
    {
        let key = PrValidationRecordKey::new(key).map_err(anyhow::Error::msg)?;
        let Some(record) = adapter
            .authority
            .load_runtime_pr_validation_record(workspace, &key)?
        else {
            continue;
        };
        if record.key() != &key {
            bail!("validation recovery record identity differs from its work-order link");
        }
        if observation::validation_terminal(&record).as_ref() == Some(&request.terminal) {
            if original.replace(record).is_some() {
                bail!("validation recovery terminal identity is ambiguous");
            }
        }
    }
    let Some(original) = original else {
        return Ok(blocked(request, "validation_terminal_missing"));
    };
    let receipt = adapter.repository.recover_work_order_validation(
        workspace,
        PlanningValidationRecoveryRequest {
            command_id: &request.command_id,
            work_order_id: &request.work_order_id,
            terminal_record_id: &request.terminal.record_id,
            kind,
            authorization_epoch: request.authorization_epoch,
            gate_revision: request.gate_revision,
            policy_version: &request.policy_version,
            original: &original,
            observed_work_order: &order,
            observed_planning_revision: planning.planning_revision,
            is_current_terminal,
        },
    )?;
    Ok(IssueWorkRecoveryReceipt {
        command_id: receipt.command_id,
        terminal_record_id: receipt.terminal_record_id,
        outcome: match receipt.outcome {
            PlanningValidationRecoveryOutcome::Registered {
                validation_record_key,
            } => IssueWorkRecoveryOutcome::Registered {
                validation_record_key,
            },
            PlanningValidationRecoveryOutcome::Blocked { reason } => {
                IssueWorkRecoveryOutcome::Blocked { reason }
            }
        },
    })
}
