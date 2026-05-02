/*
 * TUI inbound adapter는 이 native client의 주 사용자 경험을 담당한다.
 * app은 상태와 이벤트 루프를, conversation_text는 app-server 대화 표시 변환을,
 * shell_chrome은 터미널 주변 UI를 나누어 맡는다.
 */
pub mod app;
pub(crate) mod conversation_text;
pub mod shell_chrome;
