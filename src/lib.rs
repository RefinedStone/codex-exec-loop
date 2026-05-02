// 학습 주석: library crate에서도 사용되지 않는 항목을 컴파일 오류로 막습니다.
// 이 프로젝트는 binary wrapper가 얇고 실제 조립은 library에 있으므로, dead code는 끊어진 실행 경로를 뜻할 가능성이 큽니다.
#![deny(dead_code)]

// 학습 주석: `run()`은 CLI subcommand가 stdout에 결과를 쓸 수 있도록 표준 출력 핸들을 준비합니다.
use std::io;

// 학습 주석: adapter 계층은 CLI, TUI, admin API, Telegram 같은 외부 입출력 경계를 담습니다.
pub mod adapter;
// 학습 주석: application 계층은 service와 port 계약을 통해 사용 사례를 조립합니다.
pub mod application;
// 학습 주석: domain 계층은 planning, parallel mode 같은 핵심 상태와 값 객체를 정의합니다.
pub mod domain;

// 학습 주석: 모든 binary entrypoint가 호출하는 공용 실행 함수입니다.
// 먼저 CLI subcommand를 해석하고, 처리할 subcommand가 없을 때만 기본 TUI app-server shell을 실행합니다.
pub fn run() -> anyhow::Result<i32> {
    // 학습 주석: CLI helper는 doctor/init/reset 같은 명령의 결과를 stdout에 쓰므로 mutable writer를 넘깁니다.
    let mut stdout = io::stdout();
    // 학습 주석: Some(exit_code)는 CLI subcommand가 요청을 완전히 처리했다는 뜻입니다.
    // None은 "기본 TUI를 계속 시작하라"는 신호라서 오류와 구분됩니다.
    if let Some(exit_code) = adapter::inbound::cli::run_with_env_args(&mut stdout)? {
        return Ok(exit_code);
    }

    // 학습 주석: CLI가 소비하지 않은 실행은 native-first TUI shell로 진입합니다.
    // TUI 내부에서 app-server runtime과 frontend를 조립하고, 실패하면 anyhow 오류가 binary wrapper까지 올라갑니다.
    adapter::inbound::tui::app::run()?;
    // 학습 주석: TUI가 정상 종료되면 shell에 성공 종료 코드 0을 반환합니다.
    Ok(0)
}
