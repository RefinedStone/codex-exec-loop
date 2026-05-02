// 학습 주석: service는 outbound GitHub poller adapter를 여러 caller가 공유할 수 있도록 Arc로 보관합니다.
use std::sync::Arc;

// 학습 주석: GitHub polling은 네트워크/인증/파싱 실패를 그대로 application 경계로 올리므로 anyhow Result를 사용합니다.
use anyhow::Result;

// 학습 주석: 실제 GitHub 접근은 outbound port 뒤에 숨겨 service가 API client 세부사항을 모르게 합니다.
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
// 학습 주석: domain 타입은 PR activity snapshot, poll cursor, poll result의 공용 언어입니다.
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivitySnapshot, GithubPullRequestPollResult,
    GithubPullRequestPollState, GithubPullRequestTarget,
};

// 학습 주석: service clone은 Arc clone만 수행하므로 TUI refresh loop와 background worker가 같은 poller를 공유할 수 있습니다.
#[derive(Clone)]
/*
학습 주석: GithubReviewPollerService는 application layer에서 GitHub PR activity polling을
조율하는 작은 facade입니다. 실제 HTTP 호출, pagination, JSON mapping은 outbound adapter가
`GithubReviewPollerPort`로 숨기고, 이 service는 "정렬된 activity snapshot을 만들고 이전 poll
state와 비교해 새 이벤트만 고른다"는 use-case 규칙만 담당합니다.

이 분리는 TUI나 worker가 GitHub API response shape를 알 필요 없이 domain snapshot/result만
다루게 해 줍니다. 또한 tests는 fake port로 snapshot을 주입해 cursor diff 정책만 검증할 수
있습니다.
*/
pub struct GithubReviewPollerService {
    // 학습 주석: GitHub PR activity를 읽는 outbound port입니다. service는 이 port가 돌려준 snapshot을 정렬/비교만 합니다.
    github_review_poller_port: Arc<dyn GithubReviewPollerPort>,
}

// 학습 주석: 이 impl은 GitHub activity polling의 use-case 규칙을 담습니다.
// adapter가 가져온 raw snapshot을 안정적으로 정렬하고 이전 poll state와 비교해 새 change만 추려냅니다.
impl GithubReviewPollerService {
    // 학습 주석: outbound port를 주입해 poller service를 만듭니다.
    pub fn new(github_review_poller_port: Arc<dyn GithubReviewPollerPort>) -> Self {
        /*
        학습 주석: service는 outbound port를 Arc로 보관합니다. polling은 UI refresh나 background
        worker처럼 여러 owner에서 호출될 수 있으므로 clone 가능한 service가 필요하고, application
        layer는 concrete GitHub adapter 타입을 직접 소유하지 않습니다.
        */
        Self {
            github_review_poller_port,
        }
    }

    // 학습 주석: target PR의 현재 activity snapshot을 읽고 service 경계에서 정렬합니다.
    pub fn load_snapshot(
        &self,
        // 학습 주석: owner/repo/PR 번호 등 polling 대상 PR을 나타내는 domain 값입니다.
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot> {
        /*
        학습 주석: load_snapshot은 "현재 PR activity 전체를 domain snapshot으로 읽는다"는
        read-only 경로입니다. port가 가져온 event 순서는 GitHub endpoint별 pagination이나 review,
        review comment, issue comment 병합 순서에 영향을 받을 수 있으므로, service 경계에서 항상
        정렬해 이후 diff logic이 시간순 snapshot을 전제로 움직이게 합니다.
        */
        // 학습 주석: port가 읽은 snapshot은 GitHub API 호출 순서의 영향을 받을 수 있으므로 mutable로 받아 정렬합니다.
        let mut snapshot = self
            .github_review_poller_port
            .load_pull_request_activity(target)?;
        // 학습 주석: diff cursor는 event 순서를 전제로 하므로, service가 모든 caller에게 정렬된 snapshot만 제공합니다.
        snapshot.sort_events();
        Ok(snapshot)
    }

    // 학습 주석: 현재 snapshot과 이전 cursor를 비교해 이번 poll에서 새로 보여줄 change만 계산합니다.
    pub fn poll(
        &self,
        // 학습 주석: polling 대상 PR입니다.
        target: &GithubPullRequestTarget,
        // 학습 주석: 직전 poll이 남긴 cursor입니다. 없으면 baseline만 세우고 기존 활동을 재알림하지 않습니다.
        previous_state: Option<&GithubPullRequestPollState>,
    ) -> Result<GithubPullRequestPollResult> {
        /*
        학습 주석: poll은 snapshot load, cursor diff, next cursor 생성 세 단계를 묶습니다. 첫 poll은
        baseline만 세우고 기존 activity를 "새 알림"으로 재생하지 않습니다. 이후 poll은 이전 cursor
        이후의 event만 changes에 넣어 TUI나 notification layer가 중복 알림을 내지 않게 합니다.
        */
        // 학습 주석: 항상 정렬된 snapshot을 먼저 확보한 뒤 diff를 수행합니다.
        let snapshot = self.load_snapshot(target)?;
        // 학습 주석: 이전 cursor가 본 마지막 timestamp/identity 이후의 event만 changes로 추립니다.
        let changes = Self::collect_changes(&snapshot.events, previous_state);
        // 학습 주석: next_state는 changes가 아니라 전체 snapshot에서 만들어, 새 event가 없는 poll도 cursor를 보존합니다.
        let next_state = GithubPullRequestPollState::from_snapshot(&snapshot);
        /*
        학습 주석: next_state는 snapshot 전체에서 다시 계산합니다. changes만 기준으로 cursor를
        만들면 event가 없는 poll에서 state가 비거나, 같은 timestamp에 여러 event가 들어온 경우
        어떤 event를 이미 봤는지 잃을 수 있습니다. snapshot 기반 state는 "이번 poll 시점까지 본
        전체 활동"을 정확히 표현합니다.
        */

        // 학습 주석: caller는 snapshot으로 현재 전체 상태를 그리고, changes로 알림/notice만 추가로 처리합니다.
        Ok(GithubPullRequestPollResult {
            snapshot,
            changes,
            next_state,
        })
    }

    // 학습 주석: `collect_changes`는 정렬된 현재 activity와 직전 poll cursor를 비교해
    // UI나 상위 application layer가 "이번 poll에서 새로 본 GitHub 변화"만 처리하도록 돕는
    // 내부 diff helper입니다.
    fn collect_changes(
        // 학습 주석: `events`는 방금 `load_snapshot`이 GitHub PR에서 읽어 온 review/comment
        // activity입니다. 이 함수는 slice로 빌려 읽기 때문에 caller의 snapshot ownership을
        // 가져오지 않고, 새 변화로 판정된 항목만 마지막에 복제합니다.
        events: &[GithubPullRequestActivityEvent],
        // 학습 주석: `previous_state`는 직전 poll에서 저장한 timestamp cursor와 같은 timestamp
        // bucket의 event identity 집합입니다. 값이 없으면 아직 baseline을 세우지 않은 첫 poll입니다.
        previous_state: Option<&GithubPullRequestPollState>,
    ) -> Vec<GithubPullRequestActivityEvent> {
        /*
        학습 주석: change collection은 timestamp cursor와 identity set을 함께 씁니다. timestamp만
        쓰면 같은 초/밀리초에 여러 review comment가 추가될 때 마지막 timestamp의 일부 event를
        놓칠 수 있습니다. 반대로 identity set만 쓰면 오래된 event까지 매 poll마다 비교해야 하므로,
        latest timestamp를 큰 경계로 두고 같은 timestamp 안에서만 identity로 중복을 제거합니다.
        */
        // 학습 주석: 첫 poll은 "변화 탐지"가 아니라 "현재 GitHub 상태를 기준점으로 저장"하는
        // 단계입니다. 여기서 과거 event를 모두 반환하지 않아야 사용자가 이미 지나간 review를
        // 새 알림처럼 받지 않습니다.
        let Some(previous_state) = previous_state else {
            /*
            학습 주석: previous_state가 없으면 첫 poll입니다. 이때 기존 GitHub activity를 모두
            changes로 내보내면 사용자가 이미 본 과거 review/comment가 새 알림처럼 쏟아집니다.
            따라서 첫 poll은 baseline establishment로만 동작합니다.
            */
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Vec::new();
        };

        // 학습 주석: 이전 poll state는 있었지만 latest timestamp가 없다면 직전 snapshot에는
        // activity가 하나도 없었습니다. 이제 보이는 현재 event들은 모두 비어 있던 기준점 이후에
        // 처음 발견된 변화로 다루어야 합니다.
        let Some(latest_submitted_at) = previous_state.latest_submitted_at.as_ref() else {
            /*
            학습 주석: 이전 state가 있지만 latest timestamp가 없다는 것은 과거 snapshot이 비어
            있었다는 뜻입니다. 이제 처음 activity가 나타났다면 모두 새 변화로 보여 줘야 하므로
            현재 events 전체를 반환합니다.
            */
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return events.to_vec();
        };

        events
            // 학습 주석: 현재 snapshot의 각 event를 순회하며 cursor와 비교합니다. snapshot 생성부가
            // 정렬을 담당하므로 여기서는 순서를 다시 만들지 않고 포함 여부만 판단합니다.
            .iter()
            // 학습 주석: filter closure는 "직전 latest timestamp보다 늦은 event"와 "같은 timestamp지만
            // 직전 latest bucket에는 없던 event"만 통과시킵니다. 이 두 조건이 timestamp cursor와
            // identity set을 하나의 증분 diff 규칙으로 묶습니다.
            .filter(|event| {
                /*
                학습 주석: timestamp가 cursor보다 크면 명백한 새 event입니다. timestamp가 같으면
                이전 poll의 latest timestamp bucket에 없던 identity만 새 event로 봅니다. identity는
                event kind와 id를 묶은 domain key라서 review와 review comment의 id 공간이 겹쳐도
                안전하게 구분됩니다.
                */
                event.submitted_at > *latest_submitted_at
                    // 학습 주석: 같은 timestamp의 event는 시간만으로 새/기존을 나눌 수 없습니다. 그래서
                    // 직전 state가 기억한 identity set에 없을 때만 새 activity로 인정합니다.
                    || (event.submitted_at == *latest_submitted_at
                        && !previous_state
                            // 학습 주석: `seen_events_at_latest_timestamp`는 직전 poll의 맨 끝 timestamp에
                            // 함께 있던 event identity만 담습니다. cursor 경계에서만 쓰이는 좁은
                            // 중복 제거 장치라 오래된 모든 event를 저장할 필요가 없습니다.
                            .seen_events_at_latest_timestamp
                            // 학습 주석: `identity()`는 review와 review comment를 구분하는 kind까지 포함한
                            // key를 만듭니다. GitHub id 공간이 종류별로 겹쳐도 contains 검사가
                            // 잘못된 중복 판정을 하지 않게 하는 연결점입니다.
                            .contains(&event.identity()))
            })
            // 학습 주석: iterator가 들고 있는 것은 `events` slice 안의 참조입니다. 반환 타입은 소유한
            // event vector이므로, 통과한 event를 복제해 caller가 독립적으로 changes를 보관하게 합니다.
            .cloned()
            // 학습 주석: collect는 통과한 owned event들을 하나의 `Vec`으로 모아 poll result의
            // `changes` 필드에 바로 넣을 수 있는 형태로 마무리합니다.
            .collect()
    }
}

// 학습 주석: 이 테스트 모듈은 production build에는 들어가지 않고 `cargo test`에서만 컴파일됩니다.
// service의 public behavior를 같은 파일 옆에서 검증해 private helper인 `collect_changes`까지
// 간접적으로 안전하게 고정합니다.
#[cfg(test)]
// 학습 주석: `tests` 모듈은 GitHub review poller service의 test-only fixture, fake port,
// behavior test를 한곳에 묶습니다. production module namespace와 분리되므로 helper 이름이
// application code와 충돌하지 않습니다.
mod tests {
    // 학습 주석: `Arc`는 fake outbound port를 service에 주입할 때 shared ownership을 맞추기
    // 위한 표준 포인터입니다. production code와 같은 생성 경로를 쓰면 테스트가 DI contract까지
    // 함께 검증합니다.
    use std::sync::Arc;

    // 학습 주석: fake port 구현도 실제 port trait과 같은 `anyhow::Result` 반환 계약을 따릅니다.
    // 이렇게 해야 테스트 double이 실패 가능성을 감춘 별도 API가 아니라 outbound boundary의
    // 실제 호출 모양을 그대로 대체합니다.
    use anyhow::Result;

    // 학습 주석: 테스트 대상은 service 자체입니다. 아래 test들은 adapter나 GitHub API 없이
    // `GithubReviewPollerService::poll`의 snapshot 정렬, diff, next_state 조립만 확인합니다.
    use super::GithubReviewPollerService;
    // 학습 주석: outbound port trait을 가져오는 이유는 fake가 service 바깥의 GitHub 읽기 경계를
    // 흉내 내기 위해서입니다. service는 concrete adapter가 아니라 이 trait만 의존합니다.
    use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
    // 학습 주석: domain type import는 테스트 fixture가 production path와 같은 target, event,
    // snapshot, poll state 구조를 만들게 해 줍니다. test-only DTO를 만들지 않기 때문에 domain
    // contract drift를 더 빨리 발견할 수 있습니다.
    use crate::domain::github_review::{
        GithubPullRequestActivityEvent, GithubPullRequestActivityKind,
        GithubPullRequestActivitySnapshot, GithubPullRequestPollState, GithubPullRequestTarget,
    };

    // 학습 주석: `FakeGithubReviewPollerPort`는 outbound GitHub adapter 대신 service에 주입되는
    // in-memory test double입니다. 포트 경계만 대체하고 service 로직은 실제 경로 그대로 실행하게
    // 만드는 것이 이 구조체의 역할입니다.
    struct FakeGithubReviewPollerPort {
        // 학습 주석: fake는 한 번의 poll에서 반환할 snapshot만 들고 있습니다. 테스트별로 이 값을
        // 다르게 구성하면 GitHub 네트워크 호출 없이 service가 받은 activity 목록을 어떻게
        // 해석하는지만 좁게 검증할 수 있습니다.
        snapshot: GithubPullRequestActivitySnapshot,
    }

    // 학습 주석: 이 impl은 fake를 실제 `GithubReviewPollerPort`로 취급할 수 있게 합니다. service가
    // trait object를 받기 때문에 테스트도 production DI와 같은 추상화 수준에서 동작합니다.
    impl GithubReviewPollerPort for FakeGithubReviewPollerPort {
        // 학습 주석: `load_pull_request_activity` fake 구현은 입력 target을 따로 검증하지 않고
        // 준비된 snapshot을 돌려줍니다. 이 테스트들의 관심사는 target validation이 아니라
        // service가 port 결과를 정렬하고 cursor와 비교하는 application 흐름입니다.
        fn load_pull_request_activity(
            &self,
            // 학습 주석: `_target`처럼 밑줄로 시작하는 이름은 이 테스트 double에서 target 값을
            // 의도적으로 사용하지 않는다는 신호입니다. trait signature는 유지하면서 clippy의
            // unused 경고도 피합니다.
            _target: &GithubPullRequestTarget,
        ) -> Result<GithubPullRequestActivitySnapshot> {
            // 학습 주석: snapshot을 clone해서 반환하면 fake가 가진 fixture를 소비하지 않습니다.
            // 같은 port instance가 여러 poll에서 재사용되어도 테스트 데이터가 그대로 남습니다.
            Ok(self.snapshot.clone())
        }
    }

    // 학습 주석: `#[test]`는 이 함수가 첫 poll baseline 정책을 고정하는 독립 테스트임을 test runner에
    // 등록합니다. 실패하면 review poller가 과거 GitHub activity를 새 변화로 replay할 위험이 있습니다.
    #[test]
    // 학습 주석: 이 테스트는 `previous_state`가 없는 첫 poll의 핵심 정책을 고정합니다. service는
    // GitHub에서 이미 존재하던 activity를 changes로 replay하지 않고, snapshot에서 next_state만
    // 만들어 이후 poll의 기준점으로 삼아야 합니다.
    fn first_poll_establishes_baseline_without_replaying_existing_activity() {
        // 학습 주석: target fixture는 PR 단위 polling이 항상 repository와 PR number를 기준으로
        // 이루어진다는 domain contract를 보여 줍니다.
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        // 학습 주석: service fixture에는 이미 두 개의 activity가 있는 snapshot을 넣습니다. 첫 poll에서
        // 이 두 event가 changes로 나오면 baseline 정책이 깨진 것입니다.
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            // 학습 주석: snapshot helper는 PR metadata와 event vector를 한 번에 묶어 service가
            // 실제 port에서 받을 형태와 같은 객체를 만듭니다.
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        100,
                        // 학습 주석: 첫 event는 issue comment입니다. kind가 섞여 있어도 첫 poll은
                        // 종류와 무관하게 모두 baseline에만 반영해야 합니다.
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T09:00:00Z",
                    ),
                    event(
                        101,
                        // 학습 주석: 두 번째 event는 review입니다. 최신 timestamp가 poll_state cursor로
                        // 저장되는지 확인하기 위한 뒤쪽 activity 역할을 합니다.
                        GithubPullRequestActivityKind::Review,
                        "2026-04-08T10:00:00Z",
                    ),
                ],
            ),
        }));

        // 학습 주석: `None`은 "직전 poll state가 없다"는 입력입니다. service는 이를 첫 poll로
        // 해석하고 replay 방지 branch를 타야 합니다.
        let result = service.poll(&target, None).expect("poll should succeed");

        // 학습 주석: 첫 poll의 changes가 비어 있어야 사용자가 기존 GitHub 대화를 새 알림처럼 받지
        // 않습니다. next_state는 snapshot에서 계산한 poll_state와 같아야 다음 poll이 같은 기준점을
        // 이어받습니다.
        assert!(result.changes.is_empty());
        assert_eq!(result.next_state, result.snapshot.poll_state());
    }

    // 학습 주석: 이 테스트는 이미 cursor가 있는 일반 poll에서 service가 과거 activity를 버리고
    // timestamp cursor 뒤의 event만 changes로 노출하는지 확인합니다.
    #[test]
    // 학습 주석: 이전 state가 `10:00` review를 마지막으로 기억할 때, `10:30`과 `11:00` event만
    // 새 변화가 되어야 합니다. 이는 매 poll마다 같은 review 알림을 반복하지 않게 하는 핵심 규칙입니다.
    fn poll_returns_only_events_after_previous_cursor() {
        // 학습 주석: 테스트 target은 fake snapshot과 poll 호출에 같은 PR identity를 전달해
        // service가 하나의 PR stream을 diff한다는 전제를 고정합니다.
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        // 학습 주석: snapshot에는 cursor 전, cursor 시점, cursor 이후 event를 모두 넣습니다. 이렇게
        // 섞어 두어야 filter가 실제로 boundary를 기준으로 잘라내는지 볼 수 있습니다.
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            // 학습 주석: fixture snapshot은 시간 순서대로 넣어 diff 기대값을 사람이 읽기 쉽게 만듭니다.
            // 정렬 책임은 다음 테스트에서 별도로 검증합니다.
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        100,
                        // 학습 주석: 이 issue comment는 cursor보다 오래된 activity입니다. changes에
                        // 포함되면 오래된 GitHub 대화를 다시 알리는 회귀입니다.
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T09:00:00Z",
                    ),
                    event(
                        101,
                        // 학습 주석: 이 review는 previous_state의 latest timestamp와 같은 event입니다.
                        // identity set에 들어 있으므로 새 변화가 아니어야 합니다.
                        GithubPullRequestActivityKind::Review,
                        "2026-04-08T10:00:00Z",
                    ),
                    event(
                        201,
                        // 학습 주석: 이 review comment는 cursor 이후 첫 새 activity입니다. service가
                        // 반환하는 changes의 첫 항목이 되어야 합니다.
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    ),
                    event(
                        202,
                        // 학습 주석: 이 issue comment도 cursor 이후 activity입니다. kind가 달라도
                        // timestamp가 뒤라면 changes에 포함되어야 합니다.
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T11:00:00Z",
                    ),
                ],
            ),
        }));
        // 학습 주석: previous_state는 직전 poll이 `10:00` review까지 봤다는 cursor를 표현합니다.
        // latest timestamp와 그 timestamp 안의 seen identity를 같이 저장하는 구조가 diff의 기준입니다.
        let previous_state = GithubPullRequestPollState {
            // 학습 주석: latest timestamp는 "이 시간보다 뒤는 새 event 후보"라는 큰 경계입니다.
            latest_submitted_at: Some("2026-04-08T10:00:00Z".to_string()),
            // 학습 주석: 같은 timestamp 안에서는 identity set으로 이미 본 event를 제외합니다.
            seen_events_at_latest_timestamp: vec![
                event(
                    101,
                    // 학습 주석: 이 identity가 들어 있기 때문에 현재 snapshot의 id 101 review는
                    // cursor와 timestamp가 같아도 changes에서 빠져야 합니다.
                    GithubPullRequestActivityKind::Review,
                    "2026-04-08T10:00:00Z",
                )
                // 학습 주석: `identity()`는 event fixture에서 diff key만 뽑아 state에 저장합니다.
                // state가 전체 event body를 들지 않아도 중복 판정이 가능해지는 지점입니다.
                .identity(),
            ],
        };

        // 학습 주석: 이전 state를 넘겨 poll하면 service는 first-poll baseline branch가 아니라
        // cursor diff branch를 실행합니다.
        let result = service
            // 학습 주석: `Some(&previous_state)`는 state ownership을 넘기지 않고 읽기 전용 cursor만
            // 빌려 줍니다. caller는 poll 이후에도 이전 state를 비교나 logging에 사용할 수 있습니다.
            .poll(&target, Some(&previous_state))
            // 학습 주석: 이 테스트는 fake port가 성공하도록 만든 happy path이므로 실패하면 service
            // orchestration이나 fixture 구성 자체가 깨진 것입니다.
            .expect("poll should succeed");

        // 학습 주석: changes에는 cursor 이후의 id 201, 202만 남아야 합니다. 길이와 순서를 함께
        // 확인해 과거 event replay와 정렬 회귀를 동시에 잡습니다.
        assert_eq!(result.changes.len(), 2);
        assert_eq!(result.changes[0].id, 201);
        assert_eq!(result.changes[1].id, 202);
    }

    // 학습 주석: 이 테스트는 outbound port가 event를 시간순으로 보장하지 않아도 service가 먼저
    // snapshot을 정렬한 뒤 diff한다는 application 책임을 고정합니다.
    #[test]
    // 학습 주석: GitHub API나 adapter 구현이 순서를 바꿔 반환해도 UI에 전달되는 snapshot과 changes는
    // 시간 오름차순이어야 사용자가 review 흐름을 자연스럽게 읽을 수 있습니다.
    fn poll_sorts_unsorted_port_responses_before_diffing() {
        // 학습 주석: 같은 PR target을 사용해 정렬 검증이 target routing 문제가 아니라 event ordering
        // 문제에만 집중하게 합니다.
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        // 학습 주석: fake snapshot은 의도적으로 최신 event를 먼저 넣습니다. service가 정렬하지 않으면
        // result.snapshot.events와 changes 순서 assertion이 실패합니다.
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            // 학습 주석: port boundary에서 들어온 원본 순서가 불안정할 수 있다는 현실을 fixture에
            // 반영해 service의 방어 정렬을 검증합니다.
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        301,
                        // 학습 주석: 최신 review를 맨 앞에 놓아, 정렬이 없으면 snapshot 첫 항목이
                        // 잘못된 최신 event로 남게 만듭니다.
                        GithubPullRequestActivityKind::Review,
                        "2026-04-08T12:00:00Z",
                    ),
                    event(
                        101,
                        // 학습 주석: 가장 오래된 issue comment가 cursor와 같은 event입니다. 정렬 후에는
                        // snapshot 첫 항목이 되고, diff에서는 seen identity 때문에 제외됩니다.
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T08:00:00Z",
                    ),
                    event(
                        201,
                        // 학습 주석: 중간 timestamp의 review comment는 정렬 후 changes의 첫 새 항목이
                        // 되어야 합니다.
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    ),
                ],
            ),
        }));
        // 학습 주석: previous_state는 가장 오래된 id 101 event를 이미 본 상태로 표시합니다.
        // 따라서 정렬 후 diff는 그 뒤의 id 201, 301만 changes로 남겨야 합니다.
        let previous_state = GithubPullRequestPollState {
            // 학습 주석: cursor를 가장 오래된 timestamp에 두어, 정렬 결과와 changes 결과를 모두
            // 한 테스트에서 관찰할 수 있게 합니다.
            latest_submitted_at: Some("2026-04-08T08:00:00Z".to_string()),
            // 학습 주석: 같은 timestamp의 id 101은 이미 본 identity로 저장되어 changes에서 빠집니다.
            seen_events_at_latest_timestamp: vec![
                event(
                    101,
                    // 학습 주석: cursor timestamp의 event kind와 id가 identity에 포함되어 정확한
                    // 중복 제거 기준이 됩니다.
                    GithubPullRequestActivityKind::IssueComment,
                    "2026-04-08T08:00:00Z",
                )
                // 학습 주석: fixture event를 그대로 identity로 바꾸어 state와 snapshot의 key 생성
                // 방식이 서로 어긋나지 않게 합니다.
                .identity(),
            ],
        };

        // 학습 주석: poll 결과는 fake port의 원본 순서가 아니라 service가 정규화한 순서를 담아야 합니다.
        let result = service
            // 학습 주석: previous_state를 함께 넘겨 정렬과 diff가 같은 poll orchestration 안에서
            // 순서대로 일어나는지 확인합니다.
            .poll(&target, Some(&previous_state))
            // 학습 주석: 성공 결과를 풀어낸 뒤 snapshot 정렬과 changes 필터링을 각각 assertion합니다.
            .expect("poll should succeed");

        // 학습 주석: snapshot assertion은 service가 port 응답을 시간 오름차순으로 정렬했는지 확인합니다.
        // changes assertion은 정렬된 snapshot을 기준으로 cursor 뒤 id 201, 301만 새 변화로 남았는지
        // 확인합니다.
        assert_eq!(result.snapshot.events[0].id, 101);
        assert_eq!(result.snapshot.events[1].id, 201);
        assert_eq!(result.snapshot.events[2].id, 301);
        assert_eq!(result.changes.len(), 2);
        assert_eq!(result.changes[0].id, 201);
        assert_eq!(result.changes[1].id, 301);
    }

    // 학습 주석: 이 테스트는 이전 poll은 있었지만 당시 activity가 하나도 없던 상태를 검증합니다.
    // first poll과 달리 previous_state가 존재하므로, 이후 처음 나타난 activity는 새 변화로 보여야 합니다.
    #[test]
    // 학습 주석: `latest_submitted_at`이 없는 state는 "baseline은 세웠지만 비어 있었다"는 뜻입니다.
    // service가 이를 첫 poll처럼 무시하면 사용자는 비어 있던 PR에 새로 달린 review를 놓치게 됩니다.
    fn poll_surfaces_first_activity_after_empty_baseline() {
        // 학습 주석: target은 이전 baseline과 새 snapshot이 같은 PR에 속한다는 테스트 전제입니다.
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        // 학습 주석: fake snapshot에는 비어 있던 baseline 이후 처음 발견된 review comment 하나만
        // 넣습니다. expected changes도 이 event 하나입니다.
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            // 학습 주석: snapshot helper가 PR metadata를 채워 주므로 테스트 본문은 edge case인
            // "첫 activity" event에 집중할 수 있습니다.
            snapshot: snapshot(
                target.clone(),
                vec![event(
                    201,
                    // 학습 주석: 이 review comment는 previous_state의 timestamp cursor가 없는 상태에서
                    // 처음 등장한 activity입니다. 따라서 changes에 반드시 포함되어야 합니다.
                    GithubPullRequestActivityKind::ReviewComment,
                    "2026-04-08T10:30:00Z",
                )],
            ),
        }));
        // 학습 주석: default state는 latest timestamp와 seen identity가 모두 비어 있습니다. `None`이
        // 아닌 `Some(default)`로 전달해야 first-poll baseline branch와 다른 경로를 검증합니다.
        let previous_state = GithubPullRequestPollState::default();

        // 학습 주석: 이전 state가 존재하므로 service는 현재 snapshot 전체를 "비어 있던 기준점 이후의
        // 첫 변화"로 반환해야 합니다.
        let result = service
            // 학습 주석: `Some(&previous_state)`가 empty baseline case를 활성화합니다.
            .poll(&target, Some(&previous_state))
            // 학습 주석: fake port가 성공 snapshot을 반환하므로 결과를 바로 열어 changes를 확인합니다.
            .expect("poll should succeed");

        // 학습 주석: 새 activity 하나가 그대로 changes에 나타나야 빈 baseline 이후 첫 review를
        // 놓치지 않습니다.
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].id, 201);
    }

    // 학습 주석: 이 테스트는 cursor와 같은 timestamp에 새 event가 추가되는 GitHub edge case를
    // 검증합니다. timestamp만 비교하면 같은 시각의 새 review comment를 놓칠 수 있습니다.
    #[test]
    // 학습 주석: 이전 state가 id 210만 봤고 같은 timestamp의 id 320은 아직 못 봤다면, service는
    // id 320을 새 변화로 남겨야 합니다. 이것이 identity set을 함께 저장하는 이유입니다.
    fn poll_keeps_new_same_timestamp_events_visible() {
        // 학습 주석: 같은 PR target으로 동일 timestamp bucket 안의 중복 제거 동작만 분리해서 봅니다.
        let target = GithubPullRequestTarget::new("acme/widgets", 42);
        // 학습 주석: snapshot은 같은 submitted_at을 가진 두 event를 담습니다. 하나는 이미 본 event,
        // 하나는 새 event라 timestamp 비교만으로는 둘을 구분할 수 없습니다.
        let service = GithubReviewPollerService::new(Arc::new(FakeGithubReviewPollerPort {
            // 학습 주석: 같은 timestamp bucket fixture는 `seen_events_at_latest_timestamp`가 실제로
            // diff 결과를 좌우하는지 확인하기 위한 최소 입력입니다.
            snapshot: snapshot(
                target.clone(),
                vec![
                    event(
                        210,
                        // 학습 주석: id 210 issue comment는 previous_state에 이미 기록된 event입니다.
                        // changes에서 제외되어야 합니다.
                        GithubPullRequestActivityKind::IssueComment,
                        "2026-04-08T10:30:00Z",
                    ),
                    event(
                        320,
                        // 학습 주석: id 320 review comment는 같은 timestamp지만 identity set에 없습니다.
                        // 따라서 새 변화로 노출되어야 합니다.
                        GithubPullRequestActivityKind::ReviewComment,
                        "2026-04-08T10:30:00Z",
                    ),
                ],
            ),
        }));
        // 학습 주석: previous_state는 latest timestamp를 현재 snapshot과 같은 값으로 둡니다. 이때
        // 새 event 판정은 timestamp가 아니라 seen identity membership에 달려 있습니다.
        let previous_state = GithubPullRequestPollState {
            // 학습 주석: cursor timestamp와 snapshot timestamp가 같아도 diff가 완전히 멈추면 안 됩니다.
            latest_submitted_at: Some("2026-04-08T10:30:00Z".to_string()),
            // 학습 주석: latest timestamp bucket에서 이미 본 event는 id 210 하나뿐입니다.
            seen_events_at_latest_timestamp: vec![
                event(
                    210,
                    // 학습 주석: kind와 id가 함께 들어간 identity가 id 320 review comment와 구분됩니다.
                    GithubPullRequestActivityKind::IssueComment,
                    "2026-04-08T10:30:00Z",
                )
                // 학습 주석: state에는 전체 event가 아니라 중복 판정에 필요한 identity만 저장합니다.
                .identity(),
            ],
        };

        // 학습 주석: poll 호출은 같은 timestamp bucket의 identity diff branch를 통과해야 합니다.
        let result = service
            // 학습 주석: previous_state를 읽기 전용으로 빌려 주어 같은 timestamp 기준을 적용합니다.
            .poll(&target, Some(&previous_state))
            // 학습 주석: 성공 결과에서 changes만 확인하면 edge case 의도가 선명해집니다.
            .expect("poll should succeed");

        // 학습 주석: id 210은 이미 본 event라 제외되고, id 320 하나만 changes에 남아야 합니다.
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].id, 320);
    }

    // 학습 주석: `snapshot` helper는 각 테스트가 관심 있는 target과 event 목록만 넘기면
    // `GithubPullRequestActivitySnapshot` 전체를 만들 수 있게 하는 fixture factory입니다. PR metadata를
    // 한곳에 고정해 테스트 본문이 diff 시나리오에 집중하게 합니다.
    fn snapshot(
        // 학습 주석: target은 service가 poll한 PR identity입니다. helper가 이 값을 그대로 snapshot에
        // 넣어 result.snapshot과 caller의 target이 같은 PR을 가리키게 합니다.
        target: GithubPullRequestTarget,
        // 학습 주석: events는 각 테스트가 edge case에 맞게 구성한 activity stream입니다. helper는
        // 순서를 바꾸지 않으므로 정렬 테스트는 의도한 원본 순서를 service에 그대로 전달합니다.
        events: Vec<GithubPullRequestActivityEvent>,
    ) -> GithubPullRequestActivitySnapshot {
        GithubPullRequestActivitySnapshot {
            target,
            // 학습 주석: title은 diff logic에는 영향을 주지 않지만 snapshot domain type이 PR 표시 정보를
            // 함께 운반한다는 contract를 유지합니다.
            title: "Add review polling".to_string(),
            // 학습 주석: url은 GitHub PR 화면으로 돌아갈 수 있는 metadata입니다. 테스트에서는 고정값으로
            // 두어 event filtering assertion과 독립시킵니다.
            url: "https://github.com/acme/widgets/pull/42".to_string(),
            // 학습 주석: head_branch/base_branch도 poll diff에는 직접 쓰이지 않지만, service가 반환하는
            // snapshot이 실제 PR context를 잃지 않는지 보여 주는 fixture metadata입니다.
            head_branch: "feature/native-github-poller-port".to_string(),
            base_branch: "prerelease".to_string(),
            events,
        }
    }

    // 학습 주석: `event` helper는 diff에 중요한 id, kind, submitted_at만 테스트마다 바꾸고 나머지
    // GitHub activity 필드는 안정적인 기본값으로 채웁니다. 덕분에 각 테스트는 cursor와 identity
    // 조건만 읽으면 됩니다.
    fn event(
        // 학습 주석: id는 identity key의 핵심 값입니다. 테스트는 id 차이로 어떤 event가 changes에
        // 남았는지 명확히 확인합니다.
        id: u64,
        // 학습 주석: kind는 identity에 포함되는 activity 종류입니다. review, review comment,
        // issue comment가 같은 id를 가질 가능성까지 구분하는 데 필요합니다.
        kind: GithubPullRequestActivityKind,
        // 학습 주석: submitted_at은 timestamp cursor와 정렬의 기준입니다. 문자열이지만 ISO-8601 UTC
        // 형태라 lexicographic sort가 시간 순서와 일치합니다.
        submitted_at: &str,
    ) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id,
            kind,
            // 학습 주석: event가 받은 timestamp를 owned String으로 바꾸어 domain event가 test helper의
            // borrowed input lifetime에 묶이지 않게 합니다.
            submitted_at: submitted_at.to_string(),
            // 학습 주석: author/body는 notification display에 필요한 metadata지만, 이 service test에서는
            // diff 결과와 무관하므로 고정 기본값을 사용합니다.
            author_login: "reviewer".to_string(),
            body: "Looks good".to_string(),
            // 학습 주석: state는 review approval/request-changes 같은 상태가 있을 때 채워지는 선택값입니다.
            // 여기서는 event identity와 timestamp diff만 보므로 None으로 둡니다.
            state: None,
            // 학습 주석: url은 id를 포함해 assertion 실패 시 어떤 fixture event인지 추적하기 쉽게 합니다.
            url: format!("https://github.com/acme/widgets/pull/42#{id}"),
            // 학습 주석: path는 inline review comment가 파일 경로를 가질 때 쓰입니다. 현재 테스트들은
            // path 기반 분기가 아니라 poll diff를 검증하므로 비워 둡니다.
            path: None,
        }
    }
}
