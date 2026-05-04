/*
 * library crate가 실제 product wiring의 중심이다. bin wrappers는 거의 모두 여기로 진입하므로,
 * dead_code를 허용하면 "컴파일은 되지만 어떤 실행 경로에서도 연결되지 않은" adapter/service 조립이
 * 남을 수 있다. 이 crate root에서 끊어진 경계를 일찍 드러낸다.
 */
#![deny(dead_code)]

use std::io;

/*
 * adapter는 외부 입출력과 framework 의존성을 담는 가장자리다. CLI/TUI/admin API/Telegram은
 * 이쪽에서 request를 application service 호출로 바꾸고, outbound adapter는 app-server, DB,
 * filesystem 같은 실제 boundary를 port 뒤에 둔다.
 */
pub mod adapter;
// application은 use-case service와 port 계약을 둬 adapter가 domain을 직접 흔들지 않게 하는 중심층이다.
pub mod application;
// diagnostics는 TUI/app-server stdout을 오염시키지 않는 선택적 raw observability hook이다.
pub mod diagnostics;
// domain은 planning/parallel-mode의 상태 전이와 값 객체를 framework 없이 표현하는 가장 안쪽 계층이다.
pub mod domain;

/*
 * 공용 실행 함수는 native-first UX의 분기점이다. argv가 doctor/init/admin 같은 CLI command로
 * 해석되면 CLI adapter가 완료한 exit code를 돌려주고, 소비할 command가 없으면 기본 TUI
 * app-server shell을 시작한다. 그래서 bin target들은 이름이 달라도 동일한 실행 정책을 공유한다.
 */
pub fn run() -> anyhow::Result<i32> {
    diagnostics::trace_event_log::init_from_env();
    tracing::info!(
        cwd = ?std::env::current_dir().ok(),
        debug_assertions = cfg!(debug_assertions),
        arg_count = std::env::args_os().count(),
        "akra_process_started"
    );
    diagnostics::raw_event_log::emit_lazy("akra_process_started", || {
        serde_json::json!({
            "cwd": std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
            "debug_assertions": cfg!(debug_assertions),
            "arg_count": std::env::args_os().count(),
        })
    });
    /*
     * CLI adapter는 report/doctor/init 결과를 stdout에 직접 쓸 수 있어야 한다. writer를 주입하면
     * CLI parsing과 output formatting을 TUI startup과 분리해서 테스트할 수 있다.
     */
    let mut stdout = io::stdout();
    /*
     * Some(exit_code)는 CLI adapter가 요청을 완전히 소비했다는 신호다. None은 오류가 아니라
     * "interactive shell로 계속 진행"하라는 fallthrough contract라서 TUI startup과 명확히 구분된다.
     */
    if let Some(exit_code) = adapter::inbound::cli::run_with_env_args(&mut stdout)? {
        return Ok(exit_code);
    }

    /*
     * 기본 경로는 TUI inbound adapter다. 여기서 app-server runtime, shell state, terminal frontend가
     * 조립되고, 실패 context는 binary wrapper까지 올라가 stderr 출력 정책을 한곳에서 따른다.
     */
    adapter::inbound::tui::app::run()?;
    Ok(0)
}
