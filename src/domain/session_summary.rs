// session catalog는 app-server에서 epoch seconds로 넘어온 update time을 들고 온다.
// domain model에서 Local time label로 바꿔 두면 TUI session browser와 detail panel이 같은 표시 규칙을 공유한다.
use chrono::{Local, TimeZone};

// SessionSummary는 outbound app-server ThreadRecord를 TUI가 다룰 수 있는 domain 요약으로 낮춘 값이다.
// session browser search/sort, shell chrome selection, conversation lifecycle attach가 모두 이 구조체를 기준으로 움직인다.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    // id는 app-server thread/session을 다시 열 때 사용하는 안정 식별자다. UI에서는 short_id로
    // 줄여 보여 주지만, attach/load 요청에는 전체 id가 보존되어야 한다.
    pub id: String,
    // name은 provider가 알고 있는 session title이다. 없거나 공백이면 preview 첫 줄을 title로
    // 승격하므로 optional로 두고 `title()`에서 fallback policy를 한 번만 적용한다.
    pub name: Option<String>,
    // preview는 최근 대화 내용을 보여 주는 raw summary text다. session list entry, detail panel,
    // search scoring이 모두 이 값을 읽으므로 원문을 보존하고 표시 함수에서만 trimming/truncation을 한다.
    pub preview: String,
    // cwd는 session이 실행되던 workspace path다. session catalog filtering과 resumed planning
    // context가 workspace 기준으로 동작하므로 display label과 원본 path를 모두 이 값에서 파생한다.
    pub cwd: String,
    // source는 이 summary가 어떤 catalog backend에서 왔는지 설명한다. attach-only fallback,
    // handle-based reattach, provider-backed catalog를 UI에서 구분할 때 보조 정보로 쓰일 수 있다.
    pub source: String,
    // model_provider는 session을 만든 runtime/provider label이다. 같은 workspace의 여러 session을
    // 볼 때 어떤 backend로 생성된 thread인지 detail copy에서 판단할 수 있게 한다.
    pub model_provider: String,
    // updated_at_epoch는 sorting과 display label의 기준 시간이다. i64 epoch를 보존하면 adapter가
    // 시간대를 먼저 결정하지 않아도 domain/TUI가 일관된 local label을 만들 수 있다.
    pub updated_at_epoch: i64,
    // status_type은 provider가 보고한 session 상태다. 현재는 표시/검색 보조 값에 가깝지만,
    // 앞으로 running/archived/error session을 구분하는 필터 기준이 될 수 있어 domain에 남긴다.
    pub status_type: String,
    // path는 provider의 session record 위치 또는 handle path다. 사용자가 detail panel에서
    // 원본 record를 추적하거나 attach 실패를 진단할 때 cwd와 별도로 필요하다.
    pub path: String,
    // git_branch는 session이 만들어진 repository branch context다. workspace label만으로는
    // 같은 repo의 여러 branch 작업을 구분하기 어려워 optional detail로 보존한다.
    pub git_branch: Option<String>,
}

// 이 impl은 raw session metadata를 session browser가 바로 쓸 수 있는 작은 display values로 바꾼다.
// 변환 규칙을 domain model에 모아 둬서 shell_chrome, session_browser, lifecycle tests가 같은 fallback을 공유한다.
impl SessionSummary {
    // short_id는 list row에서 전체 thread id 대신 보여 주는 compact identifier다. attach에는
    // `id` 전체를 쓰고, 화면에서는 앞 8글자만 보여 긴 UUID/handle이 row 폭을 잡아먹지 않게 한다.
    pub fn short_id(&self) -> String {
        self.id.chars().take(8).collect()
    }

    // title은 session row의 주 제목이다. provider name이 있으면 우선하고, 없으면 preview 첫 줄을
    // 사용해 이름 없는 session도 browser에서 빈 제목으로 보이지 않게 한다.
    pub fn title(&self) -> String {
        self.name
            // Option<String>을 clone하는 이유는 title이 owned String을 반환해 renderer/list entry가
            // SessionSummary borrow 수명에 묶이지 않게 하기 위해서다.
            .clone()
            // provider가 빈 title을 보낼 수 있으므로 whitespace-only 값은 없는 것처럼 취급한다.
            // fallback을 이곳에 모아 모든 UI가 같은 빈-title 정책을 쓴다.
            .filter(|value| !value.trim().is_empty())
            // name이 없으면 preview 첫 줄을 title로 승격한다. `first_preview_line`이 truncation과
            // empty placeholder까지 책임지므로 title도 같은 안전한 표시 값을 얻는다.
            .unwrap_or_else(|| self.first_preview_line())
    }

    // first_preview_line은 list row에서 session 내용을 한 줄로 요약하는 값이다. raw preview는
    // 여러 줄일 수 있으므로 첫 non-empty line만 골라 title fallback과 row subtitle에 맞는 길이로 줄인다.
    pub fn first_preview_line(&self) -> String {
        self.preview
            // preview block의 첫 줄은 대개 사용자의 첫 요청 또는 최근 대화 요약이다. detail panel은
            // 전체 block을 쓰지만 list row는 첫 줄만 사용한다.
            .lines()
            .next()
            // provider text의 앞뒤 공백은 row alignment에 의미가 없으므로 표시 전에 제거한다.
            .map(str::trim)
            // 빈 첫 줄은 title/subtitle로 쓸 수 없으므로 placeholder fallback으로 내려 보낸다.
            .filter(|value| !value.is_empty())
            // session browser row는 폭이 제한되어 있어 여기서 Unicode-safe truncation을 적용한다.
            .map(Self::truncate)
            // preview가 완전히 비어도 UI가 빈 줄로 무너지는 대신 명시적인 placeholder를 보여 준다.
            .unwrap_or_else(|| "(empty preview)".to_string())
    }

    // preview_block은 detail panel용 값이다. list row와 달리 여러 줄 preview를 유지하되,
    // 순수 공백 preview는 같은 placeholder로 통일해 session detail이 빈 panel처럼 보이지 않게 한다.
    pub fn preview_block(&self) -> String {
        // trim은 provider가 붙인 바깥 공백만 제거한다. 내부 line break와 문장 구조는 detail
        // inspection에서 의미가 있을 수 있으므로 그대로 보존한다.
        let preview = self.preview.trim();
        if preview.is_empty() {
            "(empty preview)".to_string()
        } else {
            preview.to_string()
        }
    }

    // workspace_label은 긴 cwd 중 마지막 path component를 session row에 보여 주는 display helper다.
    // session search/filter는 원본 cwd를 쓰지만, 화면 row는 프로젝트 이름만 보여 주는 편이 스캔하기 쉽다.
    pub fn workspace_label(&self) -> String {
        self.cwd
            // path separator를 기준으로 뒤에서부터 나눠 `/home/me/project`의 `project`를 얻는다.
            // 현재 데이터는 provider 문자열이라 Path API 대신 표시용 문자열 분해로 충분하다.
            .rsplit('/')
            .next()
            // cwd가 `/tmp/project/`처럼 trailing slash로 끝나면 마지막 segment가 빈 문자열일 수 있다.
            // 그런 경우에는 fallback으로 전체 cwd를 보여 오해의 소지를 줄인다.
            .filter(|value| !value.is_empty())
            .unwrap_or(self.cwd.as_str())
            // renderer가 owned String을 받도록 변환한다. SessionSummary 자체를 오래 borrow하지 않고도
            // list entry DTO를 구성할 수 있다.
            .to_string()
    }

    // updated_at_label은 raw epoch를 local human-readable timestamp로 바꾼다. session browser는
    // 정렬 기준으로 epoch를 유지하면서, detail/list 표시에는 이 label을 사용한다.
    pub fn updated_at_label(&self) -> String {
        Local
            // timestamp_opt는 epoch가 local time으로 표현 불가능할 수 있음을 Result-like하게 다룬다.
            // session data가 외부 provider에서 오기 때문에 invalid timestamp를 panic으로 처리하지 않는다.
            .timestamp_opt(self.updated_at_epoch, 0)
            // ambiguous/nonexistent local time을 배제하고 단일 timestamp일 때만 formatted label을 쓴다.
            .single()
            // 표시 형식은 session browser row에서 읽기 쉬운 분 단위 precision으로 고정한다.
            .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
            // timestamp 변환 실패 시에도 원본 epoch를 보여 준다. 숨기는 것보다 진단 가능한 raw 값이
            // detail panel과 테스트에서 더 유용하다.
            .unwrap_or_else(|| self.updated_at_epoch.to_string())
    }

    // truncate는 session row용 preview/title fallback을 Unicode scalar 기준으로 줄인다. byte 기준으로
    // 자르면 한글/이모지 같은 multi-byte 문자를 깨뜨릴 수 있어 chars iterator를 사용한다.
    fn truncate(value: &str) -> String {
        // 72자는 session list에서 제목과 보조 metadata가 함께 보일 수 있게 하는 row-level copy limit다.
        const LIMIT: usize = 72;
        // chars count를 먼저 계산해 limit 이하 문자열은 원문을 그대로 반환한다. 불필요하게 ellipsis를
        // 붙이면 검색 결과와 provider preview가 실제보다 손실된 것처럼 보인다.
        let count = value.chars().count();
        if count <= LIMIT {
            return value.to_string();
        }

        // limit-1개 문자를 남긴 뒤 "..."를 붙인다. 기존 row width보다 약간 길어질 수 있지만,
        // preview가 잘렸다는 신호를 명확히 주는 쪽을 선택한 표시 helper다.
        value.chars().take(LIMIT - 1).collect::<String>() + "..."
    }
}
