/*
 * inbound adapter들은 사용자가 시스템에 들어오는 표면이다. 각 모듈은 입력 형식과
 * 화면/프로토콜별 상태를 application service 호출로 바꾸며, domain 규칙을 직접 복제하지 않는다.
 */
pub mod admin_api;
// CLI는 스크립트와 운영자가 직접 실행하는 명령줄 진입점이다.
pub mod cli;
// Telegram bot은 채팅 업데이트를 planning control 명령과 응답 텍스트로 변환한다.
pub mod telegram_bot;
// TUI는 native-first 사용자 흐름의 주 표면이며 app-server 스트림과 planning 상태를 한 화면에 엮는다.
pub mod tui;
