//! 이슈 게시, 로컬 복원, 외부 변경 흐름의 검증이다 (#2429).
//!
//! 실제로 도는 것: Gateway 세션(WebSocket)과 세션 녹화, `IssueDraftService`, `IssueJournaledPublishService`,
//! `IssueHandoffService`, `EvidenceVaultUnlockService`, `EvidenceVaultRestoreService`, 파일 일지 어댑터, 파일 Vault
//! 저장소 어댑터. 임시 폴더를 쓴다.
//!
//! 대역: GitHub(`GithubDouble`, 메모리), OS 키 저장소(`FakeVaultKeyStoreAdapter`), 프로젝트의 정책 파일과 템플릿 파일.
//! 네트워크, 실제 자격 증명, 모델 턴이 없다. 모든 글은 합성 값이다.
//!
//! "재시작"은 서비스와 세션을 버리고 같은 폴더 위에 다시 만드는 것이다. 원격(GitHub 대역)과 키 저장소는 프로세스
//! 밖에 있으므로 남는다.
//!
//! 복원은 Gateway 를 지나지 않는다 (복원 평문은 Gateway 응답에 넣지 않는다). 그래서 복원 서비스는 직접 부른다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio_tungstenite::WebSocketStream;

use super::super::hosts::GatewayHosts;
use super::super::recorder::GatewayRecorder;
use super::super::session::{GatewayServices, run_session, websocket_config};
use super::{DUPLEX_BUFFER_BYTES, EchoUpstream, kernel_request};
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;
use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
use crate::application::port::inbound::issue_handoff_port::{
    IssueHandoffPort, IssueHandoffRequest,
};
use crate::application::port::outbound::evidence_vault_port::VaultRestoreResponse;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorRunOutcome, IssueCodeValidatorRunRequest, IssueCodeValidatorRunnerPort,
};
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_template_source_port::IssueTemplateSourcePort;
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::application::service::evidence_vault_restore::{
    EvidenceVaultRestoreService, VaultApprovedMapping, VaultReconnectApproval,
    VaultReconnectReview, VaultRestoreHold,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultSharedCopyReason, VaultUnlockGrant, VaultUnlockOutcome,
    VaultUnlockRequest, VaultUserDecision,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_draft::IssueDraftService;
use crate::application::service::issue_handoff::IssueHandoffService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
use crate::application::service::issue_template::IssueTemplateService;
use crate::domain::issue_publish_intent::IssuePublishIntentState;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueLocator, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};

// 합성 값이다. 실제 비밀값이나 실제 로그가 아니다.
const SECRET: &str = "synthetic-secret-0000";
const LOG_LINE: &str = "synthetic-log worker=7 session=synthetic-session-1111";
const CWD: &str = "/work/synthetic-project";
const OWNER: &str = "sample";
const REPOSITORY: &str = "widgets";
const POLICY: &str = "[convention]\ntemplate = \".github/ISSUE_TEMPLATE/bug.md\"\nrequired_sections = [\"Impact\"]\n";
const TEMPLATE: &str = "## Steps\n\n## Expected\n";

#[derive(Default)]
struct GithubState {
    issues: HashMap<u64, IssueSharedCopy>,
    /// 실제로 받은 쓰기 요청이다: 이슈 번호와 보낸 글.
    writes: Vec<(u64, IssueSharedCopy)>,
    /// 다음 쓰기의 답을 잃는다. 값이 `true` 이면 쓰기는 원격에 반영되고, `false` 이면 반영되지 않는다.
    lose_next_answer: Option<bool>,
    /// 답을 잃을 때 읽기도 끊는다 (연결이 끊긴 상태).
    disconnect_on_lost_answer: bool,
    fail_reads: bool,
}

/// 메모리에 이슈를 가진 GitHub 대역이다. 조회와 쓰기의 계약을 모두 구현하고 받은 쓰기를 기록한다.
#[derive(Default)]
struct GithubDouble {
    state: Mutex<GithubState>,
}

impl GithubDouble {
    fn with_issues(numbers: &[u64]) -> Arc<Self> {
        let double = Self::default();
        for number in numbers {
            double.state().issues.insert(
                *number,
                IssueSharedCopy {
                    title: "Old title".to_string(),
                    body: "Old body".to_string(),
                },
            );
        }
        Arc::new(double)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, GithubState> {
        self.state.lock().unwrap()
    }

    fn issue(&self, number: u64) -> IssueSharedCopy {
        self.state().issues[&number].clone()
    }

    /// 다른 사람의 편집이다. 쓰기 기록에 남지 않는다.
    fn edit_externally(&self, number: u64, edit: impl FnOnce(&mut IssueSharedCopy)) {
        edit(self.state().issues.get_mut(&number).unwrap());
    }

    fn writes_to(&self, number: u64) -> usize {
        self.state()
            .writes
            .iter()
            .filter(|(written, _)| *written == number)
            .count()
    }

    fn snapshot(&self, issue: IssueReference) -> Result<IssueSnapshotResponse> {
        let state = self.state();
        if state.fail_reads {
            bail!("the synthetic provider read failed")
        }
        let fields = state
            .issues
            .get(&issue_number(&issue)?)
            .ok_or_else(|| anyhow!("the synthetic provider has no such issue"))?;
        let raw_fields = IssueRawFields::new(fields.title.clone(), fields.body.clone());
        Ok(IssueSnapshotResponse {
            issue,
            comparison_basis: IssueComparisonBasis {
                field_hash: raw_fields.field_hash(),
                provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
            },
            raw_fields,
        })
    }
}

fn reference(number: u64) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_1".to_string(),
        issue_id: format!("I_{number}"),
        project_scope: IssueProjectScope {
            workspace_dir: CWD.to_string(),
        },
    }
}

fn issue_number(issue: &IssueReference) -> Result<u64> {
    issue
        .issue_id
        .strip_prefix("I_")
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| anyhow!("the synthetic provider does not know this issue id"))
}

impl IssueSnapshotPort for GithubDouble {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot(IssueReference {
            project_scope: request.project_scope,
            ..reference(request.locator.number)
        })
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot(request.issue)
    }
}

impl IssueUpdatePort for GithubDouble {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        request.ensure_authorized()?;
        let number = issue_number(&request.issue)?;
        let mut state = self.state();
        state.writes.push((number, request.shared_copy.clone()));
        let lost = state.lose_next_answer.take();
        if lost != Some(false) {
            state.issues.insert(number, request.shared_copy.clone());
        }
        if lost.is_some() {
            state.fail_reads = state.disconnect_on_lost_answer;
            bail!("the connection closed before the answer")
        }
        Ok(IssueUpdateResponse {
            issue: request.issue,
            published_copy: request.shared_copy,
        })
    }
}

struct NoCreatedIssues;

impl crate::application::port::outbound::issue_creation_journal_port::IssueCreationJournalPort
    for NoCreatedIssues
{
    fn canonical_workspace(&self, cwd: &str) -> Result<String> {
        Ok(cwd.to_string())
    }

    fn read_records(
        &self,
        _: &str,
    ) -> Result<Vec<crate::domain::issue_creation::IssueCreationIntentRecord>> {
        Ok(Vec::new())
    }

    fn begin(&self, _: &str) -> Result<Box<dyn crate::application::port::outbound::issue_creation_journal_port::IssueCreationJournalTransaction + '_>>{
        unreachable!("hand-off never acquires the creation writer lease")
    }
}

/// 프로젝트의 정책 파일과 템플릿 파일의 대역이다. 사용자 파일은 없다.
struct ProjectFiles;

impl IssuePolicySourcePort for ProjectFiles {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(Some(POLICY.to_string()))
    }
}

impl IssueTemplateSourcePort for ProjectFiles {
    fn load_template(&self, _: &str, _: &str) -> Result<Option<String>> {
        Ok(Some(TEMPLATE.to_string()))
    }
}

/// 자동화 정책만 바꾸는 메모리 대역이다. 게시 공개 정책·템플릿과는 독립적이다.
struct AutomationFiles {
    policy: Mutex<String>,
}

impl IssueAutomationPolicySourcePort for AutomationFiles {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(Some(self.policy.lock().unwrap().clone()))
    }

    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

/// 시험의 정책은 코드 검증기를 고르지 않는다. 실행하면 시험이 실패한다.
struct NoCodeValidators;

impl IssueCodeValidatorRunnerPort for NoCodeValidators {
    fn run(&self, _: IssueCodeValidatorRunRequest) -> IssueCodeValidatorRunOutcome {
        unreachable!("the policy selects no code validator")
    }
}

/// 프로세스가 끝나도 남는 것이다: 디스크의 폴더, 원격, OS 키 저장소.
struct World {
    root: PathBuf,
    github: Arc<GithubDouble>,
    key_store: Arc<FakeVaultKeyStoreAdapter>,
    /// 모든 "프로세스"에서 받은 Gateway 응답이다.
    answers: Vec<String>,
}

impl World {
    fn new(label: &str, issues: &[u64]) -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-flow-{label}-{}-{:08x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        // Production prepares private runtime directories before acquiring a publication lease.
        for directory in ["journal", "vault", "recording"] {
            crate::private_directory::ensure_private_directory(&root.join(directory)).unwrap();
        }
        Self {
            root,
            github: GithubDouble::with_issues(issues),
            key_store: Arc::new(FakeVaultKeyStoreAdapter::new()),
            answers: Vec::new(),
        }
    }

    fn journal(&self) -> IssuePublishIntentJournalAdapter {
        let directory = self.root.join("journal");
        IssuePublishIntentJournalAdapter::new(move |_| Ok(directory.clone()))
    }

    /// 프로세스 하나를 시작한다: 서비스를 같은 폴더 위에 새로 만들고 Gateway 세션을 연다.
    async fn start(&self) -> Process {
        self.start_with_automation_policy(None).await
    }

    async fn start_with_automation_policy(
        &self,
        automation_policy: Option<Arc<IssueAutomationPolicyService>>,
    ) -> Process {
        let vault = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(
            self.root.join("vault"),
        ));
        let unlock = Arc::new(EvidenceVaultUnlockService::new(self.key_store.clone()));
        let drafts = IssueDraftService::new(
            IssuePolicyService::new(Arc::new(ProjectFiles)),
            IssueTemplateService::new(Arc::new(ProjectFiles)),
            self.github.clone(),
            IssueJournaledPublishService::new(
                self.github.clone(),
                self.github.clone(),
                Arc::new(self.journal()),
                vault.clone(),
            ),
            Arc::new(NoCodeValidators),
            unlock.clone(),
        );
        let drafts = match automation_policy {
            Some(policy) => drafts.with_automation_policy(policy),
            None => drafts,
        };
        let drafts = Arc::new(drafts);
        let handoff = Arc::new(
            IssueHandoffService::new(self.github.clone(), Arc::new(ProjectFiles))
                .with_publish_journal(Arc::new(self.journal()), |cwd| Ok(cwd.to_string()))
                .with_creation_journal(Arc::new(NoCreatedIssues)),
        );
        let recorder = Arc::new(GatewayRecorder::create(&self.root.join("recording")).unwrap());
        let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
        let services = GatewayServices {
            issue_drafts: Some(drafts),
            issue_handoff: Some(handoff.clone()),
            ..Default::default()
        };
        tokio::spawn(async move {
            let _ = run_session(
                gateway_side,
                GatewayHosts::new(EchoUpstream::new(true), Vec::new()),
                Arc::new(
                    crate::composition::repository_kernel::RepositoryKernelRegistry::default(),
                ),
                services,
                Some(recorder),
            )
            .await;
        });
        let (websocket, _) = tokio_tungstenite::client_async_with_config(
            "ws://localhost/",
            client_side,
            Some(websocket_config()),
        )
        .await
        .expect("client handshake through the gateway session should succeed");
        Process {
            websocket,
            next_id: 0,
            restore: EvidenceVaultRestoreService::new(self.github.clone(), vault, unlock.clone()),
            unlock,
            handoff,
        }
    }

    /// Gateway 요청 하나를 보내고 답을 받는다. 답은 나중의 평문 검사를 위해 모은다.
    async fn call(&mut self, process: &mut Process, method: &str, params: Value) -> Value {
        process.next_id += 1;
        let id = format!("r{}", process.next_id);
        let answer = kernel_request(&mut process.websocket, &id, method, params).await;
        self.answers.push(answer.to_string());
        answer
    }

    /// 편집을 열고, 미리보고, 승인한 값으로 게시한다. 게시의 답을 준다.
    async fn publish(&mut self, process: &mut Process, number: u64, text: Value) -> Value {
        let opened = self.call(process, "akra/issue/open", target(number)).await;
        assert!(opened["result"].is_object(), "{opened}");
        let preview = self
            .call(
                process,
                "akra/issue/preview",
                json!({ "target": target(number), "text": text }),
            )
            .await;
        assert_eq!(preview["result"]["publishable"], true, "{preview}");
        self.call(
            process,
            "akra/issue/publish",
            json!({
                "target": target(number),
                "text": text,
                "approval": preview["result"]["approval"],
                "editBasis": opened["result"]["editBasis"],
            }),
        )
        .await
    }

    /// 합성 평문이 지속 상태와 나간 글에 없는지 본다: 일지, Vault 파일, 세션 녹화, 모든 Gateway 응답,
    /// 공급자에 보낸 모든 쓰기.
    async fn assert_no_plaintext_left(&mut self, process: &mut Process) {
        // 녹화하는 요청을 마지막에 보낸다. 이 요청이 녹화에 있으면 녹화는 켜져 있고 앞의 요청을 모두 지났다.
        self.call(process, "akra/gateway/info", json!({})).await;
        let mut files = Vec::new();
        collect_files(&self.root, &mut files);
        let on_disk: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|path| {
                let body = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                (path, body)
            })
            .collect();
        for directory in ["journal", "recording"] {
            assert!(
                on_disk
                    .iter()
                    .any(|(path, body)| path.starts_with(self.root.join(directory))
                        && !body.is_empty()),
                "the scan must see the {directory} files"
            );
        }
        assert!(
            on_disk
                .iter()
                .any(|(_, body)| body.contains("akra/gateway/info")),
            "the session recording must be on"
        );
        let writes = self.github.state().writes.clone();
        let sent = writes
            .iter()
            .map(|(_, copy)| format!("{}\n{}", copy.title, copy.body));
        let texts = on_disk
            .iter()
            .map(|(path, body)| (path.display().to_string(), body.clone()))
            .chain(
                self.answers
                    .iter()
                    .map(|answer| ("a gateway answer".to_string(), answer.clone())),
            )
            .chain(sent.map(|text| ("a provider write".to_string(), text)));
        for (place, text) in texts {
            for plaintext in [SECRET, LOG_LINE] {
                assert!(
                    !text.contains(plaintext),
                    "`{plaintext}` is in {place}: {text}"
                );
            }
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

/// 실행 중인 프로세스 하나의 것이다. 버리면 열람 세션과 서비스가 없어진다.
struct Process {
    websocket: WebSocketStream<DuplexStream>,
    next_id: u32,
    unlock: Arc<EvidenceVaultUnlockService>,
    restore: EvidenceVaultRestoreService,
    handoff: Arc<IssueHandoffService>,
}

impl Process {
    /// 사용자가 확인한 열람 세션의 권한이다.
    fn unlocked(&self, number: u64) -> VaultUnlockGrant {
        let session = self.unlock.begin_session();
        match self.unlock.unlock(VaultUnlockRequest {
            issue: reference(number),
            session,
            decision: VaultUserDecision::Confirmed,
        }) {
            VaultUnlockOutcome::Unlocked(grant) => grant,
            other => panic!("the unlock must succeed: {other:?}"),
        }
    }

    fn restore(&self, number: u64) -> Result<VaultRestoreResponse, VaultRestoreHold> {
        self.restore.restore_for_grant(&self.unlocked(number))
    }
}

fn target(number: u64) -> Value {
    json!({ "cwd": CWD, "owner": OWNER, "repository": REPOSITORY, "number": number })
}

/// 사용자의 로컬 글이다. 비밀값과 로그 한 줄이 있다.
fn local_body() -> String {
    format!(
        "## Steps\nRun with password={SECRET}\n{LOG_LINE}\n\n## Expected\nNo crash\n\n## Impact\nLogin\n"
    )
}

/// 비밀값은 `secret`(제거), 로그 한 줄은 `log`(복원 가능 가림)로 지정한 글이다.
fn local_text(title: &str) -> Value {
    let body = local_body();
    let span = |plaintext: &str, class: &str| {
        let start = body.find(plaintext).unwrap();
        json!({ "field": "body", "start": start, "end": start + plaintext.len(), "class": class })
    };
    json!({
        "title": title,
        "body": body,
        "spans": [span(SECRET, "secret"), span(LOG_LINE, "log")],
    })
}

/// 복원본의 본문이다: 로그 줄은 돌아오고, 제거한 비밀값은 돌아오지 않는다.
fn restored_body() -> String {
    local_body().replace(SECRET, "[removed]")
}

fn intent_state(answer: &Value) -> &Value {
    &answer["result"]["intent"]["state"]
}

#[tokio::test]
async fn a_validated_masked_edit_is_published_and_only_an_unlocked_restore_shows_the_log_line() {
    let mut world = World::new("publish", &[7]);
    let mut process = world.start().await;

    // 템플릿과 필수 섹션을 읽는다.
    let opened = world.call(&mut process, "akra/issue/open", target(7)).await;
    assert_eq!(opened["result"]["title"], "Old title", "{opened}");
    assert_eq!(opened["result"]["template"]["status"], "resolved");
    assert_eq!(opened["result"]["requiredSections"], json!(["Impact"]));
    assert_eq!(opened["result"]["spanActions"]["log"], "restorable_mask");

    // 필수 섹션이 없는 글은 미리보기가 오류를 주고, 게시는 원격에 쓰지 않는다.
    let mut incomplete = local_text("New title");
    incomplete["body"] = json!(local_body().replace("## Impact\nLogin\n", ""));
    let preview = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": incomplete }),
        )
        .await;
    assert_eq!(preview["result"]["publishable"], false);
    assert_eq!(
        preview["result"]["errors"],
        json!([{ "field": "required_section", "label": "Impact", "kind": "missing" }])
    );
    let blocked = world
        .call(
            &mut process,
            "akra/issue/publish",
            json!({
                "target": target(7),
                "text": incomplete,
                "approval": preview["result"]["approval"],
                "editBasis": opened["result"]["editBasis"],
            }),
        )
        .await;
    assert_eq!(blocked["result"]["status"], "blocked");
    assert_eq!(world.github.writes_to(7), 0);

    // 미리보기는 비밀값을 지우고 로그 줄을 표식으로 가린다.
    let text = local_text("New title");
    let preview = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text }),
        )
        .await;
    let marker = preview["result"]["markers"][0]["marker"]
        .as_str()
        .expect("the log line leaves one marker")
        .to_string();
    let shared_body = preview["result"]["sharedCopy"]["body"].as_str().unwrap();
    assert!(shared_body.contains("password=[removed]"));
    assert!(shared_body.contains(&marker));

    // 승인한 공유본이 기존 이슈에 한 번 쓰인다.
    let published = world.publish(&mut process, 7, text).await;
    assert_eq!(published["result"]["status"], "confirmed", "{published}");
    assert_eq!(world.github.writes_to(7), 1);
    assert_eq!(world.github.issue(7).body, shared_body);
    assert_eq!(world.github.issue(7).title, "New title");

    // 사용자가 확인을 취소하면 권한이 없다.
    let session = process.unlock.begin_session();
    assert_eq!(
        process.unlock.unlock(VaultUnlockRequest {
            issue: reference(7),
            session,
            decision: VaultUserDecision::Cancelled,
        }),
        VaultUnlockOutcome::SharedCopyOnly(VaultSharedCopyReason::AuthenticationCancelled)
    );

    // 확인한 열람 세션만 그 구간을 복원한다. 제거한 비밀값은 복원본에도 없다.
    let grant = process.unlocked(7);
    let restored = process.restore.restore_for_grant(&grant).unwrap();
    assert_eq!(restored.title.expose(), "New title");
    assert_eq!(restored.body.expose(), restored_body());
    assert!(restored.body.expose().contains(LOG_LINE));
    assert!(!format!("{restored:?}").contains(LOG_LINE));

    // 열람 세션이 끝나면 같은 권한으로 복원할 수 없다.
    process.unlock.end_session(grant.session());
    assert_eq!(
        process.restore.restore_for_grant(&grant),
        Err(VaultRestoreHold::Locked(
            VaultSharedCopyReason::SessionExpired
        ))
    );

    // 이슈 인계는 공유본만 준다.
    let handoff = world
        .call(&mut process, "akra/issue/handoffCopy", target(7))
        .await;
    assert_eq!(handoff["result"]["sharedCopy"]["body"], shared_body);

    world.assert_no_plaintext_left(&mut process).await;
    // Vault 에는 이 게시의 레코드가 있다 (검사가 빈 폴더를 본 것이 아니다).
    let mut vault_files = Vec::new();
    collect_files(&world.root.join("vault"), &mut vault_files);
    assert!(vault_files.len() >= 3, "{vault_files:?}");
}

#[tokio::test]
async fn an_external_edit_and_a_copied_marker_hold_the_restore_until_the_user_reconnects() {
    let mut world = World::new("reconnect", &[7, 8]);
    let mut process = world.start().await;
    let published = world
        .publish(&mut process, 7, local_text("New title"))
        .await;
    assert_eq!(published["result"]["status"], "confirmed", "{published}");
    let shared = world.github.issue(7);

    // 최신 조회가 실패하면 이전에 읽은 글로 복원하지 않는다.
    world.github.state().fail_reads = true;
    let grant = process.unlocked(7);
    assert_eq!(
        process.restore.restore_for_grant(&grant),
        Err(VaultRestoreHold::LatestReadFailed)
    );
    world.github.state().fail_reads = false;

    // 다른 사람이 게시본을 고쳤다. 표식은 그대로 있지만 복원을 보류한다.
    world.github.edit_externally(7, |copy| {
        copy.body = format!("Edited by someone else.\n\n{}", copy.body);
    });
    assert_eq!(
        process.restore.restore_for_grant(&grant),
        Err(VaultRestoreHold::PublishedCopyChanged)
    );

    // 표식이 있는 글을 다른 이슈로 복사했다. 그 이슈에는 연결이 없다.
    world
        .github
        .edit_externally(8, |copy| copy.body = shared.body.clone());
    let other_grant = process.unlocked(8);
    assert_eq!(
        process.restore.restore_for_grant(&other_grant),
        Err(VaultRestoreHold::NotBound)
    );
    assert_eq!(
        process.restore.review_reconnect(&other_grant).err(),
        Some(VaultRestoreHold::NotBound)
    );

    // 검토한 뒤에 게시본이 다시 바뀌면 재연결을 확정하지 않는다.
    let stale = process.restore.review_reconnect(&grant).unwrap();
    let approval = |review: &VaultReconnectReview| {
        VaultReconnectApproval::from_review(
            review,
            review
                .markers
                .iter()
                .map(|marker| VaultApprovedMapping {
                    marker: marker.marker.clone(),
                    place: marker.candidates[0].clone(),
                })
                .collect(),
        )
        .unwrap()
    };
    world
        .github
        .edit_externally(7, |copy| copy.body.push_str("\nOne more line.\n"));
    assert_eq!(
        process.restore.reconnect(&grant, &approval(&stale)),
        Err(VaultRestoreHold::ChangedDuringReview)
    );
    assert_eq!(
        process.restore.restore_for_grant(&grant),
        Err(VaultRestoreHold::PublishedCopyChanged)
    );

    // 사용자가 지금 게시본을 검토하고 승인하면 같은 이슈의 연결이 새 판으로 바뀐다.
    let review = process.restore.review_reconnect(&grant).unwrap();
    assert_eq!(review.issue, reference(7));
    assert_eq!(review.markers.len(), 1);
    assert_eq!(review.markers[0].candidates.len(), 1);
    assert!(!format!("{review:?}").contains("Edited by someone else"));
    let approved = approval(&review);
    process.restore.reconnect(&grant, &approved).unwrap();
    let vault_root = world.root.join("vault");
    let record_snapshot = || {
        let mut paths = Vec::new();
        collect_files(&vault_root, &mut paths);
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let body = std::fs::read(&path).unwrap();
                (path, body)
            })
            .collect::<Vec<_>>()
    };
    let committed_records = record_snapshot();
    process.restore.reconnect(&grant, &approved).unwrap();
    assert_eq!(record_snapshot(), committed_records);
    let conflicting = VaultReconnectApproval::from_review(&review, Vec::new()).unwrap();
    assert_eq!(
        process.restore.reconnect(&grant, &conflicting),
        Err(VaultRestoreHold::ChangedDuringReview)
    );
    assert_eq!(record_snapshot(), committed_records);
    let restored = process.restore.restore_for_grant(&grant).unwrap();
    assert_eq!(
        restored.body.expose(),
        format!(
            "Edited by someone else.\n\n{}\nOne more line.\n",
            restored_body()
        )
    );

    // 재연결은 원격에 쓰지 않고, 다른 이슈의 연결을 만들지 않는다.
    assert_eq!(world.github.writes_to(7), 1);
    assert_eq!(world.github.writes_to(8), 0);
    assert_eq!(
        process.restore.restore_for_grant(&other_grant),
        Err(VaultRestoreHold::NotBound)
    );

    // 서비스를 같은 폴더 위에 다시 만들고 새 grant로 같은 승인을 확인한다.
    drop(process);
    let mut process = world.start().await;
    let renewed_grant = process.unlocked(7);
    process
        .restore
        .reconnect(&renewed_grant, &approved)
        .unwrap();
    assert_eq!(record_snapshot(), committed_records);
    assert_eq!(
        process.restore(7).unwrap().body.expose(),
        restored.body.expose()
    );

    // 외부 편집은 복원과 승인된 판의 인계를 모두 보류한다.
    world.github.edit_externally(7, |copy| {
        copy.body.push_str(&format!("\npassword={SECRET}\n"));
    });
    assert_eq!(
        process.restore(7).err(),
        Some(VaultRestoreHold::PublishedCopyChanged)
    );
    let handoff = world
        .call(&mut process, "akra/issue/handoffCopy", target(7))
        .await;
    assert!(handoff["error"].is_object(), "{handoff}");

    // A reconnect authorizes local restoration only, not AI hand-off.
    let approved = world
        .journal()
        .load(CWD)
        .unwrap()
        .pop()
        .unwrap()
        .approved_hash;
    let refused = process.handoff.handoff_copy(IssueHandoffRequest {
        locator: IssueLocator {
            owner: OWNER.to_string(),
            repository: REPOSITORY.to_string(),
            number: 7,
        },
        workspace_dir: Some(CWD.to_string()),
        approved: Some(approved),
    });
    assert!(!format!("{refused:?}").contains(SECRET));
    assert!(refused.is_err());

    world.assert_no_plaintext_left(&mut process).await;
}

#[tokio::test]
async fn a_lost_publish_answer_and_a_restart_send_no_second_write_and_keep_the_restore() {
    let mut world = World::new("lost-answer", &[7]);
    let mut process = world.start().await;

    // 쓰기는 원격에 반영되었지만 답을 잃었다.
    world.github.state().lose_next_answer = Some(true);
    let text = local_text("New title");
    let opened = world.call(&mut process, "akra/issue/open", target(7)).await;
    let preview = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text }),
        )
        .await;
    let publish_params = json!({
        "target": target(7),
        "text": text,
        "approval": preview["result"]["approval"],
        "editBasis": opened["result"]["editBasis"],
    });
    let first = world
        .call(&mut process, "akra/issue/publish", publish_params.clone())
        .await;
    // 답을 잃었지만 재조회가 승인한 글을 보았다: 확인된 게시이다. 쓰기는 한 번이다.
    assert_eq!(first["result"]["status"], "confirmed", "{first}");
    assert_eq!(world.github.writes_to(7), 1);

    // 재시작한다. 같은 요청을 다시 보내면 편집 기준이 지금 원격과 다르므로 쓰지 않는다.
    drop(process);
    let mut process = world.start().await;
    let again = world
        .call(&mut process, "akra/issue/publish", publish_params)
        .await;
    assert_eq!(again["result"]["status"], "conflict", "{again}");
    assert_eq!(world.github.writes_to(7), 1);
    assert_eq!(process.restore(7).unwrap().body.expose(), restored_body());

    world.assert_no_plaintext_left(&mut process).await;
}

#[tokio::test]
async fn an_unknown_publish_result_survives_a_restart_without_an_automatic_second_write() {
    let mut world = World::new("unknown-result", &[7, 9]);
    let mut process = world.start().await;
    let text = local_text("New title");
    let params = |opened: &Value, preview: &Value, number: u64| {
        json!({
            "target": target(number),
            "text": local_text("New title"),
            "approval": preview["result"]["approval"],
            "editBasis": opened["result"]["editBasis"],
        })
    };
    let preview_params =
        |number: u64| json!({ "target": target(number), "text": local_text("New title") });

    // 이슈 7: 쓰기는 반영되었고, 답을 잃었고, 재조회도 실패했다. 결과를 모른다.
    let opened = world.call(&mut process, "akra/issue/open", target(7)).await;
    let preview = world
        .call(&mut process, "akra/issue/preview", preview_params(7))
        .await;
    let publish_7 = params(&opened, &preview, 7);
    world.github.state().lose_next_answer = Some(true);
    // 게시 서비스는 쓰기 전에 원격을 읽는다. 그래서 대역은 답을 잃는 순간부터 읽기를 끊는다.
    world.github.state().disconnect_on_lost_answer = true;
    let first = world
        .call(&mut process, "akra/issue/publish", publish_7.clone())
        .await;
    assert_eq!(first["result"]["status"], "needs_confirmation", "{first}");
    let intent_id = first["result"]["intent"]["intentId"].clone();
    assert_eq!(world.github.writes_to(7), 1);

    // 연결이 끊긴 채로 다시 눌러도 쓰지 않는다.
    let repeated = world
        .call(&mut process, "akra/issue/publish", publish_7.clone())
        .await;
    assert_ne!(repeated["result"]["status"], "confirmed", "{repeated}");
    assert_eq!(world.github.writes_to(7), 1);

    // 재시작한다. 연결이 돌아왔다. 다시 누른 게시는 앞의 의도를 주고 쓰지 않는다.
    drop(process);
    world.github.state().fail_reads = false;
    world.github.state().disconnect_on_lost_answer = false;
    let mut process = world.start().await;
    let after_restart = world
        .call(&mut process, "akra/issue/publish", publish_7)
        .await;
    assert_eq!(
        after_restart["result"]["status"], "needs_confirmation",
        "{after_restart}"
    );
    assert_eq!(after_restart["result"]["intent"]["intentId"], intent_id);
    assert_eq!(world.github.writes_to(7), 1);

    // 다시 확인하면 읽기만으로 확인된 게시가 된다. 복원 링크도 이때 확정된다.
    let rechecked = world
        .call(&mut process, "akra/issue/recheck", target(7))
        .await;
    assert_eq!(intent_state(&rechecked), "confirmed", "{rechecked}");
    assert_eq!(rechecked["result"]["intent"]["intentId"], intent_id);
    assert_eq!(world.github.writes_to(7), 1);
    assert_eq!(process.restore(7).unwrap().body.expose(), restored_body());

    // 이슈 9: 쓰기는 반영되지 않았고 답을 잃었다. 재조회는 예전 글을 본다. 결과를 모른다.
    let opened = world.call(&mut process, "akra/issue/open", target(9)).await;
    let preview = world
        .call(&mut process, "akra/issue/preview", preview_params(9))
        .await;
    let publish_9 = params(&opened, &preview, 9);
    world.github.state().lose_next_answer = Some(false);
    let first = world
        .call(&mut process, "akra/issue/publish", publish_9.clone())
        .await;
    assert_eq!(first["result"]["status"], "needs_confirmation", "{first}");
    let intent_id = first["result"]["intent"]["intentId"].clone();

    // 재시작 뒤에도 자동으로 다시 보내지 않는다: 열기, 다시 확인, 다시 누른 게시 모두 쓰지 않는다.
    drop(process);
    let mut process = world.start().await;
    let reopened = world.call(&mut process, "akra/issue/open", target(9)).await;
    assert_eq!(intent_state(&reopened), "needs_confirmation", "{reopened}");
    let rechecked = world
        .call(&mut process, "akra/issue/recheck", target(9))
        .await;
    assert_eq!(intent_state(&rechecked), "needs_confirmation");
    let repeated = world
        .call(&mut process, "akra/issue/publish", publish_9.clone())
        .await;
    assert_eq!(repeated["result"]["status"], "needs_confirmation");
    assert_eq!(world.github.writes_to(9), 1);
    assert_eq!(world.github.issue(9).body, "Old body");
    // 확인되지 않은 게시에는 복원 연결이 없다.
    assert_eq!(process.restore(9).err(), Some(VaultRestoreHold::NotBound));

    // 사용자가 결과 확인을 그만둔 뒤에만 새 승인으로 다시 게시한다.
    let abandoned = world
        .call(
            &mut process,
            "akra/issue/abandon",
            json!({ "cwd": CWD, "intentId": intent_id }),
        )
        .await;
    assert_eq!(intent_state(&abandoned), "abandoned", "{abandoned}");
    let published = world.publish(&mut process, 9, text).await;
    assert_eq!(published["result"]["status"], "confirmed", "{published}");
    assert_eq!(world.github.writes_to(9), 2);
    assert_eq!(process.restore(9).unwrap().body.expose(), restored_body());

    // 일지에 남은 상태이다: 이슈마다 확인된 의도가 하나이다.
    let confirmed = world
        .journal()
        .load(CWD)
        .unwrap()
        .into_iter()
        .filter(|record| record.state == IssuePublishIntentState::Confirmed)
        .count();
    assert_eq!(confirmed, 2);

    world.assert_no_plaintext_left(&mut process).await;
}

#[tokio::test]
async fn a_restorable_publish_is_held_without_a_write_when_the_key_store_fails() {
    use crate::application::port::outbound::vault_key_store_port::VaultKeyStoreError;

    let mut world = World::new("held", &[7]);
    world
        .key_store
        .fail_with(Some(VaultKeyStoreError::Unsupported));
    let mut process = world.start().await;
    let text = local_text("New title");

    let held = world.publish(&mut process, 7, text.clone()).await;
    assert_eq!(held["result"]["status"], "held", "{held}");
    assert_eq!(world.github.writes_to(7), 0);

    // 사용자가 복원 연결 없이 공유본만 게시한다고 확인하면 게시한다. 복원 연결은 없다.
    let opened = world.call(&mut process, "akra/issue/open", target(7)).await;
    let preview = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text }),
        )
        .await;
    let published = world
        .call(
            &mut process,
            "akra/issue/publish",
            json!({
                "target": target(7),
                "text": text,
                "approval": preview["result"]["approval"],
                "editBasis": opened["result"]["editBasis"],
                "sharedCopyOnly": true,
            }),
        )
        .await;
    assert_eq!(published["result"]["status"], "confirmed", "{published}");
    assert_eq!(world.github.writes_to(7), 1);
    world.key_store.fail_with(None);
    let session = process.unlock.begin_session();
    assert_eq!(
        process.unlock.unlock(VaultUnlockRequest {
            issue: reference(7),
            session,
            decision: VaultUserDecision::Confirmed,
        }),
        VaultUnlockOutcome::SharedCopyOnly(VaultSharedCopyReason::KeyLost)
    );

    world.assert_no_plaintext_left(&mut process).await;
}

#[tokio::test]
async fn the_gateway_hand_off_refuses_an_issue_edited_after_its_approved_publish() {
    let mut world = World::new("handoff-approved", &[7]);
    let mut process = world.start().await;
    let published = world
        .publish(&mut process, 7, local_text("New title"))
        .await;
    assert_eq!(published["result"]["status"], "confirmed", "{published}");
    let unchanged = world
        .call(&mut process, "akra/issue/handoffCopy", target(7))
        .await;
    assert!(unchanged["result"]["sharedCopy"].is_object(), "{unchanged}");
    world.github.edit_externally(7, |copy| {
        copy.body.push_str("\nEdited by someone else.\n");
    });

    let handoff = world
        .call(&mut process, "akra/issue/handoffCopy", target(7))
        .await;

    assert!(handoff["error"].is_object(), "{handoff}");
    world.assert_no_plaintext_left(&mut process).await;
}

fn readiness_text() -> Value {
    json!({
        "title": "합성 화면 상태 표시 작업",
        "body": "## Steps\n상태 화면을 연다.\n\n## Expected\n서버 상태가 표시된다.\n\n## Impact\n상태 확인 흐름\n\n## 완료 기준\n- 상태 문구가 서버 응답과 일치한다.\n- 상태 표시 회귀 테스트가 통과한다.\n",
        "spans": [],
    })
}

#[tokio::test]
async fn gateway_expected_labels_reach_common_readiness_without_execution_authority() {
    let mut world = World::new("readiness-labels", &[7]);
    let source = Arc::new(AutomationFiles {
        policy: Mutex::new(
            "[[classify.rule]]\nclass = \"high\"\nlabels_any = [\"sensitive\"]\n".to_owned(),
        ),
    });
    let mut process = world
        .start_with_automation_policy(Some(Arc::new(IssueAutomationPolicyService::new(source))))
        .await;
    let text = readiness_text();
    let ordinary = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text, "expectedLabels": [] }),
        )
        .await;
    let sensitive = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text, "expectedLabels": ["sensitive"] }),
        )
        .await;

    assert_eq!(
        ordinary["result"]["readiness"]["status"], "assessed",
        "{ordinary}"
    );
    assert_eq!(
        sensitive["result"]["readiness"]["status"], "assessed",
        "{sensitive}"
    );
    assert_eq!(
        ordinary["result"]["readiness"]["assessment"]["riskClass"],
        "normal"
    );
    assert_eq!(
        sensitive["result"]["readiness"]["assessment"]["riskClass"],
        "high"
    );
    assert_eq!(
        ordinary["result"]["approval"],
        sensitive["result"]["approval"]
    );
    for answer in [&ordinary, &sensitive] {
        assert_eq!(answer["result"]["publishable"], true, "{answer}");
        let assessment = &answer["result"]["readiness"]["assessment"];
        assert_eq!(assessment["readiness"], "ready", "{answer}");
        assert_eq!(assessment["previewOnly"], true);
        assert!(assessment.get("issueId").is_none());
        assert!(assessment.get("executionGrant").is_none());
        let gates = assessment["expectedGates"]
            .as_array()
            .expect("real service readiness must serialize expected gates");
        assert!(!gates.is_empty());
        for gate in gates {
            let mut keys = gate
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            keys.sort_unstable();
            assert_eq!(keys, ["action", "actionSummary", "gate", "summary"]);
        }
    }
    assert_eq!(world.github.writes_to(7), 0);
}

#[tokio::test]
async fn gateway_automation_policy_failure_preserves_preview_approval_and_publish() {
    let mut world = World::new("readiness-policy-blocked", &[7]);
    let source = Arc::new(AutomationFiles {
        policy: Mutex::new(String::new()),
    });
    let mut process = world
        .start_with_automation_policy(Some(Arc::new(IssueAutomationPolicyService::new(
            source.clone(),
        ))))
        .await;
    let text = readiness_text();
    let opened = world.call(&mut process, "akra/issue/open", target(7)).await;
    assert!(opened["result"]["editBasis"].is_object(), "{opened}");
    let before = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text, "expectedLabels": [] }),
        )
        .await;
    assert_eq!(before["result"]["publishable"], true, "{before}");
    assert_eq!(before["result"]["readiness"]["status"], "assessed");

    const POLICY_CANARY: &str = "private-policy-canary";
    *source.policy.lock().unwrap() = format!("{POLICY_CANARY} = [");
    let after = world
        .call(
            &mut process,
            "akra/issue/preview",
            json!({ "target": target(7), "text": text, "expectedLabels": ["sensitive"] }),
        )
        .await;
    assert_eq!(after["result"]["status"], "ready", "{after}");
    assert_eq!(
        after["result"]["readiness"]["status"], "cannot_assess",
        "{after}"
    );
    assert_eq!(after["result"]["readiness"]["reason"], "policy_blocked");
    assert_eq!(after["result"]["publishable"], true);
    assert_eq!(after["result"]["approval"], before["result"]["approval"]);
    assert!(!after.to_string().contains(POLICY_CANARY));
    assert_eq!(world.github.writes_to(7), 0);

    // 이전에 검토한 게시 승인은 자동화 정책의 표시 오류와 독립적으로 유효하다.
    let published = world
        .call(
            &mut process,
            "akra/issue/publish",
            json!({
                "target": target(7),
                "text": text,
                "approval": before["result"]["approval"],
                "editBasis": opened["result"]["editBasis"],
            }),
        )
        .await;
    assert_eq!(published["result"]["status"], "confirmed", "{published}");
    assert_eq!(world.github.writes_to(7), 1);
    assert_eq!(world.github.issue(7).body, text["body"].as_str().unwrap());
    assert!(
        world
            .answers
            .iter()
            .all(|answer| !answer.contains(POLICY_CANARY))
    );
}
