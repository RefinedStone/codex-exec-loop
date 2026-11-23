//! Bounded in-memory source for application and kernel tests; no Git or model process.
use std::collections::BTreeMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::application::port::outbound::issue_assessment_evidence_port::*;

pub(crate) struct FakeIssueAssessmentEvidenceAdapter {
    state: Mutex<State>,
}

struct State {
    source: String,
    files: BTreeMap<String, String>,
    unavailable: bool,
    reads: usize,
    after_next_read: Option<Box<dyn FnOnce() + Send>>,
}

impl Default for FakeIssueAssessmentEvidenceAdapter {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                source: "a".repeat(40),
                files: BTreeMap::from([(
                    "README.md".into(),
                    "Public repository fixture.\n".into(),
                )]),
                unavailable: false,
                reads: 0,
                after_next_read: None,
            }),
        }
    }
}

impl FakeIssueAssessmentEvidenceAdapter {
    pub(crate) fn set_commit_sha(&self, source: String) {
        self.state.lock().unwrap().source = source;
    }

    pub(crate) fn set_file(&self, path: &str, text: &str) {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(path.to_owned(), text.to_owned());
    }

    pub(crate) fn make_unavailable(&self) {
        self.state.lock().unwrap().unavailable = true;
    }

    pub(crate) fn reads(&self) -> usize {
        self.state.lock().unwrap().reads
    }

    pub(crate) fn after_next_read(&self, hook: impl FnOnce() + Send + 'static) {
        self.state.lock().unwrap().after_next_read = Some(Box::new(hook));
    }
}

impl IssueAssessmentEvidencePort for FakeIssueAssessmentEvidenceAdapter {
    fn current_commit_sha(&self, _workspace: &str) -> Result<String, IssueAssessmentEvidenceError> {
        let state = self.state.lock().unwrap();
        if state.unavailable {
            return Err(IssueAssessmentEvidenceError::Unavailable);
        }
        Ok(state.source.clone())
    }

    fn read(
        &self,
        request: IssueAssessmentEvidenceRequest<'_>,
    ) -> Result<IssueAssessmentRepositoryEvidence, IssueAssessmentEvidenceError> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        if state.unavailable {
            return Err(IssueAssessmentEvidenceError::Unavailable);
        }
        if request.commit_sha != state.source {
            return Err(IssueAssessmentEvidenceError::SourceChanged);
        }
        let paths = super::permitted_paths(request.selected_paths, request.policy)?;
        let entries = paths
            .iter()
            .map(|path| match state.files.get(path) {
                Some(text) => IssueAssessmentEvidenceEntry::File {
                    path: path.clone(),
                    blob_sha: format!("{:x}", Sha256::digest(text.as_bytes())),
                    text: text.clone(),
                },
                None => IssueAssessmentEvidenceEntry::Absent { path: path.clone() },
            })
            .collect::<Vec<_>>();
        let encoded =
            serde_json::to_vec(&(ISSUE_ASSESSMENT_EVIDENCE_VERSION, &state.source, &entries))
                .unwrap();
        let evidence = IssueAssessmentRepositoryEvidence {
            commit_sha: state.source.clone(),
            selection_version: ISSUE_ASSESSMENT_EVIDENCE_VERSION.into(),
            snapshot_sha256: format!("{:x}", Sha256::digest(encoded)),
            entries,
        };
        let hook = state.after_next_read.take();
        drop(state);
        // The host has started capturing this exact snapshot, but has not observed the
        // return yet. Hooks model configuration changes and elapsed time at that boundary.
        if let Some(hook) = hook {
            hook();
        }
        Ok(evidence)
    }
}
