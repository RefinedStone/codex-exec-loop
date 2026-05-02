// 학습 주석: frontend 실행은 terminal IO와 event loop 실패를 `anyhow::Result`로 상위 entrypoint에 돌려줍니다.
use anyhow::Result;

// 학습 주석: 실제 rendering/event loop 구현은 ratatui_frontend에 있습니다. 이 파일은 shell entrypoint가
// concrete frontend module을 직접 알지 않도록 얇은 facade 이름을 제공합니다.
use super::ratatui_frontend::run as run_ratatui_frontend;
// 학습 주석: ShellRuntime은 app state, ports, runtime services를 품은 실행 context입니다. frontend는 이
// runtime을 받아 terminal loop로 넘기고 소유권을 소비합니다.
use super::shell_runtime::ShellRuntime;

// 학습 주석: frontend mode는 shell rendering/layout code가 어느 viewport model로 그려야 하는지 알려 주는
// 작은 contract입니다. 지금은 inline main buffer만 지원하지만 enum으로 두어 이전/향후 frontend variant가
// rendering branch를 명시적으로 선택하게 합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellFrontendMode {
    // 학습 주석: InlineMainBuffer는 현재 ratatui frontend가 사용하는 기본 mode입니다. shell_rendering과
    // popup_frame은 이 값을 보고 inline transcript/prompt 영역의 높이와 footer 처리를 맞춥니다.
    InlineMainBuffer,
}

// 학습 주석: ShellFrontend는 현재 별도 field가 없는 marker/facade type입니다. 상태를 들고 있지 않기 때문에
// cheap copy/default가 가능하고, entrypoint는 concrete frontend 선택을 이 타입 뒤에 숨깁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct ShellFrontend;

// 학습 주석: impl block은 shell entrypoint가 frontend를 만들고 실행하는 public surface를 한곳에 둡니다.
impl ShellFrontend {
    // 학습 주석: constructor가 field 없는 `Self`를 반환하지만 호출 지점을 명시적으로 남겨 두면 나중에
    // frontend 선택/설정 값이 생겨도 shell_entrypoint의 구조를 크게 바꾸지 않아도 됩니다.
    pub(super) fn new() -> Self {
        Self
    }

    // 학습 주석: mode accessor는 현재 test에서만 필요합니다. production path는 ratatui adapter가 rendering
    // 호출 시 직접 InlineMainBuffer를 넘기지만, test는 frontend facade가 inline-only 정책을 지키는지 확인합니다.
    #[cfg(test)]
    pub(super) fn mode(self) -> ShellFrontendMode {
        // 학습 주석: 이 값이 바뀌면 shell rendering의 layout assumptions도 함께 검토해야 합니다.
        ShellFrontendMode::InlineMainBuffer
    }

    // 학습 주석: run은 facade에서 concrete ratatui frontend로 넘어가는 handoff입니다. runtime ownership을
    // 넘긴 뒤 terminal event loop가 app state mutation과 rendering을 모두 주도합니다.
    pub(super) fn run(self, runtime: ShellRuntime) -> Result<()> {
        // 학습 주석: 현재는 frontend 선택지가 하나뿐이므로 바로 ratatui implementation을 호출합니다.
        run_ratatui_frontend(runtime)
    }
}

// 학습 주석: frontend facade test는 rendering snapshot이 아니라 wiring policy를 고정합니다. shell은 현재
// inline main buffer mode만 지원한다는 전제를 빠르게 검증합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: test는 facade type과 mode enum만 가져와 concrete ratatui frontend를 실행하지 않습니다.
    use super::{ShellFrontend, ShellFrontendMode};

    // 학습 주석: 이 test가 실패하면 frontend mode가 추가/변경된 것이므로 shell_rendering, inline_terminal_adapter,
    // tui_testkit의 mode 전달도 함께 재검토해야 합니다.
    #[test]
    fn shell_frontend_is_inline_only() {
        assert_eq!(
            // 학습 주석: constructor로 만든 facade가 보고하는 mode를 확인합니다.
            ShellFrontend::new().mode(),
            // 학습 주석: 현재 지원되는 유일한 frontend layout mode입니다.
            ShellFrontendMode::InlineMainBuffer
        );
    }
}
