use super::super::tests::Fixture;
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::IssueAutomationAutonomy;

mod lifecycle;
mod migration;
mod service;

fn insert(fixture: &Fixture, id: &str) -> IssueAutomationBindingId {
    let binding = fixture.binding(id);
    fixture
        .adapter()
        .insert_binding(fixture.cwd(), &binding)
        .unwrap();
    binding.id().clone()
}

fn intent(
    fixture: &Fixture,
    binding: &IssueAutomationBindingId,
    id: &str,
    paused: bool,
) -> IssueBindingPauseIntent {
    let adapter = fixture.adapter();
    IssueBindingPauseIntent {
        command_id: id.into(),
        binding_id: binding.clone(),
        paused,
        expected_revision: adapter
            .issue_binding_pause_state(fixture.cwd(), binding)
            .unwrap()
            .revision,
        resume_basis: (!paused).then(|| IssueBindingResumeBasis {
            binding_revision: adapter
                .get_binding(fixture.cwd(), binding)
                .unwrap()
                .unwrap()
                .revision,
            autonomy_revision: adapter
                .issue_autonomy_selection(fixture.cwd())
                .unwrap()
                .revision,
            policy_version: "a".repeat(64),
            effective_level: IssueAutomationAutonomy::Gated,
        }),
    }
}

fn finish(fixture: &Fixture, intent: &IssueBindingPauseIntent) -> StoredIssueBindingPauseCommand {
    fixture
        .adapter()
        .finish_issue_binding_pause(fixture.cwd(), intent, IssueBindingPauseCompletion::Apply)
        .unwrap()
}

fn count(fixture: &Fixture, table: &str) -> i64 {
    Connection::open(fixture.database())
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
