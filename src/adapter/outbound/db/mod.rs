// 학습 주석: DB outbound adapter 영역에는 planning authority를 SQLite에 저장하고 읽는 구현이
// 들어 있습니다. 이 module declaration이 실제 adapter 파일을 `adapter::outbound::db` namespace에
// 연결합니다.
pub mod sqlite_planning_authority_adapter;

// 학습 주석: `pub use`는 wiring code가 긴 하위 경로 대신
// `adapter::outbound::db::SqlitePlanningAuthorityAdapter`로 adapter를 가져오게 하는 re-export입니다.
pub use self::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
