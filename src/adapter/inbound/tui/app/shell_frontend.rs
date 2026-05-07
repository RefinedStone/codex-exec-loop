use anyhow::Result;

use super::ratatui_frontend::run as run_ratatui_frontend;
use super::shell_runtime::ShellRuntime;

// Frontend mode는 terminal adapter가 선택한 viewport contract를 rendering layer에 넘기는 값이다.
// 지금은 inline main buffer만 남았지만 enum 경계를 유지해 popup layout, testkit, terminal adapter가
// 같은 mode vocabulary로 viewport assumptions를 고정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellFrontendMode {
    // ratatui frontend의 production path와 testkit이 공유하는 inline transcript/prompt layout.
    InlineMainBuffer,
}

// ShellFrontend는 shell entrypoint와 concrete ratatui loop 사이의 작은 facade다.
// 상태를 들지 않는 marker로 두어 entrypoint는 frontend 선택 지점만 소유하고, terminal IO 세부 구현은
// ratatui_frontend module 안에 남긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct ShellFrontend;

impl ShellFrontend {
    // 생성 지점을 명시해 future frontend selection이나 setup option이 생겨도 entrypoint flow가 흔들리지 않게 한다.
    pub(super) fn new() -> Self {
        Self
    }

    // test-only accessor는 facade가 약속한 viewport mode를 빠르게 고정한다.
    // production draw path는 ratatui adapter에서 같은 값을 직접 전달한다.
    #[cfg(test)]
    pub(super) fn mode(self) -> ShellFrontendMode {
        // 이 값이 바뀌면 shell_rendering과 inline terminal testkit의 layout contract도 같이 바뀐다.
        ShellFrontendMode::InlineMainBuffer
    }

    // run은 initialized ShellRuntime의 소유권을 terminal event loop로 넘기는 마지막 adapter handoff다.
    // 이후 app state mutation과 rendering cadence는 ratatui frontend가 주도한다.
    pub(super) fn run(self, runtime: ShellRuntime) -> Result<()> {
        run_ratatui_frontend(runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellFrontend, ShellFrontendMode};

    // rendering snapshot이 아니라 facade wiring policy를 고정하는 test다.
    // 실패 시 frontend mode vocabulary나 inline-only 전제가 바뀐 것으로 보고 downstream layout 호출부를 재검토한다.
    #[test]
    fn shell_frontend_is_inline_only() {
        assert_eq!(
            ShellFrontend::new().mode(),
            ShellFrontendMode::InlineMainBuffer
        );
    }
}
