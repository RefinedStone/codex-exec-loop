//! Gateway 의 `akra/git/*` 처리이다 (plan 27 G1 – G3). 저장소의 커밋 그래프, ref 들의 지문, 커밋 하나의 상세를 읽어
//! 준다. git 실행은 블로킹이므로 요청마다 블로킹 작업으로 돌린다. 접속의 읽기 고리는 기다리지 않는다.
//!
//! 저장소를 고치는 메서드는 `akra/git/execute` 와 `akra/git/undo` 둘이다 (G3). 둘 다 작업 엔진을 거친다: 기대 상태를
//! 다시 보고, 백업 ref 를 쓰고, 일지에 적는다.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::recorder::{RecordDirection, SessionRecording};
use crate::application::port::inbound::git_graph_port::{
    GitGraphCursor, GitGraphPageRequest, GitGraphPort,
};
use crate::application::port::inbound::git_operation_port::GitOperationPort;
use crate::domain::git_operation::{GitOperation, GitOperationExpectation};

pub(super) const GIT_METHOD_PREFIX: &str = "akra/git/";
/// params `{ cwd, limit?, cursor? }` -> result `GitGraphPage`. `cursor` 는 앞 쪽의 `next` 이다.
const GIT_GRAPH_METHOD: &str = "akra/git/graph";
/// params `{ cwd }` -> result `GitGraphRefsState`. 화면이 저장소가 바뀌었는지 보려고 주기적으로 부른다.
const GIT_REFS_METHOD: &str = "akra/git/refs";
/// params `{ cwd, commit }` -> result `{ commit: GitCommitDetail | null }`. 그 커밋이 없으면 `null` 이다.
const GIT_COMMIT_METHOD: &str = "akra/git/commit";
/// params `{ cwd, operation }` -> result `GitOperationPlan`. 저장소를 고치지 않는다.
const GIT_PLAN_METHOD: &str = "akra/git/plan";
/// params `{ cwd, operation, expectation }` -> result `GitOperationOutcome`. `expectation` 은 계획의 것이다.
const GIT_EXECUTE_METHOD: &str = "akra/git/execute";
/// params `{ cwd }` -> result `{ operations: GitOperationRecord[] }`. 새 것이 앞이다.
const GIT_OPERATIONS_METHOD: &str = "akra/git/operations";
/// params `{ cwd, opId }` -> result `GitOperationOutcome`.
const GIT_UNDO_METHOD: &str = "akra/git/undo";
const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;
const JSON_RPC_INVALID_PARAMS: i64 = -32602;
const GIT_ERROR: i64 = -32000;

#[derive(Deserialize)]
struct RequestEnvelope {
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphParams {
    #[serde(default)]
    filter: crate::domain::git_graph::filter::GraphFilter,
    cwd: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<GitGraphCursor>,
}

#[derive(Deserialize)]
struct RefsParams {
    cwd: String,
}

#[derive(Deserialize)]
struct CommitParams {
    cwd: String,
    commit: String,
}

#[derive(Deserialize)]
struct PlanParams {
    cwd: String,
    operation: GitOperation,
}

#[derive(Deserialize)]
struct ExecuteParams {
    cwd: String,
    operation: GitOperation,
    expectation: GitOperationExpectation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UndoParams {
    cwd: String,
    op_id: String,
}

/// `akra/git/*` 가 쓰는 서비스들이다. 작업 엔진이 없으면 그 메서드들은 모르는 메서드이다.
#[derive(Clone)]
pub(super) struct GitPorts {
    pub(super) graph: Arc<dyn GitGraphPort>,
    pub(super) operations: Option<Arc<dyn GitOperationPort>>,
}

fn params<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let envelope = serde_json::from_str::<RequestEnvelope>(text).ok()?;
    serde_json::from_value(envelope.params).ok()
}

/// 요청 하나를 블로킹 작업으로 처리하고 답을 접속의 보내기 함으로 보낸다.
pub(super) fn spawn_request(
    ports: GitPorts,
    id: Value,
    method: String,
    text: String,
    outbox: mpsc::Sender<Message>,
    recording: Option<Arc<SessionRecording>>,
) {
    tokio::spawn(async move {
        let fallback_id = id.clone();
        let response = tokio::task::spawn_blocking(move || respond(&ports, id, &method, &text))
            .await
            .unwrap_or_else(|_| error_response(fallback_id, GIT_ERROR, "the git request stopped"));
        if let Some(recording) = &recording {
            recording.record(RecordDirection::Gateway, &response);
        }
        let _ = outbox.send(Message::text(response)).await;
    });
}

fn respond(ports: &GitPorts, id: Value, method: &str, text: &str) -> String {
    let graph = ports.graph.as_ref();
    let result = match (method, ports.operations.as_deref()) {
        (GIT_GRAPH_METHOD, _) => graph_result(graph, text),
        (GIT_REFS_METHOD, _) => refs_result(graph, text),
        (GIT_COMMIT_METHOD, _) => commit_result(graph, text),
        (GIT_PLAN_METHOD, Some(operations)) => plan_result(operations, text),
        (GIT_EXECUTE_METHOD, Some(operations)) => execute_result(operations, text),
        (GIT_OPERATIONS_METHOD, Some(operations)) => operations_result(operations, text),
        ("akra/git/resolvePick", Some(operations)) => resolve_pick_result(operations, text),
        (GIT_UNDO_METHOD, Some(operations)) => undo_result(operations, text),
        _ => {
            return error_response(
                id,
                JSON_RPC_METHOD_NOT_FOUND,
                &format!("unknown gateway method `{method}`"),
            );
        }
    };
    match result {
        Some(Ok(result)) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Some(Err(error)) => error_response(id, GIT_ERROR, &format!("{error:#}")),
        None => error_response(
            id,
            JSON_RPC_INVALID_PARAMS,
            &format!("the params of `{method}` are not valid"),
        ),
    }
}

/// 답이다. params 를 읽지 못하면 `None` 이다.
type MethodResult = Option<anyhow::Result<Value>>;

fn graph_result(graph: &dyn GitGraphPort, text: &str) -> MethodResult {
    let params = params::<GraphParams>(text)?;
    let page = graph.graph_page(GitGraphPageRequest {
        filter: params.filter,
        repository_dir: params.cwd,
        limit: params.limit,
        cursor: params.cursor,
    });
    Some(page.and_then(|page| Ok(serde_json::to_value(page)?)))
}

fn refs_result(graph: &dyn GitGraphPort, text: &str) -> MethodResult {
    let params = params::<RefsParams>(text)?;
    let state = graph.refs_state(&params.cwd);
    Some(state.and_then(|state| Ok(serde_json::to_value(state)?)))
}

fn commit_result(graph: &dyn GitGraphPort, text: &str) -> MethodResult {
    let params = params::<CommitParams>(text)?;
    let detail = graph.commit_detail(&params.cwd, &params.commit);
    Some(detail.and_then(|detail| Ok(json!({ "commit": serde_json::to_value(detail)? }))))
}

fn plan_result(operations: &dyn GitOperationPort, text: &str) -> MethodResult {
    let params = params::<PlanParams>(text)?;
    let plan = operations.plan(&params.cwd, params.operation);
    Some(plan.and_then(|plan| Ok(serde_json::to_value(plan)?)))
}

fn execute_result(operations: &dyn GitOperationPort, text: &str) -> MethodResult {
    let params = params::<ExecuteParams>(text)?;
    let outcome = operations.execute(&params.cwd, params.operation, params.expectation);
    Some(outcome.and_then(|outcome| Ok(serde_json::to_value(outcome)?)))
}

fn operations_result(operations: &dyn GitOperationPort, text: &str) -> MethodResult {
    let params = params::<RefsParams>(text)?;
    let records = operations.operations(&params.cwd);
    Some(records.and_then(|records| Ok(json!({ "operations": serde_json::to_value(records)? }))))
}

fn resolve_pick_result(operations: &dyn GitOperationPort, text: &str) -> MethodResult {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PickParams {
        cwd: String,
        op_id: String,
        action: crate::domain::git_operation::cherry_pick::PickAction,
    }
    let query = params::<PickParams>(text)?;
    Some(
        operations
            .resolve_pick(&query.cwd, &query.op_id, query.action)
            .and_then(|outcome| Ok(serde_json::to_value(outcome)?)),
    )
}

fn undo_result(operations: &dyn GitOperationPort, text: &str) -> MethodResult {
    let params = params::<UndoParams>(text)?;
    let outcome = operations.undo(&params.cwd, &params.op_id);
    Some(outcome.and_then(|outcome| Ok(serde_json::to_value(outcome)?)))
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod cursor_tests;

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::application::port::inbound::git_graph_port::{GitGraphPage, GitGraphRefsState};
    use crate::application::port::inbound::git_operation_port::GitOperationOutcome;
    use crate::domain::git_graph::GitCommitDetail;
    use crate::domain::git_operation::{GitOperationPlan, GitOperationRecord};

    /// 받은 요청을 답에 되비춘다: `head` 는 폴더, `stale` 은 자리가 있었는지이다.
    struct EchoGraph;

    impl GitGraphPort for EchoGraph {
        fn graph_page(&self, request: GitGraphPageRequest) -> Result<GitGraphPage> {
            if request.repository_dir == "/broken" {
                anyhow::bail!("git did not run");
            }
            Ok(GitGraphPage {
                repository: true,
                stale: request.cursor.is_some(),
                head: Some(request.repository_dir),
                refs_digest: None,
                refs: Vec::new(),
                worktrees: Vec::new(),
                rows: Vec::new(),
                next: request.cursor,
            })
        }

        fn refs_state(&self, repository_dir: &str) -> Result<GitGraphRefsState> {
            Ok(GitGraphRefsState {
                rewritten_remote: None,
                repository: true,
                refs_digest: Some(repository_dir.to_string()),
                operation_in_progress: None,
                conflicted_files: 0,
            })
        }

        fn commit_detail(
            &self,
            repository_dir: &str,
            commit: &str,
        ) -> Result<Option<GitCommitDetail>> {
            if commit == "missing" {
                return Ok(None);
            }
            Ok(Some(GitCommitDetail {
                id: commit.to_string(),
                parents: Vec::new(),
                author_name: repository_dir.to_string(),
                author_email: String::new(),
                author_time: 1,
                committer_name: String::new(),
                committer_time: 2,
                message: "subject".to_string(),
                diff: String::new(),
                diff_too_large: true,
            }))
        }
    }

    /// 받은 요청을 답에 되비추는 가짜 작업 엔진이다. 저장소가 없다.
    struct EchoOperations;

    impl GitOperationPort for EchoOperations {
        fn plan(&self, _repository_dir: &str, operation: GitOperation) -> Result<GitOperationPlan> {
            Ok(GitOperationPlan {
                picks: Vec::new(),
                operation,
                expectation: GitOperationExpectation::default(),
                branch: None,
                before: None,
                after: None,
                impact: None,
                blockers: Vec::new(),
            })
        }

        fn execute(
            &self,
            repository_dir: &str,
            operation: GitOperation,
            expectation: GitOperationExpectation,
        ) -> Result<GitOperationOutcome> {
            let mut plan = self.plan(repository_dir, operation)?;
            plan.expectation = expectation;
            Ok(GitOperationOutcome::Stale { plan })
        }

        fn operations(&self, _repository_dir: &str) -> Result<Vec<GitOperationRecord>> {
            Ok(Vec::new())
        }

        fn undo(&self, _repository_dir: &str, op_id: &str) -> Result<GitOperationOutcome> {
            Ok(GitOperationOutcome::Refused {
                message: op_id.to_string(),
            })
        }
    }

    fn answer_from(ports: &GitPorts, method: &str, params: Value) -> Value {
        let text = json!({ "id": 1, "method": method, "params": params }).to_string();
        serde_json::from_str(&respond(ports, json!(1), method, &text)).unwrap()
    }

    fn answer(method: &str, params: Value) -> Value {
        let ports = GitPorts {
            graph: Arc::new(EchoGraph),
            operations: Some(Arc::new(EchoOperations)),
        };
        answer_from(&ports, method, params)
    }

    #[test]
    fn graph_request_carries_the_folder_and_the_cursor_to_the_port() {
        let first = answer(GIT_GRAPH_METHOD, json!({ "cwd": "/repo" }));
        assert_eq!(first["result"]["head"], json!("/repo"));
        assert_eq!(first["result"]["stale"], json!(false));
        assert_eq!(first["result"]["next"], Value::Null);

        let cursor = json!({ "skip": 2, "lanes": ["abc", null], "refsDigest": "d" });
        let next = answer(
            GIT_GRAPH_METHOD,
            json!({ "cwd": "/repo", "limit": 5, "cursor": cursor }),
        );
        assert_eq!(next["result"]["stale"], json!(true));
        // 자리는 받은 모양 그대로 돌아간다. 클라이언트가 다음 요청에 다시 쓴다.
        assert_eq!(next["result"]["next"], cursor);
    }

    #[test]
    fn refs_and_commit_requests_reach_the_port() {
        let refs = answer(GIT_REFS_METHOD, json!({ "cwd": "/repo" }));
        assert_eq!(
            refs["result"],
            json!({
                "repository": true,
                "refsDigest": "/repo",
                "operationInProgress": null,
                "conflictedFiles": 0
            })
        );

        let found = answer(
            GIT_COMMIT_METHOD,
            json!({ "cwd": "/repo", "commit": "abc1234" }),
        );
        let commit = &found["result"]["commit"];
        assert_eq!(commit["id"], json!("abc1234"));
        assert_eq!(commit["authorName"], json!("/repo"));
        assert_eq!(commit["committerTime"], json!(2));
        assert_eq!(commit["diffTooLarge"], json!(true));

        let missing = answer(
            GIT_COMMIT_METHOD,
            json!({ "cwd": "/repo", "commit": "missing" }),
        );
        assert_eq!(missing["result"]["commit"], Value::Null);
        assert_eq!(
            answer(GIT_COMMIT_METHOD, json!({ "cwd": "/repo" }))["error"]["code"],
            json!(JSON_RPC_INVALID_PARAMS)
        );
    }

    #[test]
    fn operation_requests_reach_the_engine_with_their_params() {
        let operation = json!({ "kind": "move_branch", "branch": "topic", "target": "abc" });
        let plan = answer(
            GIT_PLAN_METHOD,
            json!({ "cwd": "/repo", "operation": operation }),
        );
        assert_eq!(plan["result"]["operation"], operation);
        assert_eq!(plan["result"]["blockers"], json!([]));

        let expectation =
            json!({ "head": { "commit": "c1", "branch": "main" }, "branchCommit": "b1" });
        let executed = answer(
            GIT_EXECUTE_METHOD,
            json!({ "cwd": "/repo", "operation": operation, "expectation": expectation }),
        );
        assert_eq!(executed["result"]["status"], json!("stale"));
        assert_eq!(executed["result"]["plan"]["expectation"], expectation);

        let listed = answer(GIT_OPERATIONS_METHOD, json!({ "cwd": "/repo" }));
        assert_eq!(listed["result"], json!({ "operations": [] }));
        let undone = answer(GIT_UNDO_METHOD, json!({ "cwd": "/repo", "opId": "op-1" }));
        assert_eq!(
            undone["result"],
            json!({ "status": "refused", "message": "op-1" })
        );
    }

    #[test]
    fn a_write_request_without_its_expectation_or_without_an_engine_is_refused() {
        let operation = json!({ "kind": "reset", "target": "abc" });
        // 기대 상태가 없는 실행 요청은 계획을 보지 않은 요청이다.
        let no_expectation = answer(
            GIT_EXECUTE_METHOD,
            json!({ "cwd": "/repo", "operation": operation }),
        );
        assert_eq!(
            no_expectation["error"]["code"],
            json!(JSON_RPC_INVALID_PARAMS)
        );
        let unknown_kind = answer(
            GIT_PLAN_METHOD,
            json!({ "cwd": "/repo", "operation": { "kind": "push_force" } }),
        );
        assert_eq!(
            unknown_kind["error"]["code"],
            json!(JSON_RPC_INVALID_PARAMS)
        );

        let read_only = GitPorts {
            graph: Arc::new(EchoGraph),
            operations: None,
        };
        let refused = answer_from(
            &read_only,
            GIT_PLAN_METHOD,
            json!({ "cwd": "/repo", "operation": operation }),
        );
        assert_eq!(refused["error"]["code"], json!(JSON_RPC_METHOD_NOT_FOUND));
    }

    #[test]
    fn bad_params_unknown_methods_and_port_failures_are_json_rpc_errors() {
        assert_eq!(
            answer(GIT_GRAPH_METHOD, json!({}))["error"]["code"],
            json!(JSON_RPC_INVALID_PARAMS)
        );
        assert_eq!(
            answer("akra/git/reset", json!({ "cwd": "/repo" }))["error"]["code"],
            json!(JSON_RPC_METHOD_NOT_FOUND)
        );
        let failed = answer(GIT_GRAPH_METHOD, json!({ "cwd": "/broken" }));
        assert_eq!(failed["error"]["code"], json!(GIT_ERROR));
        assert_eq!(failed["error"]["message"], json!("git did not run"));
    }
}

#[cfg(test)]
mod realign_screen;
