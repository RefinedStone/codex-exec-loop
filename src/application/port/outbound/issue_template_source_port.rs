use anyhow::Result;

/*
 * 대상 저장소의 이슈 템플릿 파일을 읽는 경계이다 (#2417). application service 는 형식 해석과 판 계산을 가진다.
 * 경로와 안전한 읽기는 outbound adapter 가 맡는다.
 *
 * 파일이 없으면 `None` 이다. 읽지 못하면 오류이다. 호출자는 오류를 "템플릿 없음"으로 바꾸지 않는다.
 */
pub trait IssueTemplateSourcePort: Send + Sync {
    /// `template_path` 는 저장소 뿌리에서 본 상대 경로이다 (`IssueConvention::template`).
    fn load_template(&self, workspace_dir: &str, template_path: &str) -> Result<Option<String>>;
}
