/*
 * application 계층은 도메인 규칙을 실제 유스케이스로 엮는 중심부다.
 * port는 service가 기대하는 외부 계약을 먼저 정의하고, service는 그 계약과 domain 모델을
 * 조합해 TUI, CLI, Telegram 같은 adapter가 호출할 수 있는 작업 단위를 제공한다.
 */
pub mod port;
pub mod service;
