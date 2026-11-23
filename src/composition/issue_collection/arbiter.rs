//! One long-lived read-only model driver per repository coordinator.
use std::sync::Arc;

use anyhow::{Result, ensure};

use crate::adapter::outbound::issue_assessment_model::{
    CodexIssueAssessmentModelAdapter, UnavailableIssueAssessmentModelAdapter,
};
use crate::application::port::outbound::issue_assessment_model_port::{
    ISSUE_ASSESSMENT_PROFILE_VERSION, IssueAssessmentModelConfiguration, IssueAssessmentModelPort,
};
use crate::domain::delegation::policy::{CODEX, Route};
use crate::domain::delegation::{Effect, ISSUE_ASSESSMENT_FEATURE};

pub(super) fn model(workspace: &str) -> Arc<dyn IssueAssessmentModelPort> {
    let workspace = workspace.to_owned();
    let reader = Arc::new(move || {
        configuration(super::super::production::delegation_route(
            &workspace,
            ISSUE_ASSESSMENT_FEATURE,
        )?)
    });
    match CodexIssueAssessmentModelAdapter::new(reader) {
        Ok(adapter) => Arc::new(adapter),
        Err(_) => Arc::new(UnavailableIssueAssessmentModelAdapter),
    }
}

fn configuration(route: Route) -> Result<IssueAssessmentModelConfiguration> {
    let [backend] = route.backends.as_slice() else {
        anyhow::bail!("issue assessment requires one read-only backend")
    };
    ensure!(
        backend.kind == CODEX
            && backend.max_resumes == Some(0)
            && route.effect == Effect::ReadOnly
            && route.fall_back_on.is_empty(),
        "issue assessment cannot resume or fall back"
    );
    let timeout_seconds = u64::try_from(route.deadline_ms)? / 1000;
    ensure!(
        timeout_seconds > 0,
        "issue assessment deadline is too short"
    );
    Ok(IssueAssessmentModelConfiguration {
        route_revision: route.revision,
        model: backend.model.clone(),
        max_running: route.max_running.min(backend.max_concurrent),
        max_started_per_day: route.max_started_per_day,
        timeout_seconds,
        profile_version: ISSUE_ASSESSMENT_PROFILE_VERSION.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::delegation::policy::{PolicyFile, resolve};

    fn route() -> Route {
        resolve(
            PolicyFile::default(),
            PolicyFile::default(),
            ISSUE_ASSESSMENT_FEATURE,
            "a".repeat(64),
        )
        .unwrap()
    }

    #[test]
    fn assessment_configuration_enforces_the_selected_backend_capacity() {
        let mut source = route();
        source.max_running = 8;
        source.backends[0].max_concurrent = 2;
        let result = configuration(source).unwrap();
        assert_eq!(result.max_running, 2);
        assert_eq!(result.max_started_per_day, 5);
        assert_eq!(result.timeout_seconds, 180);
    }

    #[test]
    fn assessment_configuration_cannot_enable_resume_or_another_backend() {
        let mut source = route();
        source.backends[0].max_resumes = Some(1);
        assert!(configuration(source).is_err());
        let mut source = route();
        source.backends[0].kind = "chatgpt_chat".into();
        assert!(configuration(source).is_err());
        let mut source = route();
        source.backends.push(source.backends[0].clone());
        assert!(configuration(source).is_err());
    }
}
