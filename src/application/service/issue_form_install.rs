//! 사용자 요청으로 고정 이슈 폼 하나를 별도 브랜치와 PR로 제안한다.
//! 자동 수집/모델 호출과 연결하지 않는다. Sent 이후에는 읽기로만 복구한다.
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::application::port::inbound::issue_form_install_port::{
    IssueFormInstallPort, IssueFormInstallPullRequest, IssueFormInstallReason,
    IssueFormInstallRequest, IssueFormInstallResponse, IssueFormInstallStatus,
};

use crate::application::port::outbound::github_automation_port::{
    GithubAutomationPort, GithubAutomationPullRequest, parse_github_repository_identity,
};
use crate::application::port::outbound::issue_form_repository_port::{
    ISSUE_FORM_PATH, IssueFormInstallPhase, IssueFormInstallSaveResult, IssueFormPreparation,
    IssueFormPullRequest, IssueFormRepository, IssueFormRepositoryPort, IssueFormTarget,
    StoredIssueFormInstall,
};

pub const ISSUE_FORM_ASSET: &str = include_str!("../../../assets/issue-automation/akra-work.yml");

pub struct IssueFormInstallService {
    git: Arc<dyn IssueFormRepositoryPort>,
    github: Arc<dyn GithubAutomationPort>,
}

impl IssueFormInstallService {
    pub fn new(
        git: Arc<dyn IssueFormRepositoryPort>,
        github: Arc<dyn GithubAutomationPort>,
    ) -> Self {
        Self { git, github }
    }

    pub fn install(&self, request: &IssueFormInstallRequest) -> Result<IssueFormInstallResponse> {
        let repository = self
            .git
            .inspect(&request.workspace_dir, &request.base_branch)?;
        let push_url = self.github.credential_redacted_push_url_for_remote(
            &repository.workspace_dir,
            &repository.push_remote,
        )?;
        let identity = self.github.repository_identity_for_push_url(
            &repository.workspace_dir,
            &repository.push_remote,
            &push_url,
        )?;
        if parse_github_repository_identity(&push_url)
            .is_none_or(|value| !value.eq_ignore_ascii_case(&identity))
        {
            bail!("issue form target identity is invalid");
        }
        let target = IssueFormTarget {
            repository: identity,
            push_remote: repository.push_remote.clone(),
            push_url,
            base_branch: request.base_branch.clone(),
            asset_sha256: format!("{:x}", Sha256::digest(ISSUE_FORM_ASSET.as_bytes())),
        };
        // A corrupt/symlinked private namespace must fail before any fetch/write.
        let previous = self.git.load(&repository, &target)?;
        let frozen_target = previous
            .as_ref()
            .map(|stored| stored.record.plan.target.clone())
            .unwrap_or(target);
        let base = self
            .remote_head(&repository, &frozen_target, &frozen_target.base_branch)?
            .ok_or_else(|| anyhow::anyhow!("issue form base branch is unavailable"))?;
        if !valid_oid(&base) {
            bail!("issue form base commit is invalid");
        }
        let fetched = self
            .github
            .fetch_branch_to_tracking_ref_for_delivery_target(
                &repository.workspace_dir,
                &frozen_target.push_remote,
                &frozen_target.push_url,
                &frozen_target.base_branch,
                &frozen_target.base_ref(),
            )?;
        if fetched != base {
            return Ok(simple_response(
                &frozen_target,
                Some(base),
                IssueFormInstallStatus::Conflict,
                Some(IssueFormInstallReason::BaseChanged),
            ));
        }
        let prepared = self
            .git
            .prepare(&repository, &frozen_target, &base, ISSUE_FORM_ASSET)?;
        match prepared {
            IssueFormPreparation::AlreadyInstalled => Ok(simple_response(
                &frozen_target,
                Some(base),
                IssueFormInstallStatus::AlreadyInstalled,
                None,
            )),
            IssueFormPreparation::Conflict => Ok(simple_response(
                &frozen_target,
                Some(base),
                IssueFormInstallStatus::Conflict,
                Some(IssueFormInstallReason::ExistingFormDiffers),
            )),
            IssueFormPreparation::Prepared(stored) => self.continue_plan(&repository, *stored),
        }
    }

    fn continue_plan(
        &self,
        repository: &IssueFormRepository,
        mut stored: StoredIssueFormInstall,
    ) -> Result<IssueFormInstallResponse> {
        // A finite CAS loop lets concurrent invocations observe the winner without retrying writes.
        for _ in 0..8 {
            let plan = &stored.record.plan;
            let target = &plan.target;
            match stored.record.phase {
                IssueFormInstallPhase::Prepared => {
                    let branch = self.remote_head(repository, target, &target.head_branch())?;
                    match branch.as_deref() {
                        Some(sha) if sha == plan.commit_sha => {
                            if let Some(next) = self.advance(
                                repository,
                                &stored,
                                IssueFormInstallPhase::BranchReady,
                                None,
                            )? {
                                stored = next;
                            } else {
                                stored = self.reload(repository, target)?;
                            }
                        }
                        Some(_) => {
                            return Ok(plan_response(
                                &stored,
                                IssueFormInstallStatus::Conflict,
                                Some(IssueFormInstallReason::BranchChanged),
                            ));
                        }
                        None => {
                            if !self.base_unchanged(repository, &stored)? {
                                return Ok(plan_response(
                                    &stored,
                                    IssueFormInstallStatus::Conflict,
                                    Some(IssueFormInstallReason::BaseChanged),
                                ));
                            }
                            let Some(sent) = self.advance(
                                repository,
                                &stored,
                                IssueFormInstallPhase::PushSent,
                                None,
                            )?
                            else {
                                stored = self.reload(repository, target)?;
                                continue;
                            };
                            // The exact-absence remote lease cannot overwrite another branch.
                            let target = &sent.record.plan.target;
                            let pushed = self.github.create_frozen_branch_for_delivery_target(
                                &repository.workspace_dir,
                                &target.push_remote,
                                &target.push_url,
                                &sent.record.plan.commit_sha,
                                &target.head_branch(),
                            );
                            if pushed.is_err() {
                                return Ok(plan_response(
                                    &sent,
                                    IssueFormInstallStatus::NeedsConfirmation,
                                    Some(IssueFormInstallReason::PushOutcomeUnknown),
                                ));
                            }
                            stored = sent;
                        }
                    }
                }
                IssueFormInstallPhase::PushSent => {
                    match self.remote_head(repository, target, &target.head_branch()) {
                        Ok(Some(sha)) if sha == plan.commit_sha => {
                            if let Some(next) = self.advance(
                                repository,
                                &stored,
                                IssueFormInstallPhase::BranchReady,
                                None,
                            )? {
                                stored = next;
                            } else {
                                stored = self.reload(repository, target)?;
                            }
                        }
                        Ok(Some(_)) => {
                            return Ok(plan_response(
                                &stored,
                                IssueFormInstallStatus::Conflict,
                                Some(IssueFormInstallReason::BranchChanged),
                            ));
                        }
                        // A crash before/after push cannot be distinguished. Do not send another push.
                        _ => {
                            return Ok(plan_response(
                                &stored,
                                IssueFormInstallStatus::NeedsConfirmation,
                                Some(IssueFormInstallReason::PushOutcomeUnknown),
                            ));
                        }
                    }
                }
                IssueFormInstallPhase::BranchReady => {
                    match self.find_matching_pull_request(repository, &stored)? {
                        PullRequestLookup::Matching(pr) => {
                            return self.confirm(
                                repository,
                                &stored,
                                *pr,
                                IssueFormInstallStatus::PullRequestFound,
                            );
                        }
                        PullRequestLookup::Mismatch => {
                            return Ok(plan_response(
                                &stored,
                                IssueFormInstallStatus::NeedsConfirmation,
                                Some(IssueFormInstallReason::PullRequestMismatch),
                            ));
                        }
                        PullRequestLookup::Missing => {}
                    }
                    if !self.branch_unchanged(repository, &stored)? {
                        return Ok(plan_response(
                            &stored,
                            IssueFormInstallStatus::Conflict,
                            Some(IssueFormInstallReason::BranchChanged),
                        ));
                    }
                    if !self.base_unchanged(repository, &stored)? {
                        return Ok(plan_response(
                            &stored,
                            IssueFormInstallStatus::Conflict,
                            Some(IssueFormInstallReason::BaseChanged),
                        ));
                    }
                    let Some(sent) =
                        self.advance(repository, &stored, IssueFormInstallPhase::Sent, None)?
                    else {
                        stored = self.reload(repository, target)?;
                        continue;
                    };
                    let target = &sent.record.plan.target;
                    // Only the CAS winner calls ensure, once. Its internal create is a remote write.
                    let result = self.github.ensure_pull_request_for_delivery_target(
                        &repository.workspace_dir, &target.push_remote, &target.push_url,
                        &target.base_branch, &target.head_branch(),
                        "chore: add Akra issue form",
                        "## 변경 내용\n\nAkra 이슈 초안의 완료 기준, 범위, 하지 않는 것을 입력하는 폼을 추가합니다.\n\n- 자동 선택 라벨을 붙이지 않습니다.\n- 기존 파일을 덮어쓰지 않으며 이 폼 하나만 추가합니다.\n",
                    );
                    match result {
                        Ok(pr) if valid_pull_request(&sent, &pr) => {
                            if self.branch_unchanged(repository, &sent).unwrap_or(false) {
                                return self.confirm(
                                    repository,
                                    &sent,
                                    pr,
                                    IssueFormInstallStatus::PullRequestCreated,
                                );
                            }
                            return Ok(plan_response(
                                &sent,
                                IssueFormInstallStatus::NeedsConfirmation,
                                Some(IssueFormInstallReason::PullRequestMismatch),
                            ));
                        }
                        _ => {
                            return Ok(plan_response(
                                &sent,
                                IssueFormInstallStatus::NeedsConfirmation,
                                Some(IssueFormInstallReason::PullRequestOutcomeUnknown),
                            ));
                        }
                    }
                }
                IssueFormInstallPhase::Sent | IssueFormInstallPhase::Confirmed => {
                    // Even a successful past response is revalidated against current base/head/SHA/target.
                    match self.find_matching_pull_request(repository, &stored) {
                        Ok(PullRequestLookup::Matching(pr)) => {
                            return self.confirm(
                                repository,
                                &stored,
                                *pr,
                                IssueFormInstallStatus::PullRequestFound,
                            );
                        }
                        Ok(PullRequestLookup::Mismatch) => {
                            return Ok(plan_response(
                                &stored,
                                IssueFormInstallStatus::NeedsConfirmation,
                                Some(IssueFormInstallReason::PullRequestMismatch),
                            ));
                        }
                        _ => {
                            return Ok(plan_response(
                                &stored,
                                IssueFormInstallStatus::NeedsConfirmation,
                                Some(IssueFormInstallReason::PullRequestOutcomeUnknown),
                            ));
                        }
                    }
                }
            }
        }
        Ok(plan_response(
            &stored,
            IssueFormInstallStatus::NeedsConfirmation,
            Some(IssueFormInstallReason::ConcurrentChange),
        ))
    }

    fn remote_head(
        &self,
        repository: &IssueFormRepository,
        target: &IssueFormTarget,
        branch: &str,
    ) -> Result<Option<String>> {
        self.github.remote_branch_head_for_delivery_target(
            &repository.workspace_dir,
            &target.push_remote,
            &target.push_url,
            branch,
        )
    }

    fn base_unchanged(
        &self,
        repository: &IssueFormRepository,
        stored: &StoredIssueFormInstall,
    ) -> Result<bool> {
        let plan = &stored.record.plan;
        Ok(self
            .remote_head(repository, &plan.target, &plan.target.base_branch)?
            .as_deref()
            == Some(plan.base_commit_sha.as_str()))
    }

    fn branch_unchanged(
        &self,
        repository: &IssueFormRepository,
        stored: &StoredIssueFormInstall,
    ) -> Result<bool> {
        let plan = &stored.record.plan;
        Ok(self
            .remote_head(repository, &plan.target, &plan.target.head_branch())?
            .as_deref()
            == Some(plan.commit_sha.as_str()))
    }

    fn find_matching_pull_request(
        &self,
        repository: &IssueFormRepository,
        stored: &StoredIssueFormInstall,
    ) -> Result<PullRequestLookup> {
        let target = &stored.record.plan.target;
        let found = self.github.find_pull_request_for_delivery_target(
            &repository.workspace_dir,
            &target.push_remote,
            &target.push_url,
            &target.base_branch,
            &target.head_branch(),
        )?;
        match found {
            Some(pr)
                if valid_pull_request(stored, &pr)
                    && self.branch_unchanged(repository, stored)? =>
            {
                Ok(PullRequestLookup::Matching(Box::new(pr)))
            }
            Some(_) => Ok(PullRequestLookup::Mismatch),
            None => Ok(PullRequestLookup::Missing),
        }
    }

    fn advance(
        &self,
        repository: &IssueFormRepository,
        stored: &StoredIssueFormInstall,
        phase: IssueFormInstallPhase,
        pull_request: Option<IssueFormPullRequest>,
    ) -> Result<Option<StoredIssueFormInstall>> {
        let mut candidate = stored.record.clone();
        candidate.phase = phase;
        candidate.pull_request = pull_request;
        match self.git.save(repository, stored, &candidate)? {
            IssueFormInstallSaveResult::Saved(next) => Ok(Some(*next)),
            IssueFormInstallSaveResult::Conflict => Ok(None),
        }
    }

    fn reload(
        &self,
        repository: &IssueFormRepository,
        target: &IssueFormTarget,
    ) -> Result<StoredIssueFormInstall> {
        self.git
            .load(repository, target)?
            .ok_or_else(|| anyhow::anyhow!("issue form install journal disappeared"))
    }

    fn confirm(
        &self,
        repository: &IssueFormRepository,
        stored: &StoredIssueFormInstall,
        pr: GithubAutomationPullRequest,
        status: IssueFormInstallStatus,
    ) -> Result<IssueFormInstallResponse> {
        let confirmed = IssueFormPullRequest {
            number: pr.number,
            url: pr.url,
        };
        if stored.record.phase == IssueFormInstallPhase::Confirmed {
            if stored.record.pull_request.as_ref() != Some(&confirmed) {
                return Ok(plan_response(
                    stored,
                    IssueFormInstallStatus::NeedsConfirmation,
                    Some(IssueFormInstallReason::PullRequestMismatch),
                ));
            }
            return Ok(plan_response(stored, status, None));
        }
        match self.advance(
            repository,
            stored,
            IssueFormInstallPhase::Confirmed,
            Some(confirmed.clone()),
        )? {
            Some(next) => Ok(plan_response(&next, status, None)),
            None => {
                let current = self.reload(repository, &stored.record.plan.target)?;
                if current.record.phase == IssueFormInstallPhase::Confirmed
                    && current.record.pull_request.as_ref() == Some(&confirmed)
                {
                    Ok(plan_response(&current, status, None))
                } else {
                    Ok(plan_response(
                        &current,
                        IssueFormInstallStatus::NeedsConfirmation,
                        Some(IssueFormInstallReason::ConcurrentChange),
                    ))
                }
            }
        }
    }
}

impl IssueFormInstallPort for IssueFormInstallService {
    fn install(&self, request: &IssueFormInstallRequest) -> Result<IssueFormInstallResponse> {
        IssueFormInstallService::install(self, request)
    }
}

enum PullRequestLookup {
    Missing,
    Matching(Box<GithubAutomationPullRequest>),
    Mismatch,
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_pull_request(stored: &StoredIssueFormInstall, pr: &GithubAutomationPullRequest) -> bool {
    let plan = &stored.record.plan;
    pr.number > 0
        && pr.state.eq_ignore_ascii_case("open")
        && pr.base_branch == plan.target.base_branch
        && pr.head_branch == plan.target.head_branch()
        && pr.head_commit_sha.as_deref() == Some(plan.commit_sha.as_str())
        && pr.url.eq_ignore_ascii_case(&format!(
            "https://github.com/{}/pull/{}",
            plan.target.repository, pr.number,
        ))
}

fn simple_response(
    target: &IssueFormTarget,
    base: Option<String>,
    status: IssueFormInstallStatus,
    reason: Option<IssueFormInstallReason>,
) -> IssueFormInstallResponse {
    IssueFormInstallResponse {
        status,
        path: ISSUE_FORM_PATH.to_owned(),
        base_branch: target.base_branch.clone(),
        head_branch: None,
        base_commit_sha: base,
        commit_sha: None,
        pull_request: None,
        reason,
    }
}

fn plan_response(
    stored: &StoredIssueFormInstall,
    status: IssueFormInstallStatus,
    reason: Option<IssueFormInstallReason>,
) -> IssueFormInstallResponse {
    let plan = &stored.record.plan;
    IssueFormInstallResponse {
        status,
        path: ISSUE_FORM_PATH.to_owned(),
        base_branch: plan.target.base_branch.clone(),
        head_branch: Some(plan.target.head_branch()),
        base_commit_sha: Some(plan.base_commit_sha.clone()),
        commit_sha: Some(plan.commit_sha.clone()),
        pull_request: stored.record.pull_request.as_ref().map(|pull_request| {
            IssueFormInstallPullRequest {
                number: pull_request.number,
                url: pull_request.url.clone(),
            }
        }),
        reason,
    }
}

#[cfg(test)]
mod tests;
