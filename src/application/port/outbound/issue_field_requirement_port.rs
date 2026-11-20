use anyhow::Result;

use crate::domain::issue_template::IssueTemplateUnsupportedInput;
use crate::domain::issue_validation::IssueFieldRequirement;
use crate::domain::issue_work::IssueReference;

/*
 * 공급자가 이슈 유형마다 요구하는 필수 필드를 읽는 계약이다 (#2433). 읽기만 한다. 답은 프로젝트 설정의
 * 필드 이름과 허용 값이다. 이슈의 글은 없다.
 *
 * 지원하는 필수 필드는 검증기의 요구(`IssueFieldRequirement`)로 옮긴다. 지원하지 않는 필수 필드는
 * `unsupported_required_fields` 에 남긴다. 버리지 않는다.
 */

/// 아는 이슈의 필수 필드를 읽는 요청이다. 프로젝트와 이슈 유형은 공급자가 이슈에서 정한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFieldRequirementRequest {
    pub issue: IssueReference,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueFieldRequirementResponse {
    /// 검증기가 검사할 수 있는 필수 필드이다 (`IssueRequirements::template_fields` 의 값).
    pub required_fields: Vec<IssueFieldRequirement>,
    /// 지원 범위 밖의 필수 필드이다. 비어 있지 않으면 사용자가 직접 확인해야 한다.
    pub unsupported_required_fields: Vec<IssueTemplateUnsupportedInput>,
}

pub trait IssueFieldRequirementPort: Send + Sync {
    /// 이슈의 프로젝트와 이슈 유형이 요구하는 필수 필드를 읽는다. 답이 다른 서버의 것이면 오류이다.
    fn read_requirements(
        &self,
        request: IssueFieldRequirementRequest,
    ) -> Result<IssueFieldRequirementResponse>;
}
