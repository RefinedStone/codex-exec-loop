use crate::domain::planning::verification::TaskVerificationRecipe;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::planning::{
    TaskAcceptanceChannel as CoreTaskAcceptanceChannel,
    TaskAcceptanceCriterion as CoreTaskAcceptanceCriterion,
};

use super::NON_BLANK;

/// 사용자가 합의한 완료 기준의 전송 형식이다. revision과 판정 결과는 입력이 아니다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskAcceptanceCriterion {
    #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
    pub id: String,
    #[schemars(length(min = 1), regex(pattern = NON_BLANK))]
    pub scenario: String,
    pub channel: TaskAcceptanceChannel,
    pub essential: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_recipe: Option<TaskVerificationRecipe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskAcceptanceChannel {
    Command,
    Data,
    Http,
    Screen,
}

impl From<CoreTaskAcceptanceCriterion> for TaskAcceptanceCriterion {
    fn from(criterion: CoreTaskAcceptanceCriterion) -> Self {
        Self {
            id: criterion.id,
            scenario: criterion.scenario,
            channel: match criterion.channel {
                CoreTaskAcceptanceChannel::Command => TaskAcceptanceChannel::Command,
                CoreTaskAcceptanceChannel::Data => TaskAcceptanceChannel::Data,
                CoreTaskAcceptanceChannel::Http => TaskAcceptanceChannel::Http,
                CoreTaskAcceptanceChannel::Screen => TaskAcceptanceChannel::Screen,
            },
            essential: criterion.essential,
            verification_recipe: criterion.verification_recipe,
        }
    }
}

impl From<TaskAcceptanceCriterion> for CoreTaskAcceptanceCriterion {
    fn from(criterion: TaskAcceptanceCriterion) -> Self {
        Self {
            id: criterion.id,
            scenario: criterion.scenario,
            channel: match criterion.channel {
                TaskAcceptanceChannel::Command => CoreTaskAcceptanceChannel::Command,
                TaskAcceptanceChannel::Data => CoreTaskAcceptanceChannel::Data,
                TaskAcceptanceChannel::Http => CoreTaskAcceptanceChannel::Http,
                TaskAcceptanceChannel::Screen => CoreTaskAcceptanceChannel::Screen,
            },
            essential: criterion.essential,
            verification_recipe: criterion.verification_recipe,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::kernel_protocol::tests::{assert_matches_spec, schema_errors};
    use crate::adapter::inbound::kernel_protocol::{
        KernelCommand, KernelCommandInput, TaskFields, describe,
    };
    use crate::application::port::inbound::stored_command_port::StoredCommand;
    use serde_json::json;

    #[test]
    fn task_acceptance_wire_keeps_omitted_null_and_empty_distinct() {
        for (input, expected) in [
            (json!({"title": "Task"}), None),
            (json!({"title": "Task", "acceptance": null}), None),
            (json!({"title": "Task", "acceptance": []}), Some(Vec::new())),
        ] {
            let fields: TaskFields = serde_json::from_value(input).unwrap();
            assert_eq!(fields.acceptance, expected);
            let stored = fields.clone().into_stored().unwrap();
            assert_eq!(TaskFields::from_stored(stored), fields);
            let serialized = serde_json::to_value(fields).unwrap();
            if expected.is_some() {
                assert_eq!(serialized["acceptance"], json!([]));
            } else {
                assert!(serialized.get("acceptance").is_none());
            }
        }
    }

    #[test]
    fn task_acceptance_commands_preserve_all_channels_and_cas() {
        let spec = serde_json::to_value(describe()).unwrap();
        for (channel, core) in [
            ("command", CoreTaskAcceptanceChannel::Command),
            ("data", CoreTaskAcceptanceChannel::Data),
            ("http", CoreTaskAcceptanceChannel::Http),
            ("screen", CoreTaskAcceptanceChannel::Screen),
        ] {
            let value = json!({
                "type": "update_task", "task_id": "task-1", "expected_planning_revision": 7,
                "task": {"title": "Task", "acceptance": [{
                    "id": "criterion-1", "scenario": "The expected result is observable.",
                    "channel": channel, "essential": true
                }]}
            });
            let command: KernelCommand = serde_json::from_value(value.clone()).unwrap();
            let KernelCommandInput::Stored(stored) =
                command.clone().into_input("/workspace").unwrap()
            else {
                panic!("task criteria must use the stored command path");
            };
            let StoredCommand::UpdateTask {
                task,
                expected_planning_revision,
                ..
            } = &stored
            else {
                panic!("expected an update");
            };
            assert_eq!(*expected_planning_revision, 7);
            assert_eq!(task.acceptance.as_ref().unwrap()[0].channel, core);
            assert_eq!(KernelCommand::from_stored(stored), command);
            assert_matches_spec(&spec, "commands", "input", &value);
        }
    }

    #[test]
    fn host_recipe_round_trips_without_changing_legacy_criterion_json() {
        let legacy = json!({
            "id": "whitespace", "scenario": "No new whitespace errors.",
            "channel": "command", "essential": true
        });
        for value in [legacy.clone(), {
            let mut value = legacy.clone();
            value["verification_recipe"] = serde_json::Value::Null;
            value
        }] {
            let wire: TaskAcceptanceCriterion = serde_json::from_value(value).unwrap();
            assert!(wire.verification_recipe.is_none());
            let core: CoreTaskAcceptanceCriterion = wire.into();
            assert_eq!(serde_json::to_value(&core).unwrap(), legacy);
            assert_eq!(
                serde_json::to_value(TaskAcceptanceCriterion::from(core)).unwrap(),
                legacy
            );
        }

        let mut selected = legacy.clone();
        selected["verification_recipe"] = json!("git_diff_check_v1");
        let command: KernelCommand = serde_json::from_value(json!({
            "type": "update_task", "task_id": "task-1", "expected_planning_revision": 7,
            "task": {"title": "Task", "acceptance": [selected.clone()]}
        }))
        .unwrap();
        let KernelCommandInput::Stored(stored) = command.clone().into_input("/workspace").unwrap()
        else {
            panic!("recipe selection must use the existing task authoring boundary");
        };
        let StoredCommand::UpdateTask { task, .. } = &stored else {
            panic!("expected an update");
        };
        let criterion = &task.acceptance.as_ref().unwrap()[0];
        assert_eq!(
            criterion.verification_recipe,
            Some(TaskVerificationRecipe::GitDiffCheckV1)
        );
        assert_eq!(KernelCommand::from_stored(stored), command);
        assert_matches_spec(
            &serde_json::to_value(describe()).unwrap(),
            "commands",
            "input",
            &serde_json::to_value(command).unwrap(),
        );

        for channel in ["data", "http", "screen"] {
            let mut value = selected.clone();
            value["channel"] = json!(channel);
            let wire: TaskAcceptanceCriterion = serde_json::from_value(value).unwrap();
            assert!(crate::domain::planning::normalize_task_acceptance(&[wire.into()]).is_err());
        }
        selected["verification_recipe"] = json!("worker_supplied_command");
        assert!(serde_json::from_value::<TaskAcceptanceCriterion>(selected).is_err());
    }

    #[test]
    fn task_acceptance_wire_and_schema_reject_host_revision_and_invalid_criteria() {
        let spec = serde_json::to_value(describe()).unwrap();
        let invalid_tasks = [
            json!({"title": "Task", "acceptance_revision": 99}),
            json!({"title": "Task", "acceptance": [{
                "id": "a", "scenario": "Result", "channel": "command", "essential": true,
                "acceptance_revision": 99
            }]}),
            json!({"title": "Task", "acceptance": [{
                "id": "a", "scenario": "Result", "channel": "command"
            }]}),
            json!({"title": "Task", "acceptance": [{
                "id": "a", "scenario": "Result", "channel": "model", "essential": true
            }]}),
        ];
        for kind in ["create_task", "update_task"] {
            let schema = &spec["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == kind)
                .unwrap()["input"];
            for task in &invalid_tasks {
                let mut value = json!({"type": kind, "task": task});
                if kind == "update_task" {
                    value["task_id"] = json!("task-1");
                    value["expected_planning_revision"] = json!(7);
                }
                assert!(
                    serde_json::from_value::<KernelCommand>(value.clone()).is_err(),
                    "{value}"
                );
                assert!(
                    !schema_errors(schema, &value, &spec["$defs"], kind).is_empty(),
                    "{value}"
                );
            }
            let mut value =
                json!({"type": kind, "task": {"title": "Task"}, "acceptance_revision": 99});
            if kind == "update_task" {
                value["task_id"] = json!("task-1");
                value["expected_planning_revision"] = json!(7);
            }
            assert!(serde_json::from_value::<KernelCommand>(value.clone()).is_err());
            assert!(!schema_errors(schema, &value, &spec["$defs"], kind).is_empty());
        }
        assert!(serde_json::from_value::<KernelCommand>(json!({
            "type": "update_task", "task_id": "task-1", "task": {"title": "Task", "acceptance": []}
        })).is_err());
    }
}
