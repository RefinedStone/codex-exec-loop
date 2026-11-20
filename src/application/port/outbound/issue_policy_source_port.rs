use anyhow::Result;

/*
 * 이슈 정책 파일의 물리 저장소 경계이다 (#2416). application service 는 TOML 스키마, 기본값, 검증을 가진다.
 * 경로와 안전한 읽기는 outbound adapter 가 맡는다.
 *
 * 파일이 없으면 `None` 이다. 읽지 못하면 오류이다. 호출자는 오류를 "설정 없음"으로 바꾸지 않는다.
 */
pub trait IssuePolicySourcePort: Send + Sync {
    /// 사용자 설정 파일의 글이다. 모든 프로젝트에 적용한다.
    fn load_user_policy_toml(&self) -> Result<Option<String>>;

    /// 프로젝트 설정 파일의 글이다.
    fn load_project_policy_toml(&self, workspace_dir: &str) -> Result<Option<String>>;
}
