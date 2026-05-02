/*
 * outbound adapter들은 application port를 실제 I/O로 옮기는 위치다.
 * service는 이 파일 아래 구현체 대신 port trait만 바라보므로, app-server나 GitHub 같은
 * 외부 계약 변경은 이 계층에서 흡수하는 것이 기본 방향이다.
 */
pub mod app_server;
// db는 sqlite 같은 영속 저장소 접근을 담당한다.
pub mod db;
// filesystem은 planning workspace와 로컬 파일 배치를 port 계약에 맞게 읽고 쓴다.
pub mod filesystem;
// git은 worktree, branch, merge 같은 로컬 Git 런타임 조작을 캡슐화한다.
pub mod git;
// github은 PR, review, issue 상태를 원격 GitHub API 호출로 연결한다.
pub mod github;
// telegram은 inbound bot runner가 사용할 Telegram HTTP API 경계를 구현한다.
pub mod telegram;
