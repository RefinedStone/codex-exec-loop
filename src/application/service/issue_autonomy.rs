//! Durable runtime selection; it does not enable collection or start a model turn.
use super::issue_automation_policy::{IssueAutomationPolicyService, ResolvedIssueAutomationPolicy};
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationDisplay, IssueConfirmationError, IssueConfirmationPreview,
    IssueConfirmationTarget, VerifiedHumanConfirmation,
};
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_work_execution_port::IssueWorkExecutionPort;
use crate::application::port::outbound::{
    IssueAutomationRestartEntry, IssueAutomationRestartRegistryPort,
};
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::autonomy::{
    IssueAutonomyChange, IssueAutonomyChoice, IssueAutonomyIntent, IssueAutonomyLevel,
    IssueAutonomyProjection, IssueAutonomyReady, IssueAutonomySelection, validate_command_id,
};
use crate::domain::issue_automation::intervention::IssueInterventionReceipt;
use crate::domain::issue_automation::work_control::IssueWorkControlAction;
use anyhow::{Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

/// Off/Observe/Propose inheritance preserves the legacy file hash. Automatic starts require a runtime choice.
pub(crate) fn effective_policy(
    mut file: ResolvedIssueAutomationPolicy,
    selection: &IssueAutonomySelection,
) -> ResolvedIssueAutomationPolicy {
    if let Some(level) = selection.requested_level {
        file.policy.set_selected_mode(
            IssueAutomationAutonomy::from(level).min(file.policy.selected_autonomy_cap()),
        );
        let encoded = serde_json::to_vec(&(
            "issue-runtime-autonomy-v1",
            &file.version,
            level,
            selection.revision,
        ))
        .expect("the runtime policy tuple is serializable");
        file.version = format!("{:x}", Sha256::digest(encoded));
    } else if file.policy.selected_mode() > IssueAutomationAutonomy::Propose {
        // A file can raise the ceiling, but cannot silently grant the first automatic start.
        file.policy
            .set_selected_mode(IssueAutomationAutonomy::Propose);
        let encoded = serde_json::to_vec(&("issue-runtime-unconfirmed-autonomy-v1", &file.version))
            .expect("the unconfirmed runtime policy is serializable");
        file.version = format!("{:x}", Sha256::digest(encoded));
    }
    file
}
pub(crate) fn resolve_runtime_policy(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    policy: &IssueAutomationPolicyService,
) -> Result<(ResolvedIssueAutomationPolicy, u64)> {
    let selection = store.issue_autonomy_selection(workspace)?;
    if selection.pending.is_some() {
        bail!("issue autonomy change is pending");
    }
    let file = policy.resolve(workspace)?;
    let revision = selection.revision;
    Ok((effective_policy(file, &selection), revision))
}
pub(crate) fn projection(
    workspace: &str,
    store: &dyn IssueAutomationStorePort,
    policy: &IssueAutomationPolicyService,
) -> IssueAutonomyProjection {
    let selection = match store.issue_autonomy_selection(workspace) {
        Ok(value) => value,
        Err(_) => return IssueAutonomyProjection::Unavailable,
    };
    let file = match policy.resolve(workspace) {
        Ok(value) => value,
        Err(_) => return IssueAutonomyProjection::PolicyBlocked,
    };
    let effective = effective_policy(file, &selection);
    let cap = effective.policy.selected_autonomy_cap();
    let mode = effective.policy.selected_mode();
    let pending = selection.pending.is_some();
    IssueAutonomyProjection::Ready(IssueAutonomyReady {
        requested_level: selection.requested_level,
        effective_level: mode,
        cap,
        revision: selection.revision,
        policy_version: effective.version,
        pending,
        choices: IssueAutonomyLevel::ALL
            .into_iter()
            .map(|level| {
                let requested = IssueAutomationAutonomy::from(level);
                let reason = if pending {
                    Some("자율 수준 변경을 복구하는 중입니다.".to_owned())
                } else if requested > cap {
                    Some(if effective.policy.binding.jira.is_some() && requested > IssueAutomationAutonomy::Propose {
                        "Jira Cloud는 연결된 PR의 부재를 확인할 수 없어 Propose와 사람 승인만 지원합니다.".to_owned()
                    } else {
                        "사용자 정책의 자율 수준 상한을 넘을 수 없습니다.".to_owned()
                    })
                } else {
                    None
                };
                IssueAutonomyChoice {
                    level,
                    enabled: reason.is_none(),
                    requires_confirmation: requires_confirmation(
                        level,
                        mode,
                        selection.requested_level,
                    ),
                    reason,
                }
            })
            .collect(),
    })
}
fn requires_confirmation(
    level: IssueAutonomyLevel,
    effective: IssueAutomationAutonomy,
    requested: Option<IssueAutonomyLevel>,
) -> bool {
    IssueAutomationAutonomy::from(level) > effective
        || (matches!(level, IssueAutonomyLevel::Gated | IssueAutonomyLevel::Auto)
            && requested != Some(level))
}

type RestartTargetResolver =
    dyn Fn(&str) -> Result<Option<IssueAutomationRestartEntry>> + Send + Sync;

struct RestartRegistryBinding {
    registry: Arc<dyn IssueAutomationRestartRegistryPort>,
    resolve_target: Arc<RestartTargetResolver>,
}

pub struct IssueAutonomyService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    policy: IssueAutomationPolicyService,
    execution: Arc<dyn IssueWorkExecutionPort>,
    command_gate: Mutex<()>,
    restart_registry: Option<RestartRegistryBinding>,
}
impl IssueAutonomyService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueAutomationStorePort>,
        policy: IssueAutomationPolicyService,
        execution: Arc<dyn IssueWorkExecutionPort>,
    ) -> Self {
        Self {
            workspace,
            store,
            policy,
            execution,
            command_gate: Mutex::new(()),
            restart_registry: None,
        }
    }
    /// Resolution is lazy: merely constructing a service or reading its state does not
    /// discover a Git identity or create the restart registry.
    pub fn with_restart_registry(
        mut self,
        registry: Arc<dyn IssueAutomationRestartRegistryPort>,
        resolve_target: impl Fn(&str) -> Result<Option<IssueAutomationRestartEntry>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.restart_registry = Some(RestartRegistryBinding {
            registry,
            resolve_target: Arc::new(resolve_target),
        });
        self
    }

    pub fn state(&self) -> IssueAutonomyProjection {
        projection(&self.workspace, self.store.as_ref(), &self.policy)
    }
    fn current(&self) -> Result<IssueAutonomyReady> {
        match self.state() {
            IssueAutonomyProjection::Ready(value) => Ok(value),
            _ => bail!("issue autonomy policy or state is unavailable"),
        }
    }
    fn target(
        &self,
        command_id: &str,
        level: IssueAutonomyLevel,
        current: &IssueAutonomyReady,
    ) -> IssueConfirmationTarget {
        IssueConfirmationTarget::Autonomy {
            workspace: self.workspace.clone(),
            command_id: command_id.to_owned(),
            from: current.effective_level,
            revision: current.revision,
            policy_version: current.policy_version.clone(),
            cap: current.cap,
            to: level,
        }
    }
    pub fn prepare_confirmation(
        &self,
        command_id: &str,
        level: IssueAutonomyLevel,
    ) -> Result<IssueConfirmationPreview> {
        validate_command_id(command_id).map_err(|reason| anyhow!(reason))?;
        let current = self.current()?;
        self.validate_choice(level, &current)?;
        let start_consent = match level {
            IssueAutonomyLevel::Gated => {
                "\nGated를 켜면 정책에서 자동 시작을 허용한 이슈를 매번 묻지 않고 등록합니다. 사용자 파일의 동시 작업·일일 시작·이슈별 시도 한도를 적용하며, 전달과 통합에는 사람 리뷰가 필요합니다. 실행은 별도로 켠 병렬 실행 설정을 따릅니다."
            }
            IssueAutonomyLevel::Auto => {
                "\nAuto를 켜면 정책이 허용한 이슈를 자동으로 시작하고 조건을 충족한 변경을 전달합니다. 공개 저장소이거나 공개 범위를 확인할 수 없으면 제안 단계로 제한합니다. 검증을 강제하도록 설정하고 필수 기준을 충족해야 전달할 수 있으며, 정책이 정한 사람 승인도 유지합니다. 이번 실행에서 자동 통합을 따로 허용하지 않았다면 PR에 사람 승인이 필요합니다. Auto 선택만으로 자동 통합이 켜지지는 않습니다. 동시 작업·일일 시작·이슈별 시도 한도와 별도로 켠 병렬 실행 설정을 따릅니다."
            }
            _ => "",
        };
        Ok(IssueConfirmationPreview {
            target: self.target(command_id, level, &current),
            display: IssueConfirmationDisplay {
                message: "이슈 자동화의 자율 수준을 변경할까요?".into(),
                detail: format!(
                    "현재: {:?}\n변경 후: {:?}\n사용자 정책 상한: {:?}\n진행 중인 작업의 기존 허가를 회수하고 현재 정책으로 다시 판정합니다.{}",
                    current.effective_level, level, current.cap, start_consent
                ),
                confirm_label: "수준 변경".into(),
            },
        })
    }
    fn validate_choice(
        &self,
        level: IssueAutonomyLevel,
        current: &IssueAutonomyReady,
    ) -> Result<()> {
        if current.pending {
            bail!("an issue autonomy change is pending");
        }
        if IssueAutomationAutonomy::from(level) > current.cap {
            bail!("the requested autonomy exceeds the user policy cap");
        }
        Ok(())
    }
    pub fn set(
        &self,
        command_id: String,
        level: IssueAutonomyLevel,
        confirmation: Option<&VerifiedHumanConfirmation>,
    ) -> Result<IssueAutonomyChange> {
        validate_command_id(&command_id).map_err(|reason| anyhow!(reason))?;
        let _guard = self
            .command_gate
            .lock()
            .map_err(|_| anyhow!("issue autonomy command gate is unavailable"))?;
        // A discovery registry failure must not prevent a new lowering command from
        // fencing old permits and committing the lower selection.
        self.recover_pending_selection()?;
        if let Some(stored) = self
            .store
            .get_issue_autonomy_command(&self.workspace, &command_id)?
        {
            if stored.intent.level != level {
                bail!("autonomy command ID was reused");
            }
            if stored.completed_revision.is_none() {
                bail!("autonomy command remains pending");
            }
            // An old command can be replayed after a newer selection. Reconcile the
            // current durable selection, never the historical command's level.
            self.reconcile_restart_registry()?;
            return Ok(IssueAutonomyChange {
                command_id,
                changed: false,
                autonomy: self.state(),
            });
        }
        let current = self.current()?;
        self.validate_choice(level, &current)?;
        if let Some(confirmation) = confirmation {
            confirmation.require_target(&self.target(&command_id, level, &current))?;
        }
        if requires_confirmation(level, current.effective_level, current.requested_level) {
            confirmation.ok_or(IssueConfirmationError::Required)?;
        }
        // Persist discovery before the pending selection: a crash immediately after
        // begin must still leave a candidate that startup can find. A candidate alone
        // is never authority; startup rechecks the durable selection under the repo lock.
        self.register_restart_candidate(level)?;
        let intent = IssueAutonomyIntent {
            command_id: command_id.clone(),
            level,
            expected_revision: current.revision,
            policy_version: current.policy_version,
        };
        let stored = self.store.begin_issue_autonomy(&self.workspace, &intent)?;
        let changed = if stored.completed_revision.is_none() {
            self.complete(&intent)?
        } else {
            false
        };
        // In particular, lowering removes discovery only after its fences and
        // completed receipt are durable. A failure is reported without rolling them back.
        self.reconcile_restart_registry()?;
        Ok(IssueAutonomyChange {
            command_id,
            changed,
            autonomy: self.state(),
        })
    }
    /// Composition calls this under the repository lock before constructing automatic runtimes.
    pub fn recover_before_runtime(&self) -> Result<()> {
        let _guard = self
            .command_gate
            .lock()
            .map_err(|_| anyhow!("issue autonomy command gate is unavailable"))?;
        self.recover_pending_selection()?;
        self.reconcile_restart_registry()
    }

    /// Manual attachment must remain available to lower a completed choice when only
    /// discovery persistence is broken. Incomplete selection/fence recovery stays strict.
    pub fn recover_before_manual_runtime(&self) -> Result<()> {
        let _guard = self
            .command_gate
            .lock()
            .map_err(|_| anyhow!("issue autonomy command gate is unavailable"))?;
        self.recover_pending_selection()?;
        // Authority read errors and a newly pending selection are not cache failures.
        let level = self.completed_selection_level()?;
        if let Err(error) = self.reconcile_restart_selection(level) {
            tracing::warn!(
                error = %error,
                "completed issue autonomy selection could not reconcile restart discovery; manual attachment remains available"
            );
        }
        Ok(())
    }

    fn recover_pending_selection(&self) -> Result<()> {
        if let Some(intent) = self
            .store
            .issue_autonomy_selection(&self.workspace)?
            .pending
        {
            self.complete(&intent)?;
        }
        Ok(())
    }
    fn register_restart_candidate(&self, level: IssueAutonomyLevel) -> Result<()> {
        if IssueAutomationAutonomy::from(level) < IssueAutomationAutonomy::Gated {
            return Ok(());
        }
        if let Some(binding) = &self.restart_registry
            && let Some(entry) = (binding.resolve_target)(&self.workspace)?
        {
            binding.registry.register(&entry)?;
        }
        Ok(())
    }

    fn completed_selection_level(&self) -> Result<Option<IssueAutonomyLevel>> {
        let selection = self.store.issue_autonomy_selection(&self.workspace)?;
        if selection.pending.is_some() {
            bail!("issue autonomy change is pending");
        }
        Ok(selection.requested_level)
    }

    fn reconcile_restart_registry(&self) -> Result<()> {
        self.reconcile_restart_selection(self.completed_selection_level()?)
    }

    fn reconcile_restart_selection(&self, level: Option<IssueAutonomyLevel>) -> Result<()> {
        let Some(binding) = &self.restart_registry else {
            return Ok(());
        };
        // Ordinary manual attachment without a runtime choice must not depend on a
        // discovery cache. Gateway removes a known pre-begin orphan under its repo lock.
        let Some(level) = level else {
            return Ok(());
        };
        let Some(entry) = (binding.resolve_target)(&self.workspace)? else {
            // Existing non-Git workspaces retain manual operation, without unattended restart.
            return Ok(());
        };
        if IssueAutomationAutonomy::from(level) >= IssueAutomationAutonomy::Gated {
            // Keep a requested automatic choice even when the current policy cap is lower.
            // Only current policy and selection, checked during startup, authorize execution.
            binding.registry.register(&entry)
        } else {
            binding.registry.unregister(&entry.repository_key)
        }
    }

    fn complete(&self, intent: &IssueAutonomyIntent) -> Result<bool> {
        // Pending selection and admission share one issue DB transaction fence. No new work
        // checkpoint can appear after this list; pre-existing intents are included before submit.
        super::issue_intervention::recover_pending_controls(
            &self.workspace,
            self.store.as_ref(),
            self.execution.as_ref(),
        )?;
        for work in self.store.list_issue_work(&self.workspace)? {
            let pending =
                match self
                    .store
                    .claim_issue_autonomy_fence(&self.workspace, intent, &work)
                {
                    Ok(pending) => pending,
                    Err(error) => {
                        // Another recoverer may have completed this entire autonomy intent.
                        if self
                            .store
                            .get_issue_autonomy_command(&self.workspace, &intent.command_id)?
                            .is_some_and(|stored| {
                                stored.intent == *intent && stored.completed_revision.is_some()
                            })
                        {
                            return Ok(false);
                        }
                        return Err(error);
                    }
                };
            let request = pending.request;
            let command_id = request.command_id.clone();
            if pending.receipt.is_some() {
                // Ownership proves a fresh epoch fence was created after this pending intent.
                // Generic recovery may have finished it without this explicit permit revocation.
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                continue;
            }
            {
                // This atomic planning epoch fences even an in-flight old grant with a newer SHA.
                let control = self.execution.apply_control(
                    &self.workspace,
                    &work.order.work_order_id,
                    &command_id,
                    IssueWorkControlAction::Retriage,
                )?;
                self.execution
                    .revoke(&self.workspace, &work.order.work_order_id)?;
                let receipt = IssueInterventionReceipt {
                    issue: work.order.issue.clone(),
                    command_id,
                    action: "retriage_issue".into(),
                    work_order_id: Some(work.order.work_order_id.clone()),
                    authorization_epoch: control.authorization_epoch,
                    changed: true,
                    delivery: None,
                    handoff: None,
                };
                if let Err(error) = self
                    .store
                    .finish_issue_intervention(&self.workspace, &receipt)
                {
                    // Startup recovery may have completed the same intent after this caller
                    // applied its fence. Its first receipt stays immutable, including changed.
                    let completed = self
                        .store
                        .get_issue_intervention(&self.workspace, &receipt.command_id)?;
                    if let Some(mut previous) = completed
                        .filter(|stored| stored.request == request)
                        .and_then(|stored| stored.receipt)
                    {
                        previous.changed = receipt.changed;
                        if previous == receipt
                            && self
                                .execution
                                .control(&self.workspace, &work.order.work_order_id)?
                                == control
                        {
                            continue;
                        }
                    }
                    return Err(error);
                }
            }
        }
        // Recovery may be completing a pending intent written before this process.
        // Confirm discovery again after fencing, before publishing automatic authority.
        self.register_restart_candidate(intent.level)?;
        Ok(self
            .store
            .finish_issue_autonomy(&self.workspace, intent)?
            .changed)
    }
}
