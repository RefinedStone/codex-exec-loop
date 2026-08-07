// automation adapter는 PR 생성, 병합, review-thread 처리처럼 GitHub에 쓰기를 수행하는 outbound boundary다.
// module declaration으로 실제 구현 파일을 GitHub adapter namespace에 포함한다.
pub mod automation;
// pr_validation adapter는 하나의 고정된 PR revision에 대한 검증 evidence를 읽는다.
pub mod pr_validation;
// review_poller adapter는 PR review/comment activity를 읽는 outbound boundary다.
// automation과 읽기 전용 polling을 별도 module로 나누어 GitHub 작업의 책임을 분리한다.
pub mod review_poller;

// automation re-export는 wiring code가 하위 파일명을 직접 의존하지 않고 adapter 이름만 import하게 한다.
pub use self::automation::GithubAutomationAdapter;
pub use self::pr_validation::GithubPrValidationAdapter;
// review poller re-export도 같은 public surface를 제공해 composition이 port 구현체를 직접 주입하게 한다.
pub use self::review_poller::GithubReviewPollerAdapter;
