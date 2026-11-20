// automation adapter는 PR 생성, 병합, review-thread 처리처럼 GitHub에 쓰기를 수행하는 outbound boundary다.
// module declaration으로 실제 구현 파일을 GitHub adapter namespace에 포함한다.
pub mod automation;
// pr_validation adapter는 하나의 고정된 PR revision에 대한 검증 evidence를 읽는다.
pub mod pr_validation;
// review_poller adapter는 PR review/comment activity를 읽는 outbound boundary다.
// automation과 읽기 전용 polling을 별도 module로 나누어 GitHub 작업의 책임을 분리한다.
pub mod review_poller;
// resource_version adapter는 작업 컨텍스트 장부의 GitHub 문서가 바뀌었는지 `gh` 로 확인한다 (plan 22).
pub mod resource_version;
// issue_snapshot adapter는 이슈 한 건의 안정 ID 와 게시본을 `gh` 로 읽는다. 쓰지 않는다 (#2420).
pub mod issue_snapshot;
// issue_source는 열린 이슈 페이지와 안정 ID 재동기화를 읽는다 (#2537).
pub mod issue_source;
// issue_board_field adapter는 GitHub Projects 항목 하나의 필드를 `gh` 로 읽고 고친다 (#2435).
pub mod issue_board_field;
// issue_update adapter는 이슈 한 건의 제목과 본문을 `gh` 로 고친다. 아직 연결한 곳이 없다 (#2423).
pub mod issue_update;

// automation re-export는 wiring code가 하위 파일명을 직접 의존하지 않고 adapter 이름만 import하게 한다.
pub use self::automation::GithubAutomationAdapter;
pub use self::issue_board_field::GithubIssueBoardFieldAdapter;
pub use self::issue_snapshot::GithubIssueSnapshotAdapter;
pub use self::issue_update::GithubIssueUpdateAdapter;
pub use self::pr_validation::GithubPrValidationAdapter;
// review poller re-export도 같은 public surface를 제공해 composition이 port 구현체를 직접 주입하게 한다.
pub use self::resource_version::GithubResourceVersionAdapter;
pub use self::review_poller::GithubReviewPollerAdapter;

pub mod issue_create;
pub use self::issue_create::GithubIssueCreateAdapter;
