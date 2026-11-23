use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

use super::{IssueAutomationError, validate_text};

/// 표시 이름이나 URL이 아닌, 저장된 바인딩의 식별자다.
/// ID는 정규화하지 않는다. 같은 ID의 좌표 변경은 저장 경계에서 거절한다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IssueAutomationBindingId(String);

impl IssueAutomationBindingId {
    pub fn new(value: impl Into<String>) -> Result<Self, IssueAutomationError> {
        let value = value.into();
        validate_text(&value, "binding.id")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for IssueAutomationBindingId {
    type Error = IssueAutomationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<IssueAutomationBindingId> for String {
    fn from(value: IssueAutomationBindingId) -> Self {
        value.0
    }
}

impl std::fmt::Display for IssueAutomationBindingId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 설정된 자율 수준이다. 값이 Gated/Auto라고 실행 허가가 생기지는 않는다.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationAutonomy {
    Off,
    #[default]
    Observe,
    Propose,
    Gated,
    Auto,
}

/// 저장 형식이다. 공개 도메인 값은 hydrate를 지나서만 복원한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedIssueAutomationBinding {
    pub id: IssueAutomationBindingId,
    pub provider: IssueProvider,
    pub server_id: String,
    pub repository_id: String,
    pub project_scope: IssueProjectScope,
    pub autonomy: IssueAutomationAutonomy,
}

/// 이슈 출처의 안정 좌표와 로컬 프로젝트 하나의 연결이다.
/// revision은 저장 포트의 wrapper가 소유한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationBinding {
    id: IssueAutomationBindingId,
    provider: IssueProvider,
    server_id: String,
    repository_id: String,
    project_scope: IssueProjectScope,
    autonomy: IssueAutomationAutonomy,
}

impl IssueAutomationBinding {
    pub fn for_source(
        repository: &super::source::IssueSourceRepository,
    ) -> Result<Self, IssueAutomationError> {
        use sha2::{Digest, Sha256};
        repository
            .validate()
            .map_err(|_| IssueAutomationError::InvalidValue {
                field: "binding.source",
            })?;
        let encoded =
            serde_json::to_vec(repository).map_err(|_| IssueAutomationError::InvalidValue {
                field: "binding.source",
            })?;
        let digest = Sha256::digest(encoded);
        Self::new(
            IssueAutomationBindingId::new(format!("issue-source-{digest:x}"))?,
            repository.provider,
            repository.server_id.clone(),
            repository.repository_id.clone(),
            repository.project_scope.clone(),
        )
    }

    pub fn new(
        id: IssueAutomationBindingId,
        provider: IssueProvider,
        server_id: impl Into<String>,
        repository_id: impl Into<String>,
        project_scope: IssueProjectScope,
    ) -> Result<Self, IssueAutomationError> {
        Self::hydrate(PersistedIssueAutomationBinding {
            id,
            provider,
            server_id: server_id.into(),
            repository_id: repository_id.into(),
            project_scope,
            autonomy: IssueAutomationAutonomy::default(),
        })
    }

    pub fn hydrate(record: PersistedIssueAutomationBinding) -> Result<Self, IssueAutomationError> {
        validate_text(record.id.as_str(), "binding.id")?;
        validate_text(&record.server_id, "binding.server_id")?;
        validate_text(&record.repository_id, "binding.repository_id")?;
        // 절대 경로/대표 checkout의 해석은 저장 경계가 한다. 여기서는 문자열을 바꾸지 않는다.
        validate_text(&record.project_scope.workspace_dir, "binding.project_scope")?;
        Ok(Self {
            id: record.id,
            provider: record.provider,
            server_id: record.server_id,
            repository_id: record.repository_id,
            project_scope: record.project_scope,
            autonomy: record.autonomy,
        })
    }

    pub fn to_record(&self) -> PersistedIssueAutomationBinding {
        PersistedIssueAutomationBinding {
            id: self.id.clone(),
            provider: self.provider,
            server_id: self.server_id.clone(),
            repository_id: self.repository_id.clone(),
            project_scope: self.project_scope.clone(),
            autonomy: self.autonomy,
        }
    }

    pub fn id(&self) -> &IssueAutomationBindingId {
        &self.id
    }

    pub fn provider(&self) -> IssueProvider {
        self.provider
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn project_scope(&self) -> &IssueProjectScope {
        &self.project_scope
    }

    pub fn autonomy(&self) -> IssueAutomationAutonomy {
        self.autonomy
    }

    /// 사람이 수준을 올릴 수 있는지 등의 권한은 이 값 변경을 요청하는 상위 경계가 검사한다.
    pub fn set_autonomy(&mut self, autonomy: IssueAutomationAutonomy) -> bool {
        if self.autonomy == autonomy {
            return false;
        }
        self.autonomy = autonomy;
        true
    }

    pub fn matches_reference(&self, reference: &IssueReference) -> bool {
        self.provider == reference.provider
            && self.server_id == reference.server_id
            && self.repository_id == reference.repository_id
            && self.project_scope == reference.project_scope
    }

    /// 같은 ID를 다른 출처/프로젝트에 재사용하지 않는다. 자율 수준만 바꿀 수 있다.
    pub fn validate_replacement(&self, candidate: &Self) -> Result<(), IssueAutomationError> {
        if self.id != candidate.id
            || self.provider != candidate.provider
            || self.server_id != candidate.server_id
            || self.repository_id != candidate.repository_id
            || self.project_scope != candidate.project_scope
        {
            return Err(IssueAutomationError::BindingChanged);
        }
        Ok(())
    }
}
