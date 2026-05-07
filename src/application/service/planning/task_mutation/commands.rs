use serde::Deserialize;

use crate::domain::planning::TaskStatus;

/*
 * worker/LLM output에서 task mutation command만 엄격하게 뽑아내는 JSON extractor다.
 * mutation service는 typed create/update command만 소비한다. 자동 응답이 accepted DB authority 전체를
 * 한 번에 교체하지 못하게 하고, 모든 변경을 command 단위 audit/revision 경로에 태우기 위한 경계다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskCreateInput {
    // create의 optional field는 "service default 사용"이라는 뜻이다. actor, timestamp,
    // priority default, relation note fallback은 direction validation 이후 service가 채운다.
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskUpdateInput {
    // update의 optional field는 "기존 값 유지"라는 뜻이다. list field를 Option<Vec<_>>로 둬
    // no change와 dependency/blocker 명시적 clear를 구분한다.
    pub task_id: String,
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskMutationCommand {
    CreateTask(PlanningTaskCreateInput),
    UpdateTask(PlanningTaskUpdateInput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskCommandExtraction {
    Commands(Vec<PlanningTaskMutationCommand>),
    InvalidCommands {
        error: String,
        rejected_json: Option<String>,
    },
    None,
}

// serde-facing shape는 private이고 service-facing input보다 더 엄격하다. unknown field는 mutation
// logic이 실행되기 전에 extraction 단계에서 실패한다.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCommandsDocument {
    planning_task_commands: PlanningTaskCommandsEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCommandsEnvelope {
    version: u32,
    #[serde(default)]
    commands: Vec<PlanningTaskCommandEnvelope>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum PlanningTaskCommandEnvelope {
    CreateTask(PlanningTaskCreateCommand),
    UpdateTask(PlanningTaskUpdateCommand),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCreateCommand {
    direction_id: Option<String>,
    direction_relation_note: Option<String>,
    title: String,
    description: Option<String>,
    status: Option<TaskStatus>,
    base_priority: Option<i32>,
    dynamic_priority_delta: Option<i32>,
    priority_reason: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    blocked_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskUpdateCommand {
    task_id: String,
    direction_id: Option<String>,
    direction_relation_note: Option<String>,
    title: Option<String>,
    description: Option<String>,
    status: Option<TaskStatus>,
    base_priority: Option<i32>,
    dynamic_priority_delta: Option<i32>,
    priority_reason: Option<String>,
    depends_on: Option<Vec<String>>,
    blocked_by: Option<Vec<String>>,
}

pub fn extract_planning_task_commands(message: &str) -> PlanningTaskCommandExtraction {
    // 먼저 JSON 가능성이 높은 영역을 찾고, 마지막 후보로 전체 message도 시도한다. code fence가
    // 없는 raw JSON reply도 동작해야 하기 때문이다. 첫 valid command document가 승리하고,
    // invalid command document는 뒤에 valid candidate가 없을 때만 repair 증거로 남긴다.
    let mut first_invalid = None;
    for candidate in candidate_json_sections(message) {
        if candidate.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        if value.get("planning_task_commands").is_some() {
            // serde error와 함께 normalized rejected JSON을 보존한다. repair prompt는 원본 응답의
            // 깨지기 쉬운 slice가 아니라 pretty JSON을 기준으로 unknown field나 unsupported version을
            // 지적할 수 있다.
            let rejected_json =
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| candidate.to_string());
            match serde_json::from_value::<PlanningTaskCommandsDocument>(value) {
                Ok(document) => {
                    if document.planning_task_commands.version != 1 {
                        return PlanningTaskCommandExtraction::InvalidCommands {
                            error: format!(
                                "planning_task_commands version {} is not supported",
                                document.planning_task_commands.version
                            ),
                            rejected_json: Some(rejected_json),
                        };
                    }
                    return PlanningTaskCommandExtraction::Commands(
                        document
                            .planning_task_commands
                            .commands
                            .into_iter()
                            .map(PlanningTaskMutationCommand::from)
                            .collect(),
                    );
                }
                Err(error) => first_invalid = Some((error.to_string(), rejected_json)),
            }
        }
    }

    first_invalid
        .map(
            |(error, rejected_json)| PlanningTaskCommandExtraction::InvalidCommands {
                error,
                rejected_json: Some(rejected_json),
            },
        )
        .unwrap_or(PlanningTaskCommandExtraction::None)
}

impl From<PlanningTaskCommandEnvelope> for PlanningTaskMutationCommand {
    fn from(command: PlanningTaskCommandEnvelope) -> Self {
        // 변환은 private serde envelope를 벗기고 service가 쓰는 operation-specific input만 남긴다.
        // validation/defaulting은 repository와 catalog context가 필요하므로 downstream mutation service에 남긴다.
        match command {
            PlanningTaskCommandEnvelope::CreateTask(command) => {
                Self::CreateTask(PlanningTaskCreateInput {
                    direction_id: command.direction_id,
                    direction_relation_note: command.direction_relation_note,
                    title: command.title,
                    description: command.description,
                    status: command.status,
                    base_priority: command.base_priority,
                    dynamic_priority_delta: command.dynamic_priority_delta,
                    priority_reason: command.priority_reason,
                    depends_on: command.depends_on,
                    blocked_by: command.blocked_by,
                })
            }
            PlanningTaskCommandEnvelope::UpdateTask(command) => {
                Self::UpdateTask(PlanningTaskUpdateInput {
                    task_id: command.task_id,
                    direction_id: command.direction_id,
                    direction_relation_note: command.direction_relation_note,
                    title: command.title,
                    description: command.description,
                    status: command.status,
                    base_priority: command.base_priority,
                    dynamic_priority_delta: command.dynamic_priority_delta,
                    priority_reason: command.priority_reason,
                    depends_on: command.depends_on,
                    blocked_by: command.blocked_by,
                })
            }
        }
    }
}

fn candidate_json_sections(message: &str) -> Vec<&str> {
    // model은 command를 fenced block으로 감싸는 경우가 많지만, repair text는 prose 안에 bare
    // object를 넣기도 한다. markdown parser 의존성 없이 fenced/balanced/full message 후보를
    // 모두 모아 extractor가 모델 출력 변형에 둔감하게 만든다.
    let mut sections = Vec::new();
    let mut remainder = message;
    while let Some(start) = remainder.find("```") {
        remainder = &remainder[start + 3..];
        let body_start = remainder.find('\n').map(|index| index + 1).unwrap_or(0);
        let after_header = &remainder[body_start..];
        let Some(end) = after_header.find("```") else {
            break;
        };
        sections.push(after_header[..end].trim());
        remainder = &after_header[end + 3..];
    }
    sections.extend(balanced_json_object_sections(message));
    sections.push(message.trim());
    sections
}

fn balanced_json_object_sections(message: &str) -> Vec<&str> {
    // 이 scanner는 작지만 JSON string을 인식한다. prose에서 balanced top-level object를 뽑되
    // 문자열 안의 brace는 무시하므로, fragment를 command로 오인하지 않고 repair용 후보를 얻기에 충분하다.
    let mut sections = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in message.char_indices() {
        let Some(start_index) = start else {
            if character == '{' {
                start = Some(index);
                depth = 1;
                in_string = false;
                escaped = false;
            }
            continue;
        };
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    sections.push(message[start_index..index + character.len_utf8()].trim());
                    start = None;
                }
            }
            _ => {}
        }
    }

    sections
}
