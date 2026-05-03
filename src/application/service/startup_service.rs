// startup check는 git workspace를 확인하기 위해 짧은 외부 명령을 실행한다.
// `Command`는 `git rev-parse`를 호출하는 도구이고, `Stdio`는 startup overlay에 불필요한 stderr가 섞이지 않게 제어한다.
use std::process::{Command, Stdio};
// `Arc`는 TUI app runtime과 background startup task가 같은 startup probe adapter를 공유하게 한다.
use std::sync::Arc;

// `Context`는 low-level 오류에 "무엇을 하다 실패했는지"를 덧붙인다.
// startup 실패는 첫 화면에 바로 노출되므로, 단순 io error보다 사용자가 이해할 수 있는 문맥이 중요하다.
use anyhow::{Context, Result};

// startup probe port는 app-server 쪽 account/init/attachment 상태를 읽는 outbound 경계이다.
// 이 service는 app-server JSON이나 connection lifecycle을 모르고, port가 정규화한 startup context만 받는다.
use crate::application::port::outbound::startup_probe_port::StartupProbePort;
// `StartupDiagnostics`는 startup overlay, prompt submit gating, recent-session loading gating이
// 공통으로 읽는 domain snapshot이다.
use crate::domain::startup_diagnostics::StartupDiagnostics;

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
        Self { startup_probe_port }
    }

    // startup overlay에 필요한 전체 diagnostics를 한 번 수집한다.
    // 이 함수의 성공/실패는 `AppRuntime`의 background message로 돌아가 `StartupState::Ready` 또는
    // `StartupState::Failed`로 줄어든다.
    pub fn run_checks(&self) -> Result<StartupDiagnostics> {
        /*
        run_checks는 startup overlay의 한 번짜리 readiness snapshot을 만든다. 실패하면
        TUI는 StartupState::Failed로 들어가고, 성공하면 `StartupDiagnostics::can_continue()` 같은
        domain 판단으로 prompt submit, session overlay, warning line을 제어한다.
        */
        /*
        The ordering is intentional: local prerequisites are checked before the
        app-server probe so obvious environment problems fail with local context, while
        app-server account/initialize details are merged only after the process can
        plausibly launch the Codex binary from this workspace.
        */
        // 현재 directory는 diagnostics의 기본 위치 표시값이다.
        // 여기서 실패하면 실행 환경 자체를 알 수 없으므로 startup check 전체를 실패로 돌린다.
        let current_directory = std::env::current_dir()
            .context("failed to resolve current directory")?
            .display()
            .to_string();

        // `codex` binary는 native client가 app-server flow를 시작할 수 있는 최소 실행 의존성이다.
        // PATH에서 찾지 못하면 이후 turn execution이 성립하지 않으므로 hard failure로 처리한다.
        let codex_path = which::which("codex").context("`codex` was not found on PATH")?;
        /*
        `codex` binary는 native TUI가 실제 turn execution/app-server flow와 연결될 수 있는지
        보는 가장 기본적인 local prerequisite이다. 여기서 실패하면 diagnostics object를 만들지 않고
        오류로 올려 startup state 자체를 Failed로 전환한다.
        */
        // workspace 확인은 soft readiness 항목이다. git repo root를 찾으면 detail에 표시하고,
        // 아니면 현재 directory를 workspace처럼 표시하되 startup 자체는 계속 진행한다.
        let workspace_status = self.detect_workspace_status()?;

        // app-server startup context는 outbound adapter가 initialize/probe 요청을 수행한 결과이다.
        // account warning이나 attachment profile은 local process check만으로는 얻을 수 없다.
        /*
        This port call is the first network/process boundary in the check. Keeping it
        after local workspace detection lets diagnostics distinguish "we could not run
        local prerequisites" from "app-server/account probing failed" without blending
        both classes into one generic startup error.
        */
        let startup_context = self.startup_probe_port.load_startup_context()?;
        /*
        app-server startup context는 local shell에서 직접 알 수 없는 account/login 상태와
        attachment profile, initialize detail을 보완한다. local checks와 port checks를 같은
        diagnostics에 담아 rendering layer가 하나의 startup overlay로 표시할 수 있게 한다.
        */

        // local check와 app-server check를 하나의 domain snapshot으로 합친다.
        // 이후 TUI rendering은 이 구조체만 보고 startup banner, warning, action availability를 계산한다.
        Ok(StartupDiagnostics {
            // 현재 프로세스가 시작된 directory이다. git root가 아니어도 사용자가 위치를 확인할 수 있게 남긴다.
            cwd: current_directory,
            // 여기까지 도달했다면 `codex` binary lookup은 성공한 상태이다.
            codex_binary_ok: true,
            // UI에는 단순 ok뿐 아니라 실제 발견된 binary path를 보여 줘 PATH 문제를 디버깅하게 한다.
            codex_binary_detail: codex_path.display().to_string(),
            // 현재 정책상 workspace는 git repo가 아니어도 ok이다. detail이 기능 제한 설명을 담당한다.
            workspace_ok: workspace_status.ok,
            // git root를 찾으면 repo root, 아니면 current directory가 들어간다.
            workspace_path: workspace_status.path,
            // "git repo: ..." 또는 "directory only ..." 같은 사람이 읽는 설명이다.
            workspace_detail: workspace_status.detail,
            // app-server launch/reattach 상태를 startup 화면에서 같은 attachment vocabulary로 보여 준다.
            attachment_profile: startup_context.attachment_profile,
            // startup context load가 성공했으므로 initialize probe는 성공으로 표시한다.
            initialize_ok: true,
            // app-server가 돌려준 initialize 설명이다. rendering layer는 이 값을 그대로 summary에 노출한다.
            initialize_detail: startup_context.initialize_detail,
            // account 상태는 prompt submit 가능 여부를 좌우하는 핵심 readiness 축이다.
            account_ok: startup_context.account_ok,
            // 계정 상태의 사람이 읽는 설명이다. 예를 들어 login 필요 같은 안내가 들어간다.
            account_detail: startup_context.account_detail,
            // blocking failure는 아니지만 startup overlay와 warning line에 보여야 하는 app-server 경고들이다.
            warnings: startup_context.warnings,
            // binary에 포함된 schema snapshot label이다. runtime schema mismatch를 볼 때 baseline이 된다.
            schema_snapshot: StartupDiagnostics::bundled_schema_snapshot_label(),
        })
    }

    // 현재 directory가 git workspace인지 판정한다. 이 함수는 startup readiness의
    // "workspace 표시 정보"를 만들 뿐, git repo가 아니라고 전체 startup을 실패시키지 않는다.
    fn detect_workspace_status(&self) -> Result<WorkspaceStatus> {
        /*
        workspace status는 git repository 안에서 실행 중인지 확인하되, git repo가 아니어도
        fatal startup failure로 보지 않는다. Akra는 일반 directory에서도 shell을 띄울 수 있고,
        이후 일부 기능만 제한하거나 workspace path를 현재 directory로 표시하면 된다.
        */
        /*
        This helper returns a service-private WorkspaceStatus rather than
        StartupDiagnostics fields directly. That keeps the soft fallback policy local:
        git discovery may fail, but the caller still receives a normalized path/detail
        pair that can be merged with app-server readiness.
        */
        // git 판정 실패 시 fallback path로 쓸 현재 directory이다.
        let current_directory = std::env::current_dir()
            .context("failed to resolve current directory for workspace status")?
            .display()
            .to_string();

        // git이 현재 directory에서 볼 수 있는 최상위 worktree path를 요청한다.
        // stdout만 읽고 stderr는 버려, git repo가 아닌 일반 directory에서 startup 화면이 에러 로그로 오염되지 않게 한다.
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
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
                path: current_directory,
                // UI와 로그가 왜 git root가 아닌 current directory를 쓰는지 설명하는 문구이다.
                detail: "directory only (not inside a git repo)".to_string(),
            }),
        }
    }
}

// workspace probe의 내부 결과이다. public domain 타입으로 바로 만들지 않고 이 작은 구조로
// 중간 상태를 담으면 `detect_workspace_status`의 soft-fallback 정책을 service 내부에 가둘 수 있다.
struct WorkspaceStatus {
    // startup diagnostics에 들어갈 workspace readiness flag이다.
    ok: bool,
    // git root 또는 current directory path이다.
    path: String,
    // UI 표시용 설명이다.
    detail: String,
}
