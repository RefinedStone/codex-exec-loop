pub mod issue_creation;
/*
 * service 모듈은 화면이나 저장소 세부사항을 모르는 유스케이스 묶음이다.
 * 각 service는 domain 모델과 outbound port를 조합하고, adapter는 이 공개 진입점만 호출한다.
 */
pub mod admin_debug_harness;
pub mod app_server_prompt_log_query;
pub mod kernel_command;
// stored_command 는 저장 명령을 planning 서비스로 실행한다 (plan 18 K4-9d).
pub mod stored_command;
// conversation_service는 사용자 turn과 app-server 스트림을 대화 단위로 조율한다.
pub mod conversation_service;
// github_review_poller_service는 GitHub review thread 상태를 polling 유스케이스로 묶는다.
pub mod github_review_poller_service;
pub mod pr_review;
// review_center는 approval/review inbox와 thread/history 조회를 application read facade로 묶는다.
pub mod review_center;
// manual_prompt_preparation은 TUI가 직접 bootstrap/intake를 조립하지 않게 하는 application preflight다.
pub mod manual_prompt_preparation;
// MCP hub service 는 등록부 확인, 도구 검색, 결과 축약을 맡는다.
pub mod mcp_hub;
// issue_handoff 는 원격 이슈를 AI 전달 정책으로 가려 인계의 공유본을 만든다 (#2428).
pub mod issue_collection;
pub mod issue_coordinator;
pub mod issue_handoff;
// issue_code_validator 는 사용자가 승인한 코드 검증기를 실행하고 필수 검증기 실패로 게시를 막는다 (#2431).
pub mod issue_code_validator;
// issue_draft 는 정책, 템플릿, 검증, 마스킹, 기록 뒤의 게시를 이슈 초안 명령으로 잇는다 (#2426).
pub mod issue_decision;
pub mod issue_draft;
// issue_masking 은 공개 정책에 따라 제목과 본문의 민감 구간을 버리거나 표식으로 가린다 (#2419).
pub mod issue_masking;
// issue_publish 는 승인한 공유본으로 기존 이슈를 고친다. 쓰기 직전에 원격을 다시 읽는다 (#2423).
pub mod issue_publish;
// issue_publish_journal 은 게시 의도를 기록한 뒤에만 쓰고, 재조회로 결과를 확인한다 (#2424).
pub mod issue_publish_journal;
// issue_policy 는 이슈 정책 파일을 검증하고 사용자 < 프로젝트 순서로 합친다 (#2416).
pub mod issue_policy;
// 자동화 정책은 별도 파일과 사용자 권한 제한으로 해석한다 (#2540).
pub mod issue_automation_policy;
pub mod issue_triage;
// issue_validation 은 초안 검증과 게시 관문이다. 둘 다 같은 domain 규칙을 부른다 (#2418).
pub mod issue_validation;
// issue_review 는 검증된 공유 초안을 모델에 보내고 구조화된 권고를 돌려준다 (#2430).
pub mod issue_review;
// issue_template 은 Markdown 템플릿과 YAML Issue Form 의 필수 입력을 해석한다 (#2417).
pub mod issue_template;
// parallel_agent_profile은 병렬 agent의 이름, 역할, avatar, persona prompt 설정을 관리한다.
pub mod parallel_agent_profile;
// parallel_mode는 여러 worktree/agent lane을 만들고 배분하는 application 흐름이다.
pub mod parallel_mode;
// post_turn_decision은 턴 완료 후 auto-follow, parallel continuation, operator alert를 분리해 결정한다.
pub(crate) mod post_turn_decision;
// post_turn_evaluation은 완료된 turn 뒤 planning/parallel 후속 평가를 UI 없는 application effect로 실행한다.
pub mod post_turn_evaluation;
// planning은 작업 방향, queue, worker 실행, control 명령을 한 도메인 흐름으로 묶는다.
pub mod planning;
// prompt_component는 사용자에게 보이지 않는 prompt 조각을 service 내부에서 재사용하게 한다.
pub(crate) mod prompt_component;
// pr_validation_query는 durable validation authority를 bounded operator projection으로 조회한다.
pub mod pr_validation_command;
pub mod pr_validation_query;
// pr_validation_rollout_evidence_query는 rollout artifact를 검증하고 fail-closed Admin projection으로 정규화한다.
pub mod pr_validation_rollout_evidence_query;
// session_service는 저장된 세션 목록과 상세 조회를 application 경계로 노출한다.
pub mod session_service;
// startup_service는 TUI 시작 전 환경 점검과 진단 정보를 모은다.
pub mod startup_service;
// turn_prompt_assembly_service는 현재 상태와 사용자 입력을 app-server turn prompt로 조립한다.
pub mod turn_prompt_assembly_service;
// git graph service 는 커밋 그래프의 한 쪽을 읽고 레인을 배치한다 (plan 27).
pub mod git_graph;
// git operation service 는 git 작업을 계획하고, 기대 상태를 다시 본 뒤 실행하고, 되돌린다 (plan 27 G3).
pub mod git_operation;
// work context service 는 스레드의 외부 문서를 모델 없이 수집한다.
pub mod resource_version;
pub mod work_context;
// evidence_vault_unlock 은 사용자 확인 뒤에 이슈와 열람 세션에 묶인 Vault 잠금 해제 권한을 준다 (#2422).
pub mod evidence_vault_unlock;
// evidence_vault_restore 는 최신 게시본이 저장한 연결과 같을 때만 복원하고, 다르면 보류하고 재연결을 받는다 (#2425).
pub mod evidence_vault_restore;
// evidence_vault_backup 은 사용자 승인 뒤에 복구 키로 Vault 백업을 내보내고 가져온다 (#2432).
pub mod evidence_vault_backup;
pub mod issue_form_install;
pub mod issue_source_provenance;

pub mod issue_intervention;
