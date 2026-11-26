//! 저장소 식별자를 planning 기준 데이터의 위치 계산에서 얻는다 (A03). 커널 잠금과 같은 값이라 같은 저장소의
//! worktree 들이 같은 식별자를 갖는다.

use std::path::Path;

use anyhow::{Result, bail};

use super::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::repository_identity_port::RepositoryIdentityPort;

pub(crate) struct PlanningRepositoryIdentityAdapter;

impl RepositoryIdentityPort for PlanningRepositoryIdentityAdapter {
    fn repository_identity(&self, cwd: &str) -> Result<String> {
        // 없는 폴더는 위치 계산이 그 글자를 그대로 식별자로 쓴다. 장부에 엉뚱한 범위를 만들지 않게 먼저 막는다.
        let path = Path::new(cwd);
        if !path.is_absolute() || !path.is_dir() {
            bail!("`cwd` must be an existing absolute folder")
        }
        Ok(
            SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(cwd)?
                .repository_identity,
        )
    }
}
