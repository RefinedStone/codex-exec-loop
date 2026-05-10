use std::sync::mpsc::Sender;
use std::thread;

use anyhow::Result;

use crate::application::service::startup_service::StartupService;
use crate::core::app::{CoreEffect, CoreEffectCompletion, CoreInput, StartupReadySnapshot};
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Clone)]
pub struct CoreEffectRunner {
    startup_service: StartupService,
    input_sender: Sender<CoreInput>,
}

impl CoreEffectRunner {
    pub fn new(startup_service: StartupService, input_sender: Sender<CoreInput>) -> Self {
        Self {
            startup_service,
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
        }
    }
}

fn startup_checks_completion(result: Result<StartupDiagnostics>) -> CoreEffectCompletion {
    CoreEffectCompletion::StartupChecksLoaded(
        result
            .map(StartupReadySnapshot::from_diagnostics)
            .map_err(|error| error.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
            CoreEffectCompletion::StartupChecksLoaded(Ok(StartupReadySnapshot {
                workspace_path: "/tmp/workspace".to_string(),
                can_continue: true,
                warnings: Vec::new(),
            }))
        );
    }

    #[test]
    fn startup_error_maps_to_core_completion() {
        assert_eq!(
            startup_checks_completion(Err(anyhow::anyhow!("codex missing"))),
            CoreEffectCompletion::StartupChecksLoaded(Err("codex missing".to_string()))
        );
    }
}
