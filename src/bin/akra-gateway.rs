/*
 * gateway bin은 데스크톱 앱 같은 로컬 클라이언트를 codex app-server 에 중계하는 상주 데몬이다.
 * 인자 해석, 실행 디렉터리, upstream 조립은 gateway inbound adapter 가 맡고, 이 파일은 process-level
 * 오류 출력과 exit code 변환만 책임진다.
 */
#[tokio::main]
async fn main() {
    let _diagnostics_guards = codex_exec_loop_native::diagnostics::init_from_env();
    #[cfg(windows)]
    attach_windowless_console();
    tracing::info!(
        cwd = ?std::env::current_dir().ok(),
        debug_assertions = cfg!(debug_assertions),
        arg_count = std::env::args_os().count(),
        "akra_gateway_process_started"
    );
    let exit_code = match codex_exec_loop_native::adapter::inbound::gateway::run_from_env().await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(exit_code);
}

/// 데스크톱 앱은 gateway 를 콘솔 없이 띄운다. 콘솔이 없는 프로세스가 콘솔 프로그램(codex 의 npm 래퍼 `node.exe` 등)을
/// 띄우면 Windows 가 그 자식마다 새 콘솔 창을 만든다. 기본 터미널이 Windows Terminal 이면 그 창이 화면에 뜬다
/// (2026-11-06 실측: 접속마다 `CASCADIA_HOSTING_WINDOW_CLASS` 창). 시작할 때 창이 없는 콘솔을 하나 붙여 두면 자식과
/// 손자가 그 콘솔을 물려받아 새 창이 생기지 않는다. 터미널에서 실행해 이미 콘솔 창이 있으면 아무것도 하지 않는다.
/// shortcut: `AllocConsoleWithOptions` 는 Windows 11 24H2(빌드 26100) 부터 있다. 더 오래된 Windows 에서는 그대로 두므로
/// 창이 뜰 수 있다. 그때는 자식을 숨긴 콘솔로 띄우는 CreateProcess 경로를 만든다.
#[cfg(windows)]
fn attach_windowless_console() {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    #[repr(C)]
    struct AllocConsoleOptions {
        mode: i32,
        use_show_window: i32,
        show_window: u16,
    }
    type AllocConsoleWithOptions =
        unsafe extern "system" fn(*const AllocConsoleOptions, *mut i32) -> i32;
    const ALLOC_CONSOLE_MODE_NO_WINDOW: i32 = 2;

    // SAFETY: 모두 인자가 없거나 우리가 만든 값만 넘기는 Win32 호출이다. 함수 포인터는 문서의 시그니처로 바꾼다.
    unsafe {
        if !GetConsoleWindow().is_null() {
            return;
        }
        let module_name: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
        let kernel32 = GetModuleHandleW(module_name.as_ptr());
        if kernel32.is_null() {
            return;
        }
        let Some(procedure) = GetProcAddress(kernel32, c"AllocConsoleWithOptions".as_ptr().cast())
        else {
            tracing::info!("gateway_windowless_console_unavailable");
            return;
        };
        let allocate: AllocConsoleWithOptions = std::mem::transmute(procedure);
        let options = AllocConsoleOptions {
            mode: ALLOC_CONSOLE_MODE_NO_WINDOW,
            use_show_window: 0,
            show_window: 0,
        };
        let mut result = 0;
        let hresult = allocate(&options, &mut result);
        tracing::info!(hresult, result, "gateway_windowless_console");
    }
}
