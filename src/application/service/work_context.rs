use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::application::port::inbound::work_context_port::{
    AttachContextRequest, ContextResource, ContextScope, ContextScopeRequest,
    ContextSelectionEntry, ContextSelectionRequest, WorkContextPort,
};
use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
use crate::application::port::outbound::repository_identity_port::RepositoryIdentityPort;
use crate::application::port::outbound::thread_transcript_port::ThreadTranscriptPort;
use crate::application::port::outbound::work_context_store_port::{
    ResourceVersionCheck, WORK_CONTEXT_SCOPE_EXCLUDE, WORK_CONTEXT_SCOPE_PATH,
    WORK_CONTEXT_SCOPE_PIN, WORK_CONTEXT_SCOPE_REPOSITORY, WORK_CONTEXT_SCOPE_TASK,
    WORK_CONTEXT_SCOPE_THREAD, WorkContextLink, WorkContextStorePort,
};
use crate::domain::work_context::{
    ContextPreference, ExternalResourceRecognizer, ExternalResourceRef,
    external_resource_recognizers, extract_external_resources, normalize_repository_path,
};

/// 스레드 id 와 작업 id 는 장부의 키이다. app-server 의 id 는 짧은 UUID 이다.
const MAX_SCOPE_ID_CHARS: usize = 256;
/// 사람이나 AI 가 직접 이은 문서의 출처이다. 수집기가 쓰는 값과 같다.
const ATTACH_SOURCES: [&str; 3] = ["user", "agent", "tool"];
/// 문서의 버전을 다시 묻기까지의 시간이다. 그 안에는 장부의 값을 쓴다.
const VERSION_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// 한 번에 외부 서비스에 묻는 문서 수의 상한이다. 문서가 많은 스레드도 정해진 시간 안에 답한다.
const MAX_VERSION_CHECKS_PER_REFRESH: usize = 20;
/// 한 번에 이만큼 실패하면 그 확인을 멈춘다. 서비스에 닿지 않을 때 문서마다 시간 제한을 기다리지 않는다.
const MAX_VERSION_FAILURES_PER_REFRESH: usize = 3;

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/*
 * 작업 컨텍스트 수집기이다 (plan 22). 스레드의 새 기록만 읽고, 인식기로 외부 문서를 찾아 장부에 잇는다.
 * 모델을 부르지 않는다. 읽은 위치는 장부에 남겨서 같은 기록을 다시 읽지 않는다.
 *
 * 2단계: 문서가 바뀌었는지는 종류별 제공자에게 지금 버전을 물어서 안다. 스레드가 문서를 처음 확인한 버전이
 * 읽은 버전이 되고, 그 뒤의 버전이 다르면 "바뀜"이다.
 */
pub struct WorkContextService {
    transcript: Arc<dyn ThreadTranscriptPort>,
    store: Arc<dyn WorkContextStorePort>,
    recognizers: Vec<Box<dyn ExternalResourceRecognizer>>,
    version_providers: Vec<Arc<dyn ExternalResourceVersionPort>>,
    /// 스레드가 아닌 범위의 저장소를 찾는다. 없으면 그 범위를 쓸 수 없다.
    repository_identity: Option<Arc<dyn RepositoryIdentityPort>>,
    /// 버전 확인은 한 번에 하나만 한다. 같은 문서를 두 요청이 함께 묻지 않는다.
    version_check: Mutex<VersionCheckState>,
    clock: Clock,
}

#[derive(Default)]
struct VersionCheckState {
    /// 닿지 않은 문서 (종류, 키)와 그 시각이다. 이 문서도 `VERSION_CHECK_INTERVAL` 뒤에 다시 묻는다.
    /// 장부에 적지 않는다. Gateway 를 다시 시작하면 한 번 더 묻는다.
    unreachable_since: HashMap<(String, String), DateTime<Utc>>,
}

impl WorkContextService {
    pub fn new(
        transcript: Arc<dyn ThreadTranscriptPort>,
        store: Arc<dyn WorkContextStorePort>,
    ) -> Self {
        Self {
            transcript,
            store,
            recognizers: external_resource_recognizers(),
            version_providers: Vec::new(),
            repository_identity: None,
            version_check: Mutex::new(VersionCheckState::default()),
            clock: Arc::new(Utc::now),
        }
    }

    /// 문서의 버전을 알려 주는 제공자들이다. 제공자가 없는 종류의 문서는 바뀜을 확인하지 않는다.
    pub fn with_version_providers(
        mut self,
        version_providers: Vec<Arc<dyn ExternalResourceVersionPort>>,
    ) -> Self {
        self.version_providers = version_providers;
        self
    }

    pub fn with_repository_identity(
        mut self,
        repository_identity: Arc<dyn RepositoryIdentityPort>,
    ) -> Self {
        self.repository_identity = Some(repository_identity);
        self
    }

    #[cfg(test)]
    fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    fn collect(&self, thread_id: &str) -> Result<()> {
        let marker = self
            .store
            .scan_marker(WORK_CONTEXT_SCOPE_THREAD, thread_id)?;
        let page = self.transcript.text_after(thread_id, marker.as_deref())?;
        for fragment in &page.fragments {
            let resources = extract_external_resources(&fragment.text, &self.recognizers);
            if !resources.is_empty() {
                self.store.link_resources(
                    WORK_CONTEXT_SCOPE_THREAD,
                    thread_id,
                    &fragment.source,
                    &resources,
                )?;
            }
        }
        // 문서를 적은 뒤에 위치를 옮긴다. 중간에 실패하면 다음에 같은 기록을 다시 읽는다 (잇기는 여러 번 해도 같다).
        if let Some(newest) = page.newest_item_id {
            self.store
                .set_scan_marker(WORK_CONTEXT_SCOPE_THREAD, thread_id, &newest)?;
        }
        Ok(())
    }

    fn links(&self, scope: &LedgerScope) -> Result<Vec<WorkContextLink>> {
        self.store.links(scope.kind, &scope.id)
    }

    fn resources(&self, scope: &LedgerScope) -> Result<Vec<ContextResource>> {
        Ok(self
            .links(scope)?
            .into_iter()
            .map(context_resource)
            .collect())
    }

    /// 요청의 범위를 장부의 키로 바꾼다. 저장소 밖 경로와 빈 작업 id 는 받지 않는다.
    fn ledger_scope(&self, request: &ContextScopeRequest) -> Result<LedgerScope> {
        let Some(identity) = &self.repository_identity else {
            bail!("repository scopes are not available")
        };
        let repository = identity.repository_identity(&request.cwd)?;
        Ok(match &request.scope {
            ContextScope::Repository => LedgerScope {
                kind: WORK_CONTEXT_SCOPE_REPOSITORY,
                id: repository,
            },
            ContextScope::Task { task_id } => {
                let task_id = valid_scope_id(task_id)?;
                if !identity.task_exists(&request.cwd, task_id)? {
                    bail!("the task does not exist in this repository");
                }
                LedgerScope {
                    kind: WORK_CONTEXT_SCOPE_TASK,
                    id: repository_scope_id(&repository, task_id),
                }
            }
            ContextScope::Path { path } => {
                let path = normalize_repository_path(path)?;
                identity.validate_path(&request.cwd, &path)?;
                LedgerScope {
                    kind: WORK_CONTEXT_SCOPE_PATH,
                    id: repository_scope_id(&repository, &path),
                }
            }
        })
    }

    fn version_provider(&self, kind: &str) -> Option<&dyn ExternalResourceVersionPort> {
        self.version_providers
            .iter()
            .find(|provider| provider.kind() == kind)
            .map(Arc::as_ref)
    }

    /// 확인한 지 오래된 문서의 지금 버전을 제공자에게 묻고 장부에 적는다. 처음 확인한 버전은 읽은 버전이 된다.
    /// `selected_resource` 가 있으면 그 문서만 확인한다.
    fn check_versions(
        &self,
        scope: &LedgerScope,
        selected_resource: Option<&ExternalResourceRef>,
    ) -> Result<()> {
        let mut state = self
            .version_check
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = (self.clock)();
        let mut due = Vec::new();
        // 기다리는 동안 다른 요청이 갱신했을 수 있으므로 잠금을 잡은 뒤 최신 연결을 읽는다.
        for link in self.links(scope)? {
            if selected_resource.is_some_and(|resource| {
                link.resource.kind != resource.kind || link.resource.key != resource.key
            }) {
                continue;
            }
            let Some(provider) = self.version_provider(&link.resource.kind) else {
                continue;
            };
            if let Some(check) = fresh_check(link.checked_version.as_ref(), now) {
                // 다른 범위가 조금 전에 확인한 문서이다. 그 버전이 이 범위가 처음 확인한 버전이다.
                if link.read_version.is_none() {
                    self.set_first_read_version(scope, &link.resource, &check.version)?;
                }
                continue;
            }
            if !provider.available(&link.resource) {
                continue;
            }
            let document = (link.resource.kind.clone(), link.resource.key.clone());
            let failed_at = state.unreachable_since.get(&document).copied();
            if failed_at.is_some_and(|failed_at| !interval_has_passed(failed_at, now)) {
                continue;
            }
            due.push((failed_at.is_some(), provider, link, document));
        }
        // 전에 닿지 않은 문서는 뒤에 묻는다. 계속 실패하는 문서가 다른 문서의 차례를 막지 않는다.
        due.sort_by_key(|(failed_before, ..)| *failed_before);
        let mut failures = 0;
        let candidates = due.into_iter().take(MAX_VERSION_CHECKS_PER_REFRESH);
        for (_, provider, link, document) in candidates {
            match provider.current_version(&link.resource) {
                Ok(version) => {
                    state.unreachable_since.remove(&document);
                    self.store.record_resource_version(
                        &link.resource,
                        &ResourceVersionCheck {
                            version: version.clone(),
                            checked_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
                        },
                    )?;
                    self.set_first_read_version(scope, &link.resource, &version)?;
                }
                // 닿지 않는 문서는 건너뛴다. 장부의 이전 값이 남는다.
                Err(_) => {
                    state.unreachable_since.insert(document, now);
                    failures += 1;
                    if failures == MAX_VERSION_FAILURES_PER_REFRESH {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// 범위에 읽은 버전이 없을 때만 적는다. 있는 값은 사용자가 문서를 열 때만 바뀐다.
    fn set_first_read_version(
        &self,
        scope: &LedgerScope,
        resource: &ExternalResourceRef,
        version: &str,
    ) -> Result<()> {
        self.store
            .set_read_version(scope.kind, &scope.id, resource, version, false)
    }
}

/// 장부의 범위 키이다.
struct LedgerScope {
    kind: &'static str,
    id: String,
}

impl LedgerScope {
    fn thread(thread_id: &str) -> Result<Self> {
        Ok(Self {
            kind: WORK_CONTEXT_SCOPE_THREAD,
            id: valid_scope_id(thread_id)?.to_string(),
        })
    }
}

/// 저장소 안 범위의 키이다. JSON 배열이라 저장소 식별자나 값에 어떤 글자가 있어도 둘이 섞이지 않는다.
fn repository_scope_id(repository: &str, value: &str) -> String {
    serde_json::json!([repository, value]).to_string()
}

fn interval_has_passed(since: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(since)
        .to_std()
        .is_ok_and(|elapsed| elapsed >= VERSION_CHECK_INTERVAL)
}

/// 확인한 지 `VERSION_CHECK_INTERVAL` 이 지나지 않은 확인이다. 읽지 못하는 시각은 지난 것으로 본다.
fn fresh_check(
    check: Option<&ResourceVersionCheck>,
    now: DateTime<Utc>,
) -> Option<&ResourceVersionCheck> {
    check.filter(|check| {
        DateTime::parse_from_rfc3339(&check.checked_at)
            .is_ok_and(|checked_at| !interval_has_passed(checked_at.with_timezone(&Utc), now))
    })
}

fn context_resource(link: WorkContextLink) -> ContextResource {
    let changed = match (&link.checked_version, &link.read_version) {
        (Some(check), Some(read)) => &check.version != read,
        _ => false,
    };
    ContextResource {
        title: link
            .resource
            .title_hint
            .unwrap_or_else(|| link.resource.url.clone()),
        kind: link.resource.kind,
        key: link.resource.key,
        url: link.resource.url,
        source: link.source,
        first_seen_at: link.first_seen_at,
        changed,
        current_version: link
            .checked_version
            .as_ref()
            .map(|check| check.version.clone()),
        version_checked_at: link.checked_version.map(|check| check.checked_at),
        confirmed_version: link.read_version,
    }
}

fn valid_scope_id(scope_id: &str) -> Result<&str> {
    let scope_id = scope_id.trim();
    if scope_id.is_empty()
        || scope_id.chars().count() > MAX_SCOPE_ID_CHARS
        || scope_id.chars().any(char::is_control)
    {
        bail!("the scope id is not valid")
    }
    Ok(scope_id)
}

/// 주소에서 문서 하나를 찾는다. 인식기가 모르는 주소나 문서가 여럿인 글은 받지 않는다.
fn attached_resource(
    url: &str,
    recognizers: &[Box<dyn ExternalResourceRecognizer>],
) -> Result<ExternalResourceRef> {
    let mut resources = extract_external_resources(url, recognizers);
    if resources.len() != 1 {
        bail!("the url must name exactly one known document")
    }
    Ok(resources.remove(0))
}

impl WorkContextPort for WorkContextService {
    fn select_resources(
        &self,
        request: &ContextSelectionRequest,
    ) -> Result<Vec<ContextSelectionEntry>> {
        let identity = self
            .repository_identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("repository scopes are not available"))?;
        let repository = identity.repository_identity(&request.cwd)?;
        if request.paths.len() > 32 {
            bail!("too many context paths");
        }
        let task = request.task_id.as_deref().map(valid_scope_id).transpose()?;
        if let Some(task) = task
            && !identity.task_exists(&request.cwd, task)?
        {
            bail!("the task does not exist in this repository");
        }
        // Match complete ancestors, never a raw string prefix. A rename is an explicit
        // new path; linked worktrees share the repository ledger.
        let mut prefixes = BTreeSet::new();
        for path in &request.paths {
            let path = normalize_repository_path(path)?;
            identity.validate_path(&request.cwd, &path)?;
            let mut prefix = path.as_str();
            loop {
                prefixes.insert(prefix.to_owned());
                if prefixes.len() > 256 {
                    bail!("too many context path scopes");
                }
                let Some((parent, _)) = prefix.rsplit_once('/') else {
                    break;
                };
                prefix = parent;
            }
        }
        let focused = task.is_some() || !prefixes.is_empty();
        let _guard = self
            .version_check
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut selected = BTreeMap::new();
        let mut add_scope =
            |kind, id: String, matched_scope: ContextScope, include, reason| -> Result<()> {
                for link in self.store.links(kind, &id)? {
                    let key = (link.resource.kind.clone(), link.resource.key.clone());
                    selected
                        .entry(key)
                        .or_insert_with(|| ContextSelectionEntry {
                            resource: context_resource(link),
                            selected: include,
                            reason,
                            matched_scope: Some(matched_scope.clone()),
                        });
                }
                Ok(())
            };
        // Task matches beat path matches, and the longest path wins among path matches.
        if let Some(task) = task {
            add_scope(
                WORK_CONTEXT_SCOPE_TASK,
                repository_scope_id(&repository, task),
                ContextScope::Task {
                    task_id: task.to_owned(),
                },
                true,
                "task_link",
            )?;
        }
        let mut prefixes: Vec<_> = prefixes.into_iter().collect();
        prefixes.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        for path in prefixes {
            add_scope(
                WORK_CONTEXT_SCOPE_PATH,
                repository_scope_id(&repository, &path),
                ContextScope::Path { path },
                true,
                "path_prefix",
            )?;
        }
        add_scope(
            WORK_CONTEXT_SCOPE_REPOSITORY,
            repository.clone(),
            ContextScope::Repository,
            !focused,
            if focused {
                "repository_only"
            } else {
                "repository_link"
            },
        )?;
        // The override applies even without another matching link. Exclude is fail-closed.
        for (kind, include, reason) in [
            (WORK_CONTEXT_SCOPE_PIN, true, "user_pin"),
            (WORK_CONTEXT_SCOPE_EXCLUDE, false, "user_exclude"),
        ] {
            for link in self.store.links(kind, &repository)? {
                let key = (link.resource.kind.clone(), link.resource.key.clone());
                let entry = selected
                    .entry(key)
                    .or_insert_with(|| ContextSelectionEntry {
                        resource: context_resource(link),
                        selected: include,
                        reason,
                        matched_scope: None,
                    });
                entry.selected = include;
                entry.reason = reason;
            }
        }
        let mut entries: Vec<_> = selected.into_values().collect();
        entries.sort_by_key(|entry| {
            if entry.reason == "user_pin" {
                0
            } else if entry.selected {
                1
            } else {
                2
            }
        });
        Ok(entries)
    }

    fn set_repository_preference(
        &self,
        cwd: &str,
        url: &str,
        preference: ContextPreference,
    ) -> Result<()> {
        let identity = self
            .repository_identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("repository scopes are not available"))?;
        let repository = identity.repository_identity(cwd)?;
        let resource = attached_resource(url, &self.recognizers)?;
        {
            let _guard = self
                .version_check
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.store
                .set_repository_preference(&repository, &resource, preference)?;
        }
        if preference == ContextPreference::Pin {
            self.check_versions(
                &LedgerScope {
                    kind: WORK_CONTEXT_SCOPE_REPOSITORY,
                    id: repository,
                },
                Some(&resource),
            )?;
        }
        Ok(())
    }

    fn thread_resources(&self, thread_id: &str) -> Result<Vec<ContextResource>> {
        let scope = LedgerScope::thread(thread_id)?;
        self.collect(&scope.id)?;
        self.resources(&scope)
    }

    fn refresh_thread_resources(&self, thread_id: &str) -> Result<Vec<ContextResource>> {
        let scope = LedgerScope::thread(thread_id)?;
        self.collect(&scope.id)?;
        self.check_versions(&scope, None)?;
        self.resources(&scope)
    }

    fn mark_thread_resource_read(
        &self,
        thread_id: &str,
        resource_kind: &str,
        resource_key: &str,
    ) -> Result<Vec<ContextResource>> {
        let scope = LedgerScope::thread(thread_id)?;
        let link = self
            .links(&scope)?
            .into_iter()
            .find(|link| link.resource.kind == resource_kind && link.resource.key == resource_key);
        let Some(link) = link else {
            bail!("the thread has no such document")
        };
        // 확인한 버전이 없으면 바뀜도 없다. 적을 것이 없다.
        if let Some(check) = &link.checked_version {
            self.store.set_read_version(
                scope.kind,
                &scope.id,
                &link.resource,
                &check.version,
                true,
            )?;
        }
        self.resources(&scope)
    }

    fn attach_scope_resource(
        &self,
        request: &AttachContextRequest,
    ) -> Result<Vec<ContextResource>> {
        if !ATTACH_SOURCES.contains(&request.source.as_str()) {
            bail!("the source must be one of {}", ATTACH_SOURCES.join(", "))
        }
        let resource = attached_resource(&request.url, &self.recognizers)?;
        let scope = self.ledger_scope(&request.scope)?;
        self.store.link_resources(
            scope.kind,
            &scope.id,
            &request.source,
            std::slice::from_ref(&resource),
        )?;
        // 이은 문서만 확인한다. 범위에 오래된 문서가 많아도 새 문서가 확인 상한 밖으로 밀리지 않는다.
        self.check_versions(&scope, Some(&resource))?;
        self.resources(&scope)
    }

    fn scope_resources(&self, request: &ContextScopeRequest) -> Result<Vec<ContextResource>> {
        let scope = self.ledger_scope(request)?;
        self.resources(&scope)
    }

    fn refresh_scope_resources(
        &self,
        request: &ContextScopeRequest,
    ) -> Result<Vec<ContextResource>> {
        let scope = self.ledger_scope(request)?;
        self.check_versions(&scope, None)?;
        self.resources(&scope)
    }

    fn mark_scope_resource_read(
        &self,
        request: &ContextScopeRequest,
        resource_kind: &str,
        resource_key: &str,
        observed_version: &str,
    ) -> Result<Vec<ContextResource>> {
        let scope = self.ledger_scope(request)?;
        // 버전 확인과 읽음 기록을 같은 서비스 잠금으로 직렬화한다.
        let _version_check = self
            .version_check
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(link) = self
            .links(&scope)?
            .into_iter()
            .find(|link| link.resource.kind == resource_kind && link.resource.key == resource_key)
        else {
            bail!("the scope has no such document");
        };
        if observed_version.is_empty()
            || link
                .checked_version
                .as_ref()
                .is_none_or(|check| check.version != observed_version)
        {
            bail!(
                "the observed document version is no longer current; reload before marking it read"
            );
        }
        self.store.set_read_version(
            scope.kind,
            &scope.id,
            &link.resource,
            observed_version,
            true,
        )?;
        self.resources(&scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::thread_transcript_port::{
        ThreadTextFragment, ThreadTextPage,
    };
    use crate::application::port::outbound::work_context_store_port::WorkContextLink;
    use std::sync::mpsc;

    #[derive(Default)]
    struct FakeTranscript {
        pages: Mutex<Vec<ThreadTextPage>>,
        requests: Mutex<Vec<Option<String>>>,
    }

    impl ThreadTranscriptPort for FakeTranscript {
        fn text_after(&self, _thread_id: &str, after: Option<&str>) -> Result<ThreadTextPage> {
            self.requests
                .lock()
                .unwrap()
                .push(after.map(str::to_string));
            let mut pages = self.pages.lock().unwrap();
            Ok(if pages.is_empty() {
                ThreadTextPage::default()
            } else {
                pages.remove(0)
            })
        }
    }

    #[derive(Default)]
    struct FakeStore {
        links: Mutex<Vec<(String, WorkContextLink)>>,
        markers: Mutex<HashMap<String, String>>,
        link_reads: Mutex<Option<mpsc::Sender<()>>>,
    }

    fn same_document(link: &WorkContextLink, resource: &ExternalResourceRef) -> bool {
        link.resource.kind == resource.kind && link.resource.key == resource.key
    }

    impl WorkContextStorePort for FakeStore {
        fn set_repository_preference(
            &self,
            repository: &str,
            resource: &ExternalResourceRef,
            preference: ContextPreference,
        ) -> Result<()> {
            let target =
                crate::application::port::outbound::work_context_store_port::preference_scope_kind(
                    preference,
                );
            for kind in [WORK_CONTEXT_SCOPE_PIN, WORK_CONTEXT_SCOPE_EXCLUDE] {
                if Some(kind) != target {
                    let id = format!("{kind}\0{repository}");
                    self.links
                        .lock()
                        .unwrap()
                        .retain(|(scope, link)| scope != &id || !same_document(link, resource));
                }
            }
            if let Some(kind) = target {
                self.link_resources(kind, repository, "user", std::slice::from_ref(resource))?;
            }
            if preference == ContextPreference::Pin {
                self.link_resources(
                    WORK_CONTEXT_SCOPE_REPOSITORY,
                    repository,
                    "user",
                    std::slice::from_ref(resource),
                )?;
            }
            Ok(())
        }

        fn link_resources(
            &self,
            scope_kind: &str,
            scope_id: &str,
            source: &str,
            resources: &[ExternalResourceRef],
        ) -> Result<()> {
            let scope_id = format!("{scope_kind}\0{scope_id}");
            let mut links = self.links.lock().unwrap();
            for resource in resources {
                let known = links.iter().any(|(scope, link)| {
                    scope == &scope_id
                        && link.resource.kind == resource.kind
                        && link.resource.key == resource.key
                });
                if !known {
                    // 확인한 버전은 문서의 값이다. 다른 범위가 적은 값이 새 연결에도 보인다.
                    let checked_version = links
                        .iter()
                        .find(|(_, link)| same_document(link, resource))
                        .and_then(|(_, link)| link.checked_version.clone());
                    links.push((
                        scope_id.to_string(),
                        WorkContextLink {
                            resource: resource.clone(),
                            source: source.to_string(),
                            first_seen_at: "2026-11-11T00:00:00Z".to_string(),
                            checked_version,
                            read_version: None,
                        },
                    ));
                }
            }
            Ok(())
        }
        fn links(&self, scope_kind: &str, scope_id: &str) -> Result<Vec<WorkContextLink>> {
            let scope_id = format!("{scope_kind}\0{scope_id}");
            let links = self
                .links
                .lock()
                .unwrap()
                .iter()
                .filter(|(scope, _)| scope == &scope_id)
                .map(|(_, link)| link.clone())
                .collect();
            if let Some(observer) = self.link_reads.lock().unwrap().as_ref() {
                let _ = observer.send(());
            }
            Ok(links)
        }
        fn scan_marker(&self, _scope_kind: &str, scope_id: &str) -> Result<Option<String>> {
            Ok(self.markers.lock().unwrap().get(scope_id).cloned())
        }
        fn set_scan_marker(&self, _scope_kind: &str, scope_id: &str, marker: &str) -> Result<()> {
            self.markers
                .lock()
                .unwrap()
                .insert(scope_id.to_string(), marker.to_string());
            Ok(())
        }
        fn record_resource_version(
            &self,
            resource: &ExternalResourceRef,
            check: &ResourceVersionCheck,
        ) -> Result<()> {
            for (_, link) in self.links.lock().unwrap().iter_mut() {
                if same_document(link, resource) {
                    link.checked_version = Some(check.clone());
                }
            }
            Ok(())
        }
        fn set_read_version(
            &self,
            scope_kind: &str,
            scope_id: &str,
            resource: &ExternalResourceRef,
            version: &str,
            replace: bool,
        ) -> Result<()> {
            let scope_id = format!("{scope_kind}\0{scope_id}");
            for (scope, link) in self.links.lock().unwrap().iter_mut() {
                if scope == &scope_id
                    && same_document(link, resource)
                    && (replace || link.read_version.is_none())
                {
                    link.read_version = Some(version.to_string());
                }
            }
            Ok(())
        }
    }

    struct VersionCheckGate {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    /// 문서 키마다 지금 버전을 답하는 가짜 GitHub 이다. 버전이 없는 키는 닿지 않는 문서이다.
    #[derive(Default)]
    struct FakeVersions {
        versions: Mutex<HashMap<String, String>>,
        asked: Mutex<Vec<String>>,
        blocked: Mutex<std::collections::HashSet<String>>,
        first_check: Mutex<Option<VersionCheckGate>>,
    }

    impl FakeVersions {
        fn set(&self, key: &str, version: &str) {
            self.versions
                .lock()
                .unwrap()
                .insert(key.to_string(), version.to_string());
        }
    }

    impl ExternalResourceVersionPort for FakeVersions {
        fn kind(&self) -> &'static str {
            "github"
        }
        fn available(&self, resource: &ExternalResourceRef) -> bool {
            !self.blocked.lock().unwrap().contains(&resource.key)
        }
        fn current_version(&self, resource: &ExternalResourceRef) -> Result<String> {
            let key = &resource.key;
            self.asked.lock().unwrap().push(key.to_string());
            let first_check = self.first_check.lock().unwrap().take();
            if let Some(gate) = first_check {
                gate.started.send(()).unwrap();
                gate.release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.versions
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("the document cannot be read"))
        }
    }

    /// 시험이 움직이는 시계이다.
    fn test_clock(start: &str) -> (Clock, Arc<Mutex<DateTime<Utc>>>) {
        let now = Arc::new(Mutex::new(
            DateTime::parse_from_rfc3339(start)
                .unwrap()
                .with_timezone(&Utc),
        ));
        let shared = now.clone();
        (Arc::new(move || *shared.lock().unwrap()), now)
    }

    fn versioned_service(
        text: &str,
    ) -> (
        WorkContextService,
        Arc<FakeVersions>,
        Arc<Mutex<DateTime<Utc>>>,
    ) {
        let transcript = Arc::new(FakeTranscript::default());
        transcript.pages.lock().unwrap().push(ThreadTextPage {
            fragments: vec![fragment("user", text)],
            newest_item_id: Some("item-1".into()),
        });
        let versions = Arc::new(FakeVersions::default());
        let (clock, now) = test_clock("2026-11-11T00:00:00Z");
        let service = WorkContextService::new(transcript, Arc::new(FakeStore::default()))
            .with_version_providers(vec![versions.clone()])
            .with_clock(clock);
        (service, versions, now)
    }

    fn changed_flags(resources: &[ContextResource]) -> Vec<(&str, bool)> {
        resources
            .iter()
            .map(|resource| (resource.key.as_str(), resource.changed))
            .collect()
    }

    #[test]
    fn linear_documents_skip_unavailable_keys_and_show_changed_then_read() {
        struct LinearVersions(Arc<FakeVersions>);
        impl ExternalResourceVersionPort for LinearVersions {
            fn kind(&self) -> &'static str {
                "linear"
            }
            fn available(&self, resource: &ExternalResourceRef) -> bool {
                self.0.available(resource)
            }
            fn current_version(&self, resource: &ExternalResourceRef) -> Result<String> {
                self.0.current_version(resource)
            }
        }
        let transcript = Arc::new(FakeTranscript::default());
        transcript.pages.lock().unwrap().push(ThreadTextPage {
            fragments: vec![fragment("user", "https://linear.app/team/issue/APP-1 https://linear.app/team/issue/APP-2 https://linear.app/team/issue/APP-3 https://linear.app/team/issue/APP-4 https://linear.app/team/issue/APP-42")],
            newest_item_id: Some("item-1".into()),
        });
        let versions = Arc::new(FakeVersions::default());
        for n in 1..=4 {
            versions
                .blocked
                .lock()
                .unwrap()
                .insert(format!("team/APP-{n}"));
        }
        versions.set("team/APP-42", "v1");
        let (clock, now) = test_clock("2026-11-11T00:00:00Z");
        let service = WorkContextService::new(transcript, Arc::new(FakeStore::default()))
            .with_version_providers(vec![Arc::new(LinearVersions(versions.clone()))])
            .with_clock(clock);
        let first = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(first.len(), 5);
        assert!(first.iter().all(|document| !document.changed));
        assert_eq!(*versions.asked.lock().unwrap(), vec!["team/APP-42"]);
        versions.set("team/APP-42", "v2");
        *now.lock().unwrap() += chrono::Duration::minutes(6);
        let later = service.refresh_thread_resources("thread-1").unwrap();
        assert!(
            later
                .iter()
                .find(|document| document.key == "team/APP-42")
                .unwrap()
                .changed
        );
        let read = service
            .mark_thread_resource_read("thread-1", "linear", "team/APP-42")
            .unwrap();
        assert!(read.iter().all(|document| !document.changed));
        assert_eq!(versions.asked.lock().unwrap().len(), 2);
    }

    #[test]
    fn concurrent_refresh_and_attach_reuse_the_completed_version_check() {
        for attach_second in [false, true] {
            let transcript = Arc::new(FakeTranscript::default());
            transcript.pages.lock().unwrap().push(ThreadTextPage {
                fragments: vec![fragment("user", "https://github.com/a/b/pull/2")],
                newest_item_id: Some("item-1".into()),
            });
            let store = Arc::new(FakeStore::default());
            let versions = Arc::new(FakeVersions::default());
            versions.set("a/b#2", "v1");
            let (started, first_started) = mpsc::channel();
            let (release, first_release) = mpsc::channel();
            *versions.first_check.lock().unwrap() = Some(VersionCheckGate {
                started,
                release: first_release,
            });
            let (clock, _) = test_clock("2026-11-11T00:00:00Z");
            let service = WorkContextService::new(transcript, store.clone())
                .with_version_providers(vec![versions.clone()])
                .with_repository_identity(Arc::new(FakeRepositories))
                .with_clock(clock);
            service.thread_resources("thread-1").unwrap();
            let (reads, observed_read) = mpsc::channel();
            *store.link_reads.lock().unwrap() = Some(reads);

            std::thread::scope(|threads| {
                let first = threads.spawn(|| service.refresh_thread_resources("thread-1"));
                first_started.recv_timeout(Duration::from_secs(5)).unwrap();
                observed_read.recv_timeout(Duration::from_secs(5)).unwrap();
                let (entered, second_entered) = mpsc::channel();
                let service = &service;
                let second = threads.spawn(move || {
                    entered.send(()).unwrap();
                    if attach_second {
                        service.attach_scope_resource(&attach(
                            scope("/one", ContextScope::Repository),
                            "https://github.com/a/b/pull/2",
                        ))
                    } else {
                        service.refresh_thread_resources("thread-1")
                    }
                });
                second_entered.recv_timeout(Duration::from_secs(5)).unwrap();
                // Before the fix the second request reads stale rows while the provider
                // is held here. With serialization it cannot read until the first finishes.
                // The deadline releases that correctly blocked request without a sleep.
                let _ = observed_read.recv_timeout(Duration::from_secs(1));
                release.send(()).unwrap();
                let first = first.join().unwrap().unwrap();
                let second = second.join().unwrap().unwrap();
                assert_eq!(first[0].current_version.as_deref(), Some("v1"));
                assert_eq!(second[0].confirmed_version.as_deref(), Some("v1"));
            });
            assert_eq!(
                *versions.asked.lock().unwrap(),
                ["a/b#2"],
                "the waiting request must reuse the first check (attach={attach_second})"
            );
        }
    }

    #[test]
    fn the_first_check_is_the_read_version_and_a_later_version_marks_the_document_changed() {
        // plan 22 stage 2: no model call. The provider gives the current version, the ledger keeps what the thread read.
        let (service, versions, now) = versioned_service(
            "see https://github.com/a/b/pull/2 and https://github.com/a/b/issues/9",
        );
        versions.set("a/b#2", "v1");
        versions.set("a/b#9", "v1");

        let listed = service.thread_resources("thread-1").unwrap();
        assert_eq!(changed_flags(&listed), [("a/b#2", false), ("a/b#9", false)]);
        assert!(
            versions.asked.lock().unwrap().is_empty(),
            "listing asks no service"
        );
        assert_eq!(listed[0].version_checked_at, None);

        let first = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(changed_flags(&first), [("a/b#2", false), ("a/b#9", false)]);
        assert_eq!(
            first[0].version_checked_at.as_deref(),
            Some("2026-11-11T00:00:00Z")
        );

        // 확인한 지 얼마 안 된 문서는 다시 묻지 않는다.
        versions.set("a/b#2", "v2");
        *now.lock().unwrap() += chrono::Duration::seconds(60);
        let soon = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(changed_flags(&soon), [("a/b#2", false), ("a/b#9", false)]);
        assert_eq!(versions.asked.lock().unwrap().len(), 2);

        *now.lock().unwrap() += chrono::Duration::seconds(5 * 60);
        let later = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(changed_flags(&later), [("a/b#2", true), ("a/b#9", false)]);
        // 목록 조회도 장부의 값으로 바뀜을 보인다.
        assert_eq!(
            changed_flags(&service.thread_resources("thread-1").unwrap()),
            [("a/b#2", true), ("a/b#9", false)]
        );

        let read = service
            .mark_thread_resource_read("thread-1", "github", "a/b#2")
            .unwrap();
        assert_eq!(changed_flags(&read), [("a/b#2", false), ("a/b#9", false)]);
        assert!(
            service
                .mark_thread_resource_read("thread-1", "github", "a/b#404")
                .is_err()
        );
    }

    #[test]
    fn unreachable_documents_and_kinds_without_a_provider_are_left_alone() {
        let (service, versions, now) = versioned_service(
            "https://github.com/a/b/pull/2 https://github.com/a/b/pull/3 https://acme.atlassian.net/browse/ABC-1",
        );
        versions.set("a/b#2", "v1");

        let first = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            changed_flags(&first),
            [
                ("a/b#2", false),
                ("a/b#3", false),
                ("acme.atlassian.net/ABC-1", false)
            ]
        );
        assert_eq!(
            first[1].version_checked_at, None,
            "the document was not reached"
        );
        assert_eq!(
            first[2].version_checked_at, None,
            "no provider knows Jira yet"
        );
        assert_eq!(*versions.asked.lock().unwrap(), ["a/b#2", "a/b#3"]);

        // 닿지 않던 문서도 간격을 기다린다. 처음 닿은 버전이 기준이 된다.
        versions.set("a/b#3", "v7");
        *now.lock().unwrap() += chrono::Duration::seconds(10);
        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(*versions.asked.lock().unwrap(), ["a/b#2", "a/b#3"]);

        *now.lock().unwrap() += chrono::Duration::seconds(5 * 60);
        let later = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(changed_flags(&later)[1], ("a/b#3", false));
        assert_eq!(
            later[1].version_checked_at.as_deref(),
            Some("2026-11-11T00:05:10Z")
        );
        assert_eq!(
            *versions.asked.lock().unwrap(),
            ["a/b#2", "a/b#3", "a/b#2", "a/b#3"]
        );
    }

    #[test]
    fn a_refresh_stops_after_a_few_failures_and_later_documents_get_their_turn() {
        // When the service is down, every document would wait for its timeout. Stop early.
        let (service, versions, _now) = versioned_service(
            "https://github.com/a/b/issues/1 https://github.com/a/b/issues/2 \
             https://github.com/a/b/issues/3 https://github.com/a/b/issues/4 \
             https://github.com/a/b/issues/5",
        );
        versions.set("a/b#5", "v1");

        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(*versions.asked.lock().unwrap(), ["a/b#1", "a/b#2", "a/b#3"]);

        // 실패한 문서는 기다린다. 뒤의 문서가 막히지 않는다.
        let second = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            *versions.asked.lock().unwrap(),
            ["a/b#1", "a/b#2", "a/b#3", "a/b#4", "a/b#5"]
        );
        assert!(second[4].version_checked_at.is_some());
    }

    #[test]
    fn documents_that_always_fail_do_not_take_the_turn_of_the_others() {
        // Three documents always fail (no access). The fourth must still get its checks.
        let (service, versions, now) = versioned_service(
            "https://github.com/a/b/issues/1 https://github.com/a/b/issues/2 \
             https://github.com/a/b/issues/3 https://github.com/a/b/issues/4",
        );
        versions.set("a/b#4", "v1");
        service.refresh_thread_resources("thread-1").unwrap();
        service.refresh_thread_resources("thread-1").unwrap();
        versions.asked.lock().unwrap().clear();

        versions.set("a/b#4", "v2");
        *now.lock().unwrap() += chrono::Duration::seconds(6 * 60);
        let later = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            *versions.asked.lock().unwrap(),
            ["a/b#4", "a/b#1", "a/b#2", "a/b#3"]
        );
        assert_eq!(changed_flags(&later)[3], ("a/b#4", true));
    }

    #[test]
    fn a_second_thread_takes_a_recent_check_as_its_read_version() {
        // The check time belongs to the document. Thread 2 must not lose its baseline because thread 1 asked first.
        let transcript = Arc::new(FakeTranscript::default());
        for _ in 0..2 {
            transcript.pages.lock().unwrap().push(ThreadTextPage {
                fragments: vec![fragment("user", "https://github.com/a/b/pull/2")],
                newest_item_id: Some("item-1".into()),
            });
        }
        let versions = Arc::new(FakeVersions::default());
        versions.set("a/b#2", "v1");
        let (clock, now) = test_clock("2026-11-11T00:00:00Z");
        let service = WorkContextService::new(transcript, Arc::new(FakeStore::default()))
            .with_version_providers(vec![versions.clone()])
            .with_clock(clock);

        service.refresh_thread_resources("thread-1").unwrap();
        versions.blocked.lock().unwrap().insert("a/b#2".into());
        *now.lock().unwrap() += chrono::Duration::seconds(60);
        service.refresh_thread_resources("thread-2").unwrap();
        assert_eq!(
            versions.asked.lock().unwrap().len(),
            1,
            "the check is recent"
        );

        versions.blocked.lock().unwrap().clear();
        versions.set("a/b#2", "v2");
        *now.lock().unwrap() += chrono::Duration::seconds(10 * 60);
        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            changed_flags(&service.thread_resources("thread-2").unwrap()),
            [("a/b#2", true)]
        );

        // 한 스레드가 읽어도 다른 스레드의 표시는 남는다.
        service
            .mark_thread_resource_read("thread-1", "github", "a/b#2")
            .unwrap();
        assert_eq!(
            changed_flags(&service.thread_resources("thread-1").unwrap()),
            [("a/b#2", false)]
        );
        assert_eq!(
            changed_flags(&service.thread_resources("thread-2").unwrap()),
            [("a/b#2", true)]
        );
    }

    #[test]
    fn an_unavailable_provider_does_not_spend_failures_or_create_a_cooldown() {
        let (service, versions, _now) =
            versioned_service("https://github.com/a/b/issues/1 https://github.com/a/b/issues/2");
        versions.blocked.lock().unwrap().insert("a/b#1".into());
        versions.set("a/b#1", "v1");
        versions.set("a/b#2", "v1");
        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(*versions.asked.lock().unwrap(), ["a/b#2"]);
        versions.blocked.lock().unwrap().clear();
        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(*versions.asked.lock().unwrap(), ["a/b#2", "a/b#1"]);
    }

    #[test]
    fn one_refresh_asks_a_bounded_number_of_documents() {
        let text = (1..=MAX_VERSION_CHECKS_PER_REFRESH + 5)
            .map(|number| format!("https://github.com/a/b/issues/{number}"))
            .collect::<Vec<_>>()
            .join(" ");
        let (service, versions, _now) = versioned_service(&text);
        for number in 1..=MAX_VERSION_CHECKS_PER_REFRESH + 5 {
            versions.set(&format!("a/b#{number}"), "v1");
        }

        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            versions.asked.lock().unwrap().len(),
            MAX_VERSION_CHECKS_PER_REFRESH
        );
        // 남은 문서는 다음 확인에서 묻는다. 확인한 문서는 간격을 기다린다.
        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            versions.asked.lock().unwrap().len(),
            MAX_VERSION_CHECKS_PER_REFRESH + 5
        );
    }

    #[test]
    fn notion_fetch_versions_drive_changed_and_read_state_without_losing_failed_checks() {
        use crate::adapter::outbound::mcp_hub::NotionResourceVersionAdapter;
        use crate::application::port::outbound::mcp_hub_backend_port::McpHubBackendPort;
        use crate::domain::mcp_hub::{McpHubServerSpec, McpHubServerStatus, McpHubToolInfo};
        use serde_json::{Value, json};

        struct Hub(Mutex<(bool, Value)>);
        impl McpHubBackendPort for Hub {
            fn registered_servers(&self) -> Result<Vec<McpHubServerSpec>> {
                if !self.0.lock().unwrap().0 {
                    return Ok(Vec::new());
                }
                Ok(vec![
                    McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").unwrap(),
                ])
            }
            fn register_server(&self, _: &McpHubServerSpec) -> Result<()> {
                unreachable!()
            }
            fn unregister_server(&self, _: &str) -> Result<bool> {
                unreachable!()
            }
            fn login(&self, _: &str, _: &mut dyn FnMut(&str)) -> Result<()> {
                unreachable!()
            }
            fn server_statuses(&self, only: Option<&str>) -> Result<Vec<McpHubServerStatus>> {
                assert_eq!(only, Some("notion"));
                Ok(vec![McpHubServerStatus {
                    name: "notion".into(),
                    auth: "oAuth".into(),
                    connection: "connected".into(),
                    error: None,
                    tools: vec![McpHubToolInfo {
                        name: "notion-fetch".into(),
                        description: String::new(),
                        input_schema: json!({}),
                    }],
                }])
            }
            fn call_tool(&self, server: &str, tool: &str, arguments: Value) -> Result<Value> {
                assert_eq!((server, tool), ("notion", "notion-fetch"));
                assert_eq!(arguments, json!({"id":"0123456789abcdef0123456789abcdef"}));
                Ok(self.0.lock().unwrap().1.clone())
            }
        }
        let payload = |timestamp| json!({"structuredContent":{"page_last_edited_at":timestamp}});
        let hub = Arc::new(Hub(Mutex::new((false, payload("2026-11-17T00:00:00Z")))));
        let transcript = Arc::new(FakeTranscript::default());
        transcript.pages.lock().unwrap().push(ThreadTextPage {
            fragments: vec![fragment(
                "user",
                "https://notion.so/0123456789abcdef0123456789abcdef",
            )],
            newest_item_id: Some("item-1".into()),
        });
        let (clock, now) = test_clock("2026-11-17T01:00:00Z");
        let service = WorkContextService::new(transcript, Arc::new(FakeStore::default()))
            .with_version_providers(vec![Arc::new(NotionResourceVersionAdapter::new(
                hub.clone(),
            ))])
            .with_clock(clock);
        assert!(
            service.refresh_thread_resources("thread-1").unwrap()[0]
                .version_checked_at
                .is_none()
        );
        hub.0.lock().unwrap().0 = true;
        let first = service.refresh_thread_resources("thread-1").unwrap();
        assert!(!first[0].changed);
        assert!(first[0].version_checked_at.is_some());
        hub.0.lock().unwrap().1 = payload("2026-11-17T00:01:00Z");
        *now.lock().unwrap() += chrono::Duration::minutes(6);
        assert!(service.refresh_thread_resources("thread-1").unwrap()[0].changed);
        hub.0.lock().unwrap().1 = json!({"isError":true});
        *now.lock().unwrap() += chrono::Duration::minutes(6);
        assert!(service.refresh_thread_resources("thread-1").unwrap()[0].changed);
        assert!(
            !service
                .mark_thread_resource_read("thread-1", "notion", "0123456789abcdef0123456789abcdef")
                .unwrap()[0]
                .changed
        );
    }

    fn fragment(source: &str, text: &str) -> ThreadTextFragment {
        ThreadTextFragment {
            source: source.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn collects_new_text_once_and_keeps_the_first_source() {
        let transcript = Arc::new(FakeTranscript::default());
        transcript.pages.lock().unwrap().extend([
            ThreadTextPage {
                fragments: vec![
                    fragment("user", "fix https://acme.atlassian.net/browse/ABC-1"),
                    fragment("agent", "reading https://acme.atlassian.net/browse/ABC-1 and https://github.com/a/b/pull/2"),
                ],
                newest_item_id: Some("item-2".into()),
            },
            ThreadTextPage {
                fragments: vec![fragment("tool", "no links here")],
                newest_item_id: Some("item-3".into()),
            },
        ]);
        let service = WorkContextService::new(transcript.clone(), Arc::new(FakeStore::default()));

        let first = service.thread_resources("thread-1").unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(
            (first[0].kind.as_str(), first[0].source.as_str()),
            ("jira", "user")
        );
        assert_eq!(first[0].title, "ABC-1");
        assert_eq!(
            (first[1].kind.as_str(), first[1].source.as_str()),
            ("github", "agent")
        );

        let second = service.thread_resources("thread-1").unwrap();
        assert_eq!(second, first);
        // 세 번째 호출은 새 기록이 없다. 위치는 그대로이다.
        service.thread_resources("thread-1").unwrap();
        assert_eq!(
            *transcript.requests.lock().unwrap(),
            vec![None, Some("item-2".to_string()), Some("item-3".to_string())]
        );
    }

    /// 폴더 이름의 첫 조각을 저장소 식별자로 쓰는 가짜이다. `/one/wt` 와 `/one/main` 은 같은 저장소이다.
    struct FakeRepositories;

    impl RepositoryIdentityPort for FakeRepositories {
        fn repository_identity(&self, cwd: &str) -> Result<String> {
            match cwd.split('/').nth(1) {
                Some(name) if !name.is_empty() => Ok(format!("repo-{name}")),
                _ => bail!("not a repository"),
            }
        }
        fn task_exists(&self, _cwd: &str, task_id: &str) -> Result<bool> {
            Ok(task_id == "T-1")
        }
        fn validate_path(&self, _cwd: &str, path: &str) -> Result<()> {
            if path.starts_with("escape/") {
                bail!("outside fixture repository");
            }
            Ok(())
        }
    }

    fn scope(cwd: &str, scope: ContextScope) -> ContextScopeRequest {
        ContextScopeRequest {
            cwd: cwd.to_string(),
            scope,
        }
    }

    fn attach(scope: ContextScopeRequest, url: &str) -> AttachContextRequest {
        AttachContextRequest {
            scope,
            url: url.to_string(),
            source: "agent".to_string(),
        }
    }

    fn path(path: &str) -> ContextScope {
        ContextScope::Path {
            path: path.to_string(),
        }
    }

    #[test]
    fn context_selection_matches_complete_segments_and_explicit_preferences() {
        let (service, versions, _) = versioned_service("");
        let service = service.with_repository_identity(Arc::new(FakeRepositories));
        for n in 1..=7 {
            versions.set(&format!("a/b#{n}"), "v1");
        }
        let attach_at = |scope_kind, n| {
            service
                .attach_scope_resource(&attach(
                    scope("/one/wt", scope_kind),
                    &format!("https://github.com/a/b/pull/{n}"),
                ))
                .unwrap();
        };
        attach_at(ContextScope::Repository, 1);
        attach_at(path("src/lib"), 2);
        attach_at(path("src/lib/deep"), 2);
        attach_at(path("src/library"), 3);
        attach_at(
            ContextScope::Task {
                task_id: "T-1".into(),
            },
            4,
        );
        attach_at(path("src/lib"), 4);
        attach_at(path("src/lib"), 5);
        service
            .set_repository_preference(
                "/one/main",
                "https://github.com/a/b/pull/5",
                ContextPreference::Exclude,
            )
            .unwrap();
        service
            .set_repository_preference(
                "/one/main",
                "https://github.com/a/b/pull/6",
                ContextPreference::Pin,
            )
            .unwrap();
        let request = ContextSelectionRequest {
            cwd: "/one/main".into(),
            task_id: Some("T-1".into()),
            paths: vec!["src/lib/deep/main.rs".into()],
        };
        let asked = versions.asked.lock().unwrap().len();
        let entries = service.select_resources(&request).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.resource.key.as_str(), e.selected, e.reason))
                .collect::<Vec<_>>(),
            [
                ("a/b#6", true, "user_pin"),
                ("a/b#2", true, "path_prefix"),
                ("a/b#4", true, "task_link"),
                ("a/b#1", false, "repository_only"),
                ("a/b#5", false, "user_exclude")
            ]
        );
        assert_eq!(entries[1].matched_scope, Some(path("src/lib/deep")));
        assert_eq!(versions.asked.lock().unwrap().len(), asked);
        assert!(
            service
                .select_resources(&ContextSelectionRequest {
                    cwd: "/two/main".into(),
                    ..request.clone()
                })
                .unwrap()
                .is_empty()
        );
        for bad in ["../outside", "escape/file"] {
            assert!(
                service
                    .select_resources(&ContextSelectionRequest {
                        paths: vec![bad.into()],
                        ..request.clone()
                    })
                    .is_err()
            );
        }
        assert!(
            service
                .select_resources(&ContextSelectionRequest {
                    task_id: Some("missing".into()),
                    ..request.clone()
                })
                .is_err()
        );
        assert!(
            service
                .attach_scope_resource(&attach(
                    scope(
                        "/one/wt",
                        ContextScope::Task {
                            task_id: "missing".into()
                        }
                    ),
                    "https://github.com/a/b/pull/7"
                ))
                .is_err()
        );
        service
            .set_repository_preference(
                "/one/main",
                "https://github.com/a/b/pull/5",
                ContextPreference::Pin,
            )
            .unwrap();
        assert!(
            service
                .select_resources(&request)
                .unwrap()
                .iter()
                .any(|entry| entry.resource.key == "a/b#5"
                    && entry.selected
                    && entry.reason == "user_pin")
        );
        service
            .set_repository_preference(
                "/one/main",
                "https://github.com/a/b/pull/5",
                ContextPreference::Automatic,
            )
            .unwrap();
        assert!(
            service
                .select_resources(&request)
                .unwrap()
                .iter()
                .any(|entry| entry.resource.key == "a/b#5"
                    && entry.selected
                    && entry.reason == "path_prefix")
        );
    }

    // #2828: deterministic metadata-only baseline, not a model quality or token benchmark.
    #[test]
    fn context_selection_fixed_cases_measure_four_independent_metrics() {
        let (service, versions, _) = versioned_service("");
        let service = service.with_repository_identity(Arc::new(FakeRepositories));
        for n in 1..=7 {
            versions.set(&format!("a/b#{n}"), "v1");
        }
        let attach_at = |scope_kind, n| {
            service
                .attach_scope_resource(&attach(
                    scope("/one/wt", scope_kind),
                    &format!("https://github.com/a/b/pull/{n}"),
                ))
                .unwrap();
        };
        for n in 1..=6 {
            attach_at(ContextScope::Repository, n);
        }
        attach_at(path("src/server"), 3);
        attach_at(path("src/ui"), 4);
        attach_at(
            ContextScope::Task {
                task_id: "T-1".into(),
            },
            7,
        );
        service
            .set_repository_preference(
                "/one/main",
                "https://github.com/a/b/pull/5",
                ContextPreference::Pin,
            )
            .unwrap();
        service
            .set_repository_preference(
                "/one/main",
                "https://github.com/a/b/pull/6",
                ContextPreference::Exclude,
            )
            .unwrap();
        // Source oracle advances two documents after the cached ledger was observed.
        // Neither arm fetches again; selection cannot make an unobserved version fresh.
        versions.set("a/b#2", "v2");
        versions.set("a/b#3", "v2");
        let baseline = service
            .scope_resources(&scope("/one/wt", ContextScope::Repository))
            .unwrap();
        fn metrics(
            resources: &[ContextResource],
            required: &[&str],
        ) -> (usize, usize, usize, usize) {
            let keys: BTreeSet<_> = resources.iter().map(|r| r.key.as_str()).collect();
            let missing = required.iter().filter(|key| !keys.contains(**key)).count();
            let irrelevant = keys.iter().filter(|key| !required.contains(key)).count();
            let stale = resources
                .iter()
                .filter(|r| {
                    let current = if matches!(r.key.as_str(), "a/b#2" | "a/b#3") {
                        "v2"
                    } else {
                        "v1"
                    };
                    r.current_version.as_deref() != Some(current)
                })
                .count();
            let input = resources.iter().map(|r| serde_json::json!({"kind":r.kind,"key":r.key,"url":r.url,"version":r.current_version})).collect::<Vec<_>>();
            (
                missing,
                irrelevant,
                stale,
                serde_json::to_vec(&input).unwrap().len(),
            )
        }
        for (name, task, target, required, expected_before, expected_after) in [
            (
                "server",
                Some("T-1"),
                "src/server/api.rs",
                vec!["a/b#3", "a/b#5", "a/b#7"],
                (1, 4, 2),
                (0, 0, 1),
            ),
            (
                "ui",
                None,
                "src/ui/main.ts",
                vec!["a/b#4", "a/b#5"],
                (0, 4, 2),
                (0, 0, 0),
            ),
        ] {
            let request = ContextSelectionRequest {
                cwd: "/one/main".into(),
                task_id: task.map(str::to_owned),
                paths: vec![target.into()],
            };
            let select = || {
                service
                    .select_resources(&request)
                    .unwrap()
                    .into_iter()
                    .filter(|entry| entry.selected)
                    .map(|entry| entry.resource)
                    .collect::<Vec<_>>()
            };
            let selected = select();
            assert_eq!(
                selected,
                select(),
                "fixed case must replay deterministically"
            );
            let before = metrics(&baseline, &required);
            let after = metrics(&selected, &required);
            assert_eq!((before.0, before.1, before.2), expected_before);
            assert_eq!((after.0, after.1, after.2), expected_after);
            assert!(after.3 < before.3);
            println!(
                "context-eval-v1 {name}: missing/irrelevant/stale/input_bytes baseline={before:?} selected={after:?}"
            );
        }
    }

    #[test]
    fn attached_documents_keep_their_scope_source_and_confirmed_version() {
        // A03 first slice: explicit repository, task, and path scopes beside the thread scope.
        let (service, versions, now) = versioned_service("https://github.com/a/b/pull/2");
        let service = service.with_repository_identity(Arc::new(FakeRepositories));
        versions.set("a/b#2", "v1");
        let task = || {
            scope(
                "/one/wt",
                ContextScope::Task {
                    task_id: " T-1 ".to_string(),
                },
            )
        };

        let attached = service
            .attach_scope_resource(&attach(task(), "https://github.com/a/b/pull/2"))
            .unwrap();
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].source, "agent");
        assert_eq!(attached[0].confirmed_version.as_deref(), Some("v1"));
        assert_eq!(attached[0].current_version.as_deref(), Some("v1"));
        assert!(!attached[0].changed);
        // 다른 worktree 의 같은 작업은 같은 범위이다. 다른 저장소와 다른 범위 종류는 섞이지 않는다.
        assert_eq!(
            service
                .scope_resources(&scope(
                    "/one/main",
                    ContextScope::Task {
                        task_id: "T-1".to_string()
                    }
                ))
                .unwrap(),
            attached
        );
        assert!(
            service
                .scope_resources(&scope(
                    "/two/main",
                    ContextScope::Task {
                        task_id: "T-1".to_string()
                    }
                ))
                .unwrap()
                .is_empty()
        );
        assert!(
            service
                .scope_resources(&scope("/one/wt", ContextScope::Repository))
                .unwrap()
                .is_empty()
        );

        // 경로는 한 가지 모양으로 적는다. 저장소 밖 경로는 받지 않는다.
        service
            .attach_scope_resource(&attach(
                scope("/one/wt", path("./src\\lib")),
                "https://github.com/a/b/pull/2",
            ))
            .unwrap();
        assert_eq!(
            service
                .scope_resources(&scope("/one/main", path("src/lib/")))
                .unwrap()
                .len(),
            1
        );
        for bad in ["../x", "/one/wt/src", "src/../.."] {
            assert!(
                service
                    .attach_scope_resource(&attach(
                        scope("/one/wt", path(bad)),
                        "https://github.com/a/b/pull/2"
                    ))
                    .is_err(),
                "{bad}"
            );
        }

        // 나중에 확인한 버전이 다르면 바뀜이다. 목록은 외부 서비스에 묻지 않는다.
        versions.set("a/b#2", "v2");
        *now.lock().unwrap() += chrono::Duration::seconds(6 * 60);
        service.refresh_thread_resources("thread-1").unwrap();
        let asked = versions.asked.lock().unwrap().len();
        let listed = service.scope_resources(&task()).unwrap();
        assert_eq!(versions.asked.lock().unwrap().len(), asked);
        assert_eq!(listed[0].confirmed_version.as_deref(), Some("v1"));
        assert_eq!(listed[0].current_version.as_deref(), Some("v2"));
        assert!(listed[0].changed);
    }

    #[test]
    fn scoped_refresh_and_read_keep_the_other_scope_and_reject_stale_versions() {
        let (service, versions, now) = versioned_service("");
        let service = service.with_repository_identity(Arc::new(FakeRepositories));
        let repo = scope("/one/main", ContextScope::Repository);
        let task = scope(
            "/one/wt",
            ContextScope::Task {
                task_id: "T-1".to_string(),
            },
        );
        let url = "https://github.com/a/b/pull/2";
        versions.set("a/b#2", "v1");
        service
            .attach_scope_resource(&attach(repo.clone(), url))
            .unwrap();
        service
            .attach_scope_resource(&attach(task.clone(), url))
            .unwrap();
        versions.set("a/b#2", "v2");
        *now.lock().unwrap() += chrono::Duration::seconds(6 * 60);
        let refreshed = service.refresh_scope_resources(&task).unwrap();
        assert!(refreshed[0].changed);
        assert_eq!(refreshed[0].current_version.as_deref(), Some("v2"));
        assert!(
            service
                .mark_scope_resource_read(&task, "github", "a/b#2", "v1")
                .is_err()
        );
        assert!(
            service
                .mark_scope_resource_read(&task, "github", "a/b#3", "v2")
                .is_err()
        );
        assert!(service.scope_resources(&task).unwrap()[0].changed);
        let read = service
            .mark_scope_resource_read(&task, "github", "a/b#2", "v2")
            .unwrap();
        assert!(!read[0].changed);
        assert_eq!(read[0].confirmed_version.as_deref(), Some("v2"));
        let other = service.scope_resources(&repo).unwrap();
        assert!(other[0].changed);
        assert_eq!(other[0].confirmed_version.as_deref(), Some("v1"));
    }

    #[test]
    fn attach_checks_the_attached_document_even_when_the_scope_has_many_stale_documents() {
        let (service, versions, now) = versioned_service("");
        let service = service.with_repository_identity(Arc::new(FakeRepositories));
        let repository = || scope("/one", ContextScope::Repository);
        for number in 0..MAX_VERSION_CHECKS_PER_REFRESH {
            let url = format!("https://github.com/a/b/pull/{number}");
            versions.set(&format!("a/b#{number}"), "old");
            service
                .attach_scope_resource(&attach(repository(), &url))
                .unwrap();
        }
        *now.lock().unwrap() += chrono::Duration::seconds(6 * 60);
        versions.set("a/b#99", "new");
        versions.asked.lock().unwrap().clear();

        let listed = service
            .attach_scope_resource(&attach(repository(), "https://github.com/a/b/pull/99"))
            .unwrap();
        assert_eq!(*versions.asked.lock().unwrap(), ["a/b#99"]);
        let attached = listed.iter().find(|item| item.key == "a/b#99").unwrap();
        assert_eq!(attached.confirmed_version.as_deref(), Some("new"));
    }

    #[test]
    fn attach_rejects_unknown_documents_sources_and_repositories() {
        let service = WorkContextService::new(
            Arc::new(FakeTranscript::default()),
            Arc::new(FakeStore::default()),
        );
        let request = attach(
            scope("/one", ContextScope::Repository),
            "https://github.com/a/b/pull/2",
        );
        assert!(
            service.attach_scope_resource(&request).is_err(),
            "no repository identity"
        );
        let service = service.with_repository_identity(Arc::new(FakeRepositories));
        assert_eq!(service.attach_scope_resource(&request).unwrap().len(), 1);
        for url in [
            "https://example.com/doc",
            "https://github.com/a/b/pull/2 https://github.com/a/b/pull/3",
        ] {
            let mut bad = request.clone();
            bad.url = url.to_string();
            assert!(service.attach_scope_resource(&bad).is_err(), "{url}");
        }
        let mut bad = request.clone();
        bad.source = "system".to_string();
        assert!(service.attach_scope_resource(&bad).is_err());
        let mut bad = request.clone();
        bad.scope.cwd = "/".to_string();
        assert!(service.attach_scope_resource(&bad).is_err());
        assert!(
            service
                .scope_resources(&scope(
                    "/one",
                    ContextScope::Task {
                        task_id: " ".to_string()
                    }
                ))
                .is_err()
        );
    }

    #[test]
    fn threads_do_not_share_documents_and_bad_ids_are_rejected() {
        let transcript = Arc::new(FakeTranscript::default());
        transcript.pages.lock().unwrap().push(ThreadTextPage {
            fragments: vec![fragment("user", "https://github.com/a/b/issues/9")],
            newest_item_id: Some("item-1".into()),
        });
        let service = WorkContextService::new(transcript, Arc::new(FakeStore::default()));
        assert_eq!(service.thread_resources("thread-1").unwrap().len(), 1);
        assert!(service.thread_resources("thread-2").unwrap().is_empty());
        assert!(service.thread_resources("  ").is_err());
    }
}
