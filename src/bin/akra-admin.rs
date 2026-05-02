// 학습 주석: admin API runner는 async server를 띄우므로 Tokio runtime이 필요합니다.
// 이 속성이 async main을 동기식 프로세스 진입점으로 감싸 줍니다.
#[tokio::main]
async fn main() {
    // 학습 주석: admin API는 환경변수에서 설정을 읽어 inbound adapter를 실행합니다.
    // 성공은 code 0, 설정/서버 오류는 chain을 출력한 뒤 code 1로 바꿉니다.
    let exit_code = match codex_exec_loop_native::adapter::inbound::admin_api::run_from_env().await
    {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // 학습 주석: async main 내부에서도 마지막에는 명시적인 process exit으로 shell과 supervisor가 실패를 감지하게 합니다.
    std::process::exit(exit_code);
}
