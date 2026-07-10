use anyhow::Result;

/*
 * ParallelAgentProfileRepositoryPort는 workspace별 profile JSON의 물리 저장소 경계다.
 * application service는 JSON schema와 default/validation만 소유하고, `.akra` 경로와
 * symlink/atomic-write 정책은 outbound adapter가 책임진다.
 */
pub trait ParallelAgentProfileRepositoryPort: Send + Sync {
    fn load_profile_config_json(&self, workspace_dir: &str) -> Result<Option<String>>;

    fn save_profile_config_json(&self, workspace_dir: &str, body: &str) -> Result<()>;
}
