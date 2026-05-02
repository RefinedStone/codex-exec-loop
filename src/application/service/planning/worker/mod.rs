/*
 * planning worker service는 queue에 쌓인 작업을 실제 실행 lane으로 보내는 조율 계층이다.
 * orchestration 하위 모듈은 worker port와 planning 상태 갱신을 엮어 한 작업 단위의 생애주기를 관리한다.
 */
pub(crate) mod orchestration;
