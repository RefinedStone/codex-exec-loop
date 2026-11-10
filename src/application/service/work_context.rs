use std::sync::Arc;

use anyhow::{Result, bail};

use crate::application::port::inbound::work_context_port::{ThreadResource, WorkContextPort};
use crate::application::port::outbound::thread_transcript_port::ThreadTranscriptPort;
use crate::application::port::outbound::work_context_store_port::{
    WORK_CONTEXT_SCOPE_THREAD, WorkContextStorePort,
};
use crate::domain::work_context::{
    ExternalResourceRecognizer, external_resource_recognizers, extract_external_resources,
};

/// 스레드 id 는 장부의 키이다. app-server 의 id 는 짧은 UUID 이다.
const MAX_THREAD_ID_CHARS: usize = 256;

/*
 * 작업 컨텍스트 수집기이다 (plan 22). 스레드의 새 기록만 읽고, 인식기로 외부 문서를 찾아 장부에 잇는다.
 * 모델을 부르지 않는다. 읽은 위치는 장부에 남겨서 같은 기록을 다시 읽지 않는다.
 */
pub struct WorkContextService {
    transcript: Arc<dyn ThreadTranscriptPort>,
    store: Arc<dyn WorkContextStorePort>,
    recognizers: Vec<Box<dyn ExternalResourceRecognizer>>,
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
        }
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
}

impl WorkContextPort for WorkContextService {
    fn thread_resources(&self, thread_id: &str) -> Result<Vec<ThreadResource>> {
        let thread_id = thread_id.trim();
        if thread_id.is_empty() || thread_id.chars().count() > MAX_THREAD_ID_CHARS {
            bail!("thread id is not valid")
        }
        self.collect(thread_id)?;
        Ok(self
            .store
            .links(WORK_CONTEXT_SCOPE_THREAD, thread_id)?
            .into_iter()
            .map(|link| ThreadResource {
                title: link
                    .resource
                    .title_hint
                    .unwrap_or_else(|| link.resource.url.clone()),
                kind: link.resource.kind,
                key: link.resource.key,
                url: link.resource.url,
                source: link.source,
                first_seen_at: link.first_seen_at,
            })
            .collect())
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
