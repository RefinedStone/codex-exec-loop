/*
 * adapter 계층은 바깥 세계와 application service 사이의 변환막이다.
 * inbound는 사용자나 스케줄러가 호출하는 표면이고, outbound는 service가 필요로 하는
 * 외부 시스템 접근을 실제 파일, GitHub, app-server, Telegram API로 연결한다.
 */
pub mod inbound;
pub mod outbound;
