/*
 * port 모듈은 application service가 바깥 세계를 직접 알지 않도록 만드는 계약층이다.
 * 현재는 service가 호출하는 outbound 경계만 공개하며, 실제 구현은 adapter/outbound에서
 * 이 trait들을 만족시키는 방식으로 붙는다.
 */
pub mod outbound;
