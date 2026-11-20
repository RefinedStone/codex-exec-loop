/*
 * 이슈 작업의 값 타입이다 (#2414, #2415). 외부 이슈 한 건을 가리키는 참조와, 밖으로 나가도 되는 공유본이다.
 *
 * 공유본은 가린 뒤의 글이다. 가린 구간의 평문과 키는 이 모듈에 없다. 그 값은 Evidence Vault 의 복원 계약
 * (`evidence_vault_port`)에만 있다.
 */
use serde::{Deserialize, Serialize};

/// 이슈를 가진 서비스의 종류이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueProvider {
    GitHub,
}

/// 이슈를 쓰는 로컬 프로젝트이다. 같은 이슈도 프로젝트가 다르면 규약, 정책, Vault 가 다르다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueProjectScope {
    /// 저장소 커널의 작업 폴더이다 (저장소의 대표 checkout). 절대 경로이다.
    pub workspace_dir: String,
}

/// 외부 이슈 한 건의 참조이다. ID 는 공급자가 준 안정 ID 이다. 이름이나 번호가 바뀌어도 값이 같다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueReference {
    pub provider: IssueProvider,
    /// 공급자의 서버이다. 같은 공급자의 다른 설치(예: 직접 운영하는 서버)를 구분한다.
    pub server_id: String,
    pub repository_id: String,
    pub issue_id: String,
    pub project_scope: IssueProjectScope,
}

/// 이슈의 공유본이다. 가린 뒤의 제목과 본문이다. 일반 Kernel 이벤트와 Gateway 응답에 넣어도 된다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSharedCopy {
    pub title: String,
    pub body: String,
}
