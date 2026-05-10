use std::sync::mpsc::Sender;
use std::thread;

use anyhow::Result;

use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::core::app::SessionCatalogReadySnapshot;
use crate::core::app::{CoreEffect, CoreEffectCompletion, CoreInput, StartupReadySnapshot};
use crate::core::runtime::CoreEffectExecutor;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Clone)]
pub struct CoreEffectRunner {
    startup_service: StartupService,
    session_service: SessionService,
    input_sender: Sender<CoreInput>,
}

impl CoreEffectRunner {
    pub fn new(
        startup_service: StartupService,
        session_service: SessionService,
        input_sender: Sender<CoreInput>,
    ) -> Self {
        Self {
            startup_service,
            session_service,
            input_sender,
        }
    }

    pub fn spawn_startup_checks(&self) {
        let startup_service = self.startup_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let completion = startup_checks_completion(startup_service.run_checks());
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn run_effect(&self, effect: CoreEffect) {
        match effect {
            CoreEffect::RunStartupChecks => self.spawn_startup_checks(),
            CoreEffect::LoadSessionCatalog {
                limit,
                workspace_directory,
            } => self.spawn_session_catalog_load(limit, workspace_directory),
        }
    }

    pub fn spawn_session_catalog_load(&self, limit: usize, workspace_directory: String) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let request = SessionCatalogRequest::for_workspace(limit, workspace_directory);
            let completion =
                session_catalog_completion(session_service.load_session_catalog(request));
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }
}

impl CoreEffectExecutor for CoreEffectRunner {
    fn run_effect(&self, effect: CoreEffect) {
        CoreEffectRunner::run_effect(self, effect);
    }
}

fn startup_checks_completion(result: Result<StartupDiagnostics>) -> CoreEffectCompletion {
    CoreEffectCompletion::StartupChecksLoaded(
        result
            .map(StartupReadySnapshot::from_diagnostics)
            .map(Box::new)
            .map_err(|error| error.to_string()),
    )
}

fn session_catalog_completion(result: Result<SessionCatalog>) -> CoreEffectCompletion {
    CoreEffectCompletion::SessionCatalogLoaded(
        result
            .map(SessionCatalogReadySnapshot::from_catalog)
            .map_err(|error| error.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalogTier};
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    fn startup_success_maps_to_core_completion() {
        let diagnostics = StartupDiagnostics {
            cwd: "/tmp/workspace".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/usr/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/workspace".to_string(),
            workspace_detail: "git repo: /tmp/workspace".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::default(),
            initialize_ok: true,
            initialize_detail: "initialized".to_string(),
            account_ok: true,
            account_detail: "authenticated".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "embedded schema".to_string(),
        };

        assert_eq!(
            startup_checks_completion(Ok(diagnostics)),
            CoreEffectCompletion::StartupChecksLoaded(Ok(Box::new(StartupReadySnapshot {
                cwd: "/tmp/workspace".to_string(),
                workspace_path: "/tmp/workspace".to_string(),
                can_continue: true,
                codex_binary: crate::core::app::StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "/usr/bin/codex".to_string(),
                },
                workspace: crate::core::app::StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "git repo: /tmp/workspace".to_string(),
                },
                app_server_initialize: crate::core::app::StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "initialized".to_string(),
                },
                account: crate::core::app::StartupDiagnosticSnapshot {
                    ok: true,
                    detail: "authenticated".to_string(),
                },
                attachment: crate::core::app::StartupAttachmentSnapshot {
                    mode_label: "provider-launched".to_string(),
                    recovery_anchor_label: "provider-thread-id".to_string(),
                },
                warnings: Vec::new(),
                schema_snapshot: "embedded schema".to_string(),
            })))
        );
    }

    #[test]
    fn startup_error_maps_to_core_completion() {
        assert_eq!(
            startup_checks_completion(Err(anyhow::anyhow!("codex missing"))),
            CoreEffectCompletion::StartupChecksLoaded(Err("codex missing".to_string()))
        );
    }

    #[test]
    fn session_catalog_success_maps_to_core_completion() {
        let catalog = RecentSessions {
            items: Vec::new(),
            warnings: vec!["partial catalog".to_string()],
            next_cursor: None,
        }
        .into();

        assert_eq!(
            session_catalog_completion(Ok(catalog)),
            CoreEffectCompletion::SessionCatalogLoaded(Ok(SessionCatalogReadySnapshot {
                tier_label: SessionCatalogTier::ProviderBackedCatalog
                    .label()
                    .to_string(),
                item_count: 0,
                warnings: vec!["partial catalog".to_string()],
            }))
        );
    }

    #[test]
    fn session_catalog_error_maps_to_core_completion() {
        assert_eq!(
            session_catalog_completion(Err(anyhow::anyhow!("catalog unavailable"))),
            CoreEffectCompletion::SessionCatalogLoaded(Err("catalog unavailable".to_string()))
        );
    }
}
