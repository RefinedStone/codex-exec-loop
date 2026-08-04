/*
 * port 모듈은 application service가 바깥 세계를 직접 알지 않도록 만드는 계약층이다.
 * 여러 adapter 방향이 공유하는 application 경계 계약과 service가 요구하는 outbound
 * capability를 소유하며, 구체 service/adapter 구현 타입에는 의존하지 않는다.
 */
pub mod conversation_stream;
pub mod inbound;
pub mod outbound;
pub(crate) mod planning_task_tool_contract;
