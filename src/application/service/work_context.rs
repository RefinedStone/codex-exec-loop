use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::application::port::inbound::work_context_port::{ThreadResource, WorkContextPort};
use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
use crate::application::port::outbound::thread_transcript_port::ThreadTranscriptPort;
use crate::application::port::outbound::work_context_store_port::{
    ResourceVersionCheck, WORK_CONTEXT_SCOPE_THREAD, WorkContextLink, WorkContextStorePort,
};
use crate::domain::work_context::{
    ExternalResourceRecognizer, external_resource_recognizers, extract_external_resources,
};

/// 스레드 id 는 장부의 키이다. app-server 의 id 는 짧은 UUID 이다.
const MAX_THREAD_ID_CHARS: usize = 256;
/// 문서의 버전을 다시 묻기까지의 시간이다. 그 안에는 장부의 값을 쓴다.
const VERSION_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// 한 번에 외부 서비스에 묻는 문서 수의 상한이다. 문서가 많은 스레드도 정해진 시간 안에 답한다.
const MAX_VERSION_CHECKS_PER_REFRESH: usize = 20;

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
    clock: Clock,
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

    fn links(&self, thread_id: &str) -> Result<Vec<WorkContextLink>> {
        self.store.links(WORK_CONTEXT_SCOPE_THREAD, thread_id)
    }

    fn resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>> {
        Ok(self
            .links(thread_id)?
            .into_iter()
            .map(thread_resource)
            .collect())
    }

    fn version_provider(&self, kind: &str) -> Option<&dyn ExternalResourceVersionPort> {
        self.version_providers
            .iter()
            .find(|provider| provider.kind() == kind)
            .map(Arc::as_ref)
    }

    /// 확인한 지 오래된 문서의 지금 버전을 제공자에게 묻고 장부에 적는다. 처음 확인한 버전은 읽은 버전이 된다.
    fn check_versions(&self, thread_id: &str) -> Result<()> {
        let now = (self.clock)();
        let mut checks = 0;
        for link in self.links(thread_id)? {
            let Some(provider) = self.version_provider(&link.resource.kind) else {
                continue;
            };
            if !check_is_due(link.checked_version.as_ref(), now) {
                continue;
            }
            if checks == MAX_VERSION_CHECKS_PER_REFRESH {
                break;
            }
            checks += 1;
            // 닿지 않는 문서는 건너뛴다. 이전 값이 남고, 다음 확인에서 다시 묻는다.
            let Ok(version) = provider.current_version(&link.resource) else {
                continue;
            };
            self.store.record_resource_version(
                &link.resource,
                &ResourceVersionCheck {
                    version: version.clone(),
                    checked_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
                },
            )?;
            self.store.set_read_version(
                WORK_CONTEXT_SCOPE_THREAD,
                thread_id,
                &link.resource,
                &version,
                false,
            )?;
        }
        Ok(())
    }
}

/// 확인한 적이 없거나, 확인한 지 `VERSION_CHECK_INTERVAL` 이 지났다. 읽지 못하는 시각은 지난 것으로 본다.
fn check_is_due(check: Option<&ResourceVersionCheck>, now: DateTime<Utc>) -> bool {
    let Some(checked_at) = check
        .and_then(|check| DateTime::parse_from_rfc3339(&check.checked_at).ok())
        .map(|checked_at| checked_at.with_timezone(&Utc))
    else {
        return true;
    };
    now.signed_duration_since(checked_at)
        .to_std()
        .is_ok_and(|elapsed| elapsed >= VERSION_CHECK_INTERVAL)
}

fn thread_resource(link: WorkContextLink) -> ThreadResource {
    let changed = match (&link.checked_version, &link.read_version) {
        (Some(check), Some(read)) => &check.version != read,
        _ => false,
    };
    ThreadResource {
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
        version_checked_at: link.checked_version.map(|check| check.checked_at),
    }
}

fn valid_thread_id(thread_id: &str) -> Result<&str> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || thread_id.chars().count() > MAX_THREAD_ID_CHARS {
        bail!("thread id is not valid")
    }
    Ok(thread_id)
}

impl WorkContextPort for WorkContextService {
    fn thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>> {
        let thread_id = valid_thread_id(thread_id)?;
        self.collect(thread_id)?;
        self.resources(thread_id)
    }

    fn refresh_thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>> {
        let thread_id = valid_thread_id(thread_id)?;
        self.collect(thread_id)?;
        self.check_versions(thread_id)?;
        self.resources(thread_id)
    }

    fn mark_thread_resource_read(
        &self,
        thread_id: &str,
        resource_kind: &str,
        resource_key: &str,
    ) -> Result<Vec<ThreadResource>> {
        let thread_id = valid_thread_id(thread_id)?;
        let link = self
            .links(thread_id)?
            .into_iter()
            .find(|link| link.resource.kind == resource_kind && link.resource.key == resource_key);
        let Some(link) = link else {
            bail!("the thread has no such document")
        };
        // 확인한 버전이 없으면 바뀜도 없다. 적을 것이 없다.
        if let Some(check) = &link.checked_version {
            self.store.set_read_version(
                WORK_CONTEXT_SCOPE_THREAD,
                thread_id,
                &link.resource,
                &check.version,
                true,
            )?;
        }
        self.resources(thread_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::thread_transcript_port::{
        ThreadTextFragment, ThreadTextPage,
    };
    use crate::application::port::outbound::work_context_store_port::WorkContextLink;
    use crate::domain::work_context::ExternalResourceRef;
    use std::collections::HashMap;
    use std::sync::Mutex;

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
    }

    fn same_document(link: &WorkContextLink, resource: &ExternalResourceRef) -> bool {
        link.resource.kind == resource.kind && link.resource.key == resource.key
    }

    impl WorkContextStorePort for FakeStore {
        fn link_resources(
            &self,
            _scope_kind: &str,
            scope_id: &str,
            source: &str,
            resources: &[ExternalResourceRef],
        ) -> Result<()> {
            let mut links = self.links.lock().unwrap();
            for resource in resources {
                let known = links.iter().any(|(scope, link)| {
                    scope == scope_id
                        && link.resource.kind == resource.kind
                        && link.resource.key == resource.key
                });
                if !known {
                    links.push((
                        scope_id.to_string(),
                        WorkContextLink {
                            resource: resource.clone(),
                            source: source.to_string(),
                            first_seen_at: "2026-11-11T00:00:00Z".to_string(),
                            checked_version: None,
                            read_version: None,
                        },
                    ));
                }
            }
            Ok(())
        }
        fn links(&self, _scope_kind: &str, scope_id: &str) -> Result<Vec<WorkContextLink>> {
            Ok(self
                .links
                .lock()
                .unwrap()
                .iter()
                .filter(|(scope, _)| scope == scope_id)
                .map(|(_, link)| link.clone())
                .collect())
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
            _scope_kind: &str,
            scope_id: &str,
            resource: &ExternalResourceRef,
            version: &str,
            replace: bool,
        ) -> Result<()> {
            for (scope, link) in self.links.lock().unwrap().iter_mut() {
                if scope == scope_id
                    && same_document(link, resource)
                    && (replace || link.read_version.is_none())
                {
                    link.read_version = Some(version.to_string());
                }
            }
            Ok(())
        }
    }

    /// 문서 키마다 지금 버전을 답하는 가짜 GitHub 이다. 버전이 없는 키는 닿지 않는 문서이다.
    #[derive(Default)]
    struct FakeVersions {
        versions: Mutex<HashMap<String, String>>,
        asked: Mutex<Vec<String>>,
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
        fn current_version(&self, resource: &ExternalResourceRef) -> Result<String> {
            self.asked.lock().unwrap().push(resource.key.clone());
            self.versions
                .lock()
                .unwrap()
                .get(&resource.key)
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

    fn changed_flags(resources: &[ThreadResource]) -> Vec<(&str, bool)> {
        resources
            .iter()
            .map(|resource| (resource.key.as_str(), resource.changed))
            .collect()
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

        // 닿지 않던 문서는 다음에 바로 다시 묻는다. 처음 닿은 버전이 기준이 된다.
        versions.set("a/b#3", "v7");
        *now.lock().unwrap() += chrono::Duration::seconds(10);
        let second = service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(changed_flags(&second)[1], ("a/b#3", false));
        assert_eq!(
            *versions.asked.lock().unwrap(),
            ["a/b#2", "a/b#3", "a/b#3"],
            "the checked document waits for its interval"
        );
    }

    #[test]
    fn one_refresh_asks_a_bounded_number_of_documents() {
        let text = (1..=MAX_VERSION_CHECKS_PER_REFRESH + 5)
            .map(|number| format!("https://github.com/a/b/issues/{number}"))
            .collect::<Vec<_>>()
            .join(" ");
        let (service, versions, _now) = versioned_service(&text);

        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            versions.asked.lock().unwrap().len(),
            MAX_VERSION_CHECKS_PER_REFRESH
        );
        // 남은 문서는 다음 확인에서 묻는다. 닿지 않은 문서는 기다리지 않기 때문이다.
        service.refresh_thread_resources("thread-1").unwrap();
        assert_eq!(
            versions.asked.lock().unwrap().len(),
            2 * MAX_VERSION_CHECKS_PER_REFRESH
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
