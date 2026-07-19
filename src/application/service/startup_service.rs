// startup check는 git workspace를 확인하기 위해 짧은 외부 명령을 실행한다.
// `Command`는 `git rev-parse`를 호출하는 도구이고, `Stdio`는 startup overlay에 불필요한 stderr가 섞이지 않게 제어한다.
use std::path::Path;
use std::process::Stdio;
// `Arc`는 TUI app runtime과 background startup task가 같은 startup probe adapter를 공유하게 한다.
use std::sync::Arc;

// `Context`는 low-level 오류에 "무엇을 하다 실패했는지"를 덧붙인다.
// startup 실패는 첫 화면에 바로 노출되므로, 단순 io error보다 사용자가 이해할 수 있는 문맥이 중요하다.
use anyhow::{Context, Result};

use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptLogMaintenanceMode, AppServerPromptLogMaintenancePort,
};
// startup probe port는 app-server 쪽 account/init/attachment 상태를 읽는 outbound 경계이다.
// 이 service는 app-server JSON이나 connection lifecycle을 모르고, port가 정규화한 startup context만 받는다.
use crate::application::port::outbound::startup_probe_port::StartupProbePort;
// `StartupDiagnostics`는 startup overlay, prompt submit gating, recent-session loading gating이
// 공통으로 읽는 domain snapshot이다.
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::git_subprocess;
use crate::panic_observation::catch_redacted_worker_unwind;
use crate::subprocess;

#[derive(Clone)]
/*
StartupService는 TUI가 첫 화면에서 보여 주는 startup diagnostics를 만드는 application
facade이다. local process checks(`codex` binary, current cwd, git root)와 app-server를 통한
startup context(account, attachment profile, initialize detail)를 하나의 `StartupDiagnostics`로
합친다.

이 service가 outbound port를 받는 이유는 app-server protocol 세부 사항을 service 밖으로 밀어내기
위해서이다. TUI runtime은 `run_checks` 결과만 받아 Ready/Failed state로 줄이고, rendering layer는
domain diagnostics를 화면 문구로 바꾼다.
*/
pub struct StartupService {
    // app-server startup probe 구현이다. local shell check는 service 내부에서 처리하고,
    // account/init/attachment처럼 app-server가 알아야 하는 값만 이 port로 위임한다.
    startup_probe_port: Arc<dyn StartupProbePort>,
    prompt_log_maintenance: Option<PromptLogMaintenance>,
    #[cfg(test)]
    local_startup_prerequisites: Option<LocalStartupPrerequisites>,
}

impl StartupService {
    // startup service를 구성한다. shell entrypoint는 실제 app-server adapter를 넘기고,
    // TUI runtime tests는 fake port를 넣어 startup state transition만 검증할 수 있다.
    pub fn new(startup_probe_port: Arc<dyn StartupProbePort>) -> Self {
        /*
        startup probe port는 app-server와 통신하는 outbound capability이다. Arc로 보관해
        TUI runtime이 background startup task에 service clone을 넘겨도 같은 adapter/runtime handle을
        공유할 수 있다.
        */
        Self {
            startup_probe_port,
            prompt_log_maintenance: None,
            #[cfg(test)]
            local_startup_prerequisites: None,
        }
    }

    pub fn with_prompt_log_maintenance(
        mut self,
        port: Arc<dyn AppServerPromptLogMaintenancePort>,
        mode: AppServerPromptLogMaintenanceMode,
    ) -> Self {
        self.prompt_log_maintenance = Some(PromptLogMaintenance { port, mode });
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_local_startup_prerequisites(
        mut self,
        workspace_directory: &str,
    ) -> Self {
        self.local_startup_prerequisites = Some(LocalStartupPrerequisites {
            current_directory: workspace_directory.to_string(),
            codex_binary_detail: "/usr/bin/codex".to_string(),
            workspace_status: WorkspaceStatus {
                ok: true,
                path: workspace_directory.to_string(),
                detail: format!("git repo: {workspace_directory}"),
            },
        });
        self
    }

    // startup overlay에 필요한 전체 diagnostics를 한 번 수집한다.
    // 이 함수의 성공/실패는 `AppRuntime`의 background message로 돌아가 `StartupState::Ready` 또는
    // `StartupState::Failed`로 줄어든다.
    pub fn run_checks(&self, workspace_directory: &str) -> Result<StartupDiagnostics> {
        self.run_checks_with_local_prerequisites(workspace_directory, || {
            #[cfg(test)]
            if let Some(local) = self.local_startup_prerequisites.clone() {
                return Ok(local);
            }
            load_local_startup_prerequisites(workspace_directory)
        })
    }

    fn run_checks_with_local_prerequisites(
        &self,
        workspace_directory: &str,
        load_local_prerequisites: impl FnOnce() -> Result<LocalStartupPrerequisites>,
    ) -> Result<StartupDiagnostics> {
        let maintenance_warning = self.maintain_prompt_logs_best_effort(workspace_directory);
        let local = load_local_prerequisites()?;
        let startup_context = self.startup_probe_port.load_startup_context()?;
        let mut warnings = startup_context.warnings;
        if let Some(warning) = maintenance_warning {
            warnings.push(warning.operator_message().to_string());
        }

        Ok(StartupDiagnostics {
            cwd: local.current_directory,
            codex_binary_ok: true,
            codex_binary_detail: local.codex_binary_detail,
            workspace_ok: local.workspace_status.ok,
            workspace_path: local.workspace_status.path,
            workspace_detail: local.workspace_status.detail,
            attachment_profile: startup_context.attachment_profile,
            initialize_ok: true,
            initialize_detail: startup_context.initialize_detail,
            account_ok: startup_context.account_ok,
            account_detail: startup_context.account_detail,
            warnings,
            schema_snapshot: StartupDiagnostics::bundled_schema_snapshot_label(),
        })
    }

    fn maintain_prompt_logs_best_effort(
        &self,
        workspace_directory: &str,
    ) -> Option<StartupMaintenanceWarning> {
        let maintenance = self.prompt_log_maintenance.as_ref()?;
        match catch_redacted_worker_unwind(|| {
            maintenance
                .port
                .maintain_app_server_prompt_logs(workspace_directory, maintenance.mode)
        }) {
            Ok(Ok(_)) => None,
            Ok(Err(_)) => {
                tracing::warn!(
                    mode = ?maintenance.mode,
                    "app-server prompt-log privacy maintenance failed during startup"
                );
                Some(StartupMaintenanceWarning::PromptLogMaintenanceFailed)
            }
            Err(_) => {
                tracing::warn!(
                    mode = ?maintenance.mode,
                    "app-server prompt-log privacy maintenance panicked during startup"
                );
                Some(StartupMaintenanceWarning::PromptLogMaintenancePanicked)
            }
        }
    }
}

#[derive(Clone)]
struct LocalStartupPrerequisites {
    current_directory: String,
    codex_binary_detail: String,
    workspace_status: WorkspaceStatus,
}

fn load_local_startup_prerequisites(
    workspace_directory: &str,
) -> Result<LocalStartupPrerequisites> {
    let current_directory = std::env::current_dir()
        .context("failed to resolve current directory")?
        .display()
        .to_string();
    let codex_command = crate::trusted_executable::pinned_codex_command()
        .context("failed to pin a trusted `codex` executable at startup")?;
    let workspace_status = detect_workspace_status_for(Path::new(workspace_directory))?;

    Ok(LocalStartupPrerequisites {
        current_directory,
        codex_binary_detail: codex_command.source_executable.display().to_string(),
        workspace_status,
    })
}

#[derive(Clone)]
struct PromptLogMaintenance {
    port: Arc<dyn AppServerPromptLogMaintenancePort>,
    mode: AppServerPromptLogMaintenanceMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupMaintenanceWarning {
    PromptLogMaintenanceFailed,
    PromptLogMaintenancePanicked,
}

impl StartupMaintenanceWarning {
    fn operator_message(self) -> &'static str {
        match self {
            Self::PromptLogMaintenanceFailed => {
                "app-server prompt-log privacy maintenance failed; startup continued"
            }
            Self::PromptLogMaintenancePanicked => {
                "app-server prompt-log privacy maintenance panicked; startup continued"
            }
        }
    }
}

fn detect_workspace_status_for(current_directory: &Path) -> Result<WorkspaceStatus> {
    let fallback_directory = current_directory.display().to_string();

    // git이 현재 directory에서 볼 수 있는 최상위 worktree path를 요청한다.
    // stdout만 읽고 stderr는 버려, git repo가 아닌 일반 directory에서 startup 화면이 에러 로그로 오염되지 않게 한다.
    let mut command = git_subprocess::command(["rev-parse", "--show-toplevel"]);
    command
        .current_dir(current_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = subprocess::command_output(&mut command, "git rev-parse --show-toplevel");
    /*
    `git rev-parse --show-toplevel`은 현재 directory가 git worktree 안에 있을 때 canonical
    repo root를 돌려준다. stderr는 startup overlay를 어지럽히지 않도록 버리고, 실패는 아래
    fallback branch에서 directory-only workspace로 표현한다.
    */

    // 성공한 git 결과만 repo workspace로 인정한다. 명령 실행 실패, git 미설치,
    // non-zero exit은 모두 directory-only fallback으로 통일한다.
    match output {
        Ok(result) if result.status.success() => {
            // git stdout은 trailing newline을 포함하므로 trim해서 UI 표시 path로 만든다.
            let root = String::from_utf8_lossy(&result.stdout).trim().to_string();
            /*
            Empty stdout is not expected from a successful `--show-toplevel`, but
            this branch still treats the command success as the authoritative git
            signal. If git ever returns unusual output, the detail line preserves
            the raw trimmed value for diagnosis instead of inventing a path.
            */
            Ok(WorkspaceStatus {
                // git repo 안이므로 workspace check는 명확히 ok이다.
                ok: true,
                // root는 detail에도 쓰기 때문에 clone한다. 이 작은 struct에서는 복사 비용보다 명료성이 우선이다.
                path: root.clone(),
                // UI가 "git repo로 인식됨"을 명확히 보여 주는 설명이다.
                detail: format!("git repo: {root}"),
            })
        }
        // git repo가 아니어도 Akra TUI 자체는 열 수 있으므로 ok=true fallback을 반환한다.
        _ => Ok(WorkspaceStatus {
            /*
            fallback은 `ok: true`이다. 여기서 false로 두면 startup diagnostics가
            session loading이나 prompt submit을 과도하게 막을 수 있다. 대신 detail에
            "not inside a git repo"를 남겨 UI가 기능 제한의 원인을 설명하게 한다.
            */
            // hard failure가 아니라 "directory-only 모드"라는 soft state이다.
            ok: true,
            // repo root가 없으므로 현재 directory를 workspace path로 사용한다.
            path: fallback_directory,
            // UI와 로그가 왜 git root가 아닌 current directory를 쓰는지 설명하는 문구이다.
            detail: "directory only (not inside a git repo)".to_string(),
        }),
    }
}

// workspace probe의 내부 결과이다. public domain 타입으로 바로 만들지 않고 이 작은 구조로
// 중간 상태를 담으면 `detect_workspace_status`의 soft-fallback 정책을 service 내부에 가둘 수 있다.
#[derive(Clone)]
struct WorkspaceStatus {
    // startup diagnostics에 들어갈 workspace readiness flag이다.
    ok: bool,
    // git root 또는 current directory path이다.
    path: String,
    // UI 표시용 설명이다.
    detail: String,
}

#[cfg(test)]
mod tests {
    use super::{
        LocalStartupPrerequisites, StartupMaintenanceWarning, StartupService, WorkspaceStatus,
        detect_workspace_status_for,
    };
    use crate::application::port::outbound::app_server_prompt_log_port::{
        AppServerPromptLogMaintenanceMode, AppServerPromptLogMaintenancePort,
    };
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use anyhow::Result;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    const SENSITIVE_MAINTENANCE_PANIC_PAYLOAD: &str =
        "sensitive prompt contents from maintenance panic";

    fn ready_startup_context() -> AppServerStartupContext {
        AppServerStartupContext {
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_detail: "ready".to_string(),
            account_detail: "ready".to_string(),
            account_ok: true,
            warnings: Vec::new(),
        }
    }

    struct UnusedStartupProbePort;

    impl StartupProbePort for UnusedStartupProbePort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(ready_startup_context())
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StartupStep {
        Maintenance,
        LocalPrerequisites,
        StartupProbe,
    }

    struct OrderedStartupProbePort {
        steps: Arc<Mutex<Vec<StartupStep>>>,
    }

    impl StartupProbePort for OrderedStartupProbePort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            self.steps
                .lock()
                .expect("startup steps mutex should not be poisoned")
                .push(StartupStep::StartupProbe);
            Ok(ready_startup_context())
        }
    }

    struct OrderedPromptLogMaintenancePort {
        steps: Arc<Mutex<Vec<StartupStep>>>,
    }

    impl AppServerPromptLogMaintenancePort for OrderedPromptLogMaintenancePort {
        fn maintain_app_server_prompt_logs(
            &self,
            _workspace_dir: &str,
            _mode: AppServerPromptLogMaintenanceMode,
        ) -> Result<()> {
            self.steps
                .lock()
                .expect("startup steps mutex should not be poisoned")
                .push(StartupStep::Maintenance);
            Ok(())
        }
    }

    struct PanickingPromptLogMaintenancePort;

    impl AppServerPromptLogMaintenancePort for PanickingPromptLogMaintenancePort {
        fn maintain_app_server_prompt_logs(
            &self,
            _workspace_dir: &str,
            _mode: AppServerPromptLogMaintenanceMode,
        ) -> Result<()> {
            panic!("{SENSITIVE_MAINTENANCE_PANIC_PAYLOAD}");
        }
    }

    struct RecordingPromptLogMaintenancePort {
        calls: Mutex<Vec<(String, AppServerPromptLogMaintenanceMode)>>,
        result: std::result::Result<(), String>,
    }

    impl RecordingPromptLogMaintenancePort {
        fn succeeding() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Ok(()),
            }
        }

        fn failing(message: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                result: Err(message.to_string()),
            }
        }
    }

    impl AppServerPromptLogMaintenancePort for RecordingPromptLogMaintenancePort {
        fn maintain_app_server_prompt_logs(
            &self,
            workspace_dir: &str,
            mode: AppServerPromptLogMaintenanceMode,
        ) -> Result<()> {
            self.calls
                .lock()
                .expect("maintenance calls mutex should not be poisoned")
                .push((workspace_dir.to_string(), mode));
            self.result.clone().map_err(anyhow::Error::msg)
        }
    }

    #[test]
    fn workspace_status_falls_back_to_directory_only_outside_git_repo() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "akra-startup-service-non-git-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&temp_dir).expect("temp workspace should be created");

        let status = detect_workspace_status_for(&temp_dir)
            .expect("workspace status should fall back outside git repositories");

        assert!(status.ok);
        assert_eq!(status.path, temp_dir.display().to_string());
        assert_eq!(status.detail, "directory only (not inside a git repo)");

        fs::remove_dir_all(&temp_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn prompt_log_maintenance_uses_the_requested_workspace_and_typed_mode() {
        let maintenance = Arc::new(RecordingPromptLogMaintenancePort::succeeding());
        let service = StartupService::new(Arc::new(UnusedStartupProbePort))
            .with_prompt_log_maintenance(
                maintenance.clone(),
                AppServerPromptLogMaintenanceMode::PurgeExpired,
            );

        assert_eq!(
            service.maintain_prompt_logs_best_effort("/tmp/workspace-a"),
            None
        );
        assert_eq!(
            maintenance
                .calls
                .lock()
                .expect("maintenance calls mutex should not be poisoned")
                .as_slice(),
            [(
                "/tmp/workspace-a".to_string(),
                AppServerPromptLogMaintenanceMode::PurgeExpired,
            )]
        );
    }

    #[test]
    fn prompt_log_maintenance_error_becomes_a_nonfatal_redacted_warning() {
        let maintenance = Arc::new(RecordingPromptLogMaintenancePort::failing(
            "sensitive prompt contents",
        ));
        let service = StartupService::new(Arc::new(UnusedStartupProbePort))
            .with_prompt_log_maintenance(maintenance, AppServerPromptLogMaintenanceMode::ClearAll);

        let warning = service
            .maintain_prompt_logs_best_effort("/tmp/workspace-a")
            .expect("maintenance failure should become a warning");

        assert_eq!(
            warning,
            StartupMaintenanceWarning::PromptLogMaintenanceFailed
        );
        assert_eq!(
            warning.operator_message(),
            "app-server prompt-log privacy maintenance failed; startup continued"
        );
        assert!(!warning.operator_message().contains("sensitive"));
    }

    #[test]
    fn prompt_log_maintenance_runs_before_local_checks_and_startup_probe() {
        let steps = Arc::new(Mutex::new(Vec::new()));
        let service = StartupService::new(Arc::new(OrderedStartupProbePort {
            steps: steps.clone(),
        }))
        .with_prompt_log_maintenance(
            Arc::new(OrderedPromptLogMaintenancePort {
                steps: steps.clone(),
            }),
            AppServerPromptLogMaintenanceMode::PurgeExpired,
        );

        service
            .run_checks_with_local_prerequisites("/tmp/workspace-a", || {
                steps
                    .lock()
                    .expect("startup steps mutex should not be poisoned")
                    .push(StartupStep::LocalPrerequisites);
                Ok(LocalStartupPrerequisites {
                    current_directory: "/tmp/workspace-a".to_string(),
                    codex_binary_detail: "/usr/bin/codex".to_string(),
                    workspace_status: WorkspaceStatus {
                        ok: true,
                        path: "/tmp/workspace-a".to_string(),
                        detail: "git repo: /tmp/workspace-a".to_string(),
                    },
                })
            })
            .expect("ordered startup checks should succeed");

        assert_eq!(
            *steps
                .lock()
                .expect("startup steps mutex should not be poisoned"),
            [
                StartupStep::Maintenance,
                StartupStep::LocalPrerequisites,
                StartupStep::StartupProbe,
            ]
        );
    }

    #[test]
    fn prompt_log_maintenance_panic_becomes_a_nonfatal_redacted_warning() {
        let service = StartupService::new(Arc::new(UnusedStartupProbePort))
            .with_prompt_log_maintenance(
                Arc::new(PanickingPromptLogMaintenancePort),
                AppServerPromptLogMaintenanceMode::ClearAll,
            );

        let warning = service
            .maintain_prompt_logs_best_effort("/tmp/workspace-a")
            .expect("maintenance panic should become a warning");

        assert_eq!(
            warning,
            StartupMaintenanceWarning::PromptLogMaintenancePanicked
        );
        assert!(
            !warning
                .operator_message()
                .contains(SENSITIVE_MAINTENANCE_PANIC_PAYLOAD)
        );
    }
}
