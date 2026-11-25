//! Desktop/CLI가 같은 기존 task snapshot 읽기를 사용한다.
use crate::adapter::inbound::kernel_protocol::PlanningTaskList;
use crate::application::port::inbound::planning_task_tool_port::{
    PlanningTaskToolListRequest, PlanningTaskToolRequest,
};
use anyhow::Result;

pub(crate) fn query(workspace: &str) -> Result<PlanningTaskList> {
    let snapshot = super::production::build_planning_task_tool_port(workspace).run(
        workspace,
        PlanningTaskToolRequest::ListTasks(PlanningTaskToolListRequest {
            version: 1,
            status: Vec::new(),
            limit: None,
        }),
    )?;
    Ok(PlanningTaskList::from_snapshot(snapshot))
}
