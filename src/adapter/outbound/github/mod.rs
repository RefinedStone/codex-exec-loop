// 학습 주석: automation adapter는 PR 생성, 병합, review-thread 처리처럼 GitHub에 쓰기를 수행하는
// outbound boundary입니다. module declaration으로 실제 구현 파일을 GitHub adapter namespace에
// 포함합니다.
pub mod automation;
// 학습 주석: review_poller adapter는 PR review/comment activity를 읽는 outbound boundary입니다.
// automation과 읽기 전용 polling을 별도 module로 나누어 GitHub 작업의 책임을 분리합니다.
pub mod review_poller;

// 학습 주석: automation re-export는 wiring code가 하위 파일명을 직접 의존하지 않고
// `adapter::outbound::github::GithubAutomationAdapter`만 import하게 해 줍니다.
pub use self::automation::GithubAutomationAdapter;
// 학습 주석: review poller re-export도 같은 public surface를 제공합니다. application composition은
// 이 이름으로 GitHub review polling port 구현체를 주입합니다.
pub use self::review_poller::GithubReviewPollerAdapter;
