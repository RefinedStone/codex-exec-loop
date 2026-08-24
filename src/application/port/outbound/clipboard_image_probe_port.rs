use crate::domain::clipboard_image::ClipboardImageProbeOutcome;

/*
 * ClipboardImageProbePort는 OS 클립보드의 이미지 데이터를 읽어 스테이징 파일로
 * 저장하는 아웃바운드 경계다. 구현은 platform tool(osascript, PowerShell,
 * wl-paste, xclip)을 실행하는 process I/O 책임을 가지며, blocking 호출이므로
 * 호출자(composition worker)가 실행 스레드를 소유한다.
 */
pub trait ClipboardImageProbePort: Send + Sync {
    fn probe_clipboard_image(&self) -> ClipboardImageProbeOutcome;
}
