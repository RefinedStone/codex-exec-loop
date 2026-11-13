//! 커널 스펙이다 (plan 18 K4-1, 설계 `docs/plan/18-k4-command-spec.md` §3.1, §3.2).
//!
//! `akra/kernel/describe` 의 답이고, `schema/akra-kernel-spec.json` 으로 내보내는 값이다. 이름과 뜻은 표에서 온다:
//! 명령·기록·이유는 domain 의 표, 메서드·조회·이벤트·오류는 이 adapter 의 표이다. 모양(JSON Schema)은 전송 타입에서
//! 만든다. 그래서 여기에 손으로 적는 목록은 없다.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{
    AttachRequest, AttachResponse, CommandRequest, CommandResponse, CommandResult,
    EventNotification, KernelCommand, KernelErrorKind, KernelEvent, KernelEventKind,
    KernelMethodKind, KernelQuery, KernelQueryAnswer, KernelQueryKind, NextStep, PROTOCOL_VERSION,
    QueryRequest, error_next, reason_next,
};
use crate::domain::kernel_event::KernelEvent as KernelRecord;
use crate::domain::kernel_spec::{
    CommandCompletion, CommandEffect, CommandIdempotency, CommandScope, CommandState,
    KernelCommandKind, KernelRecordKind, OperationState, ReasonCode,
};
use crate::domain::parallel_mode::ParallelModeAgentRosterState;

/// 스키마의 방언이다. newtype variant 의 스키마는 `$ref` 옆에 `properties` 를 둔다. 그것은 이 방언에서만 맞다.
const SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/*
 * 자기 설명이다 (plan 18 §4.3). 프로토콜의 이름, 뜻, 모양을 담는다. 필드마다의 설명은 아직 없다 (K4 설계 §8).
 */
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KernelSpec {
    /// 이 스펙 안의 모든 스키마가 따르는 JSON Schema 방언이다.
    #[serde(rename = "$schema")]
    pub schema_dialect: &'static str,
    pub protocol_version: u32,
    /// 명령의 `scope`, `effect`, `idempotency` 가 쓰는 값과 그 뜻이다.
    pub terms: Terms,
    pub methods: Vec<MethodEntry>,
    pub commands: Vec<CommandEntry>,
    pub queries: Vec<QueryEntry>,
    pub events: Vec<EventEntry>,
    pub records: Vec<RecordEntry>,
    pub reasons: Vec<ReasonCodeEntry>,
    pub errors: Vec<ErrorEntry>,
    /// 스키마가 `#/$defs/<이름>` 으로 가리키는 타입이다.
    #[serde(rename = "$defs")]
    pub definitions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Terms {
    pub scope: Vec<TermEntry>,
    pub state: Vec<TermEntry>,
    pub effect: Vec<TermEntry>,
    pub idempotency: Vec<TermEntry>,
    /// 병렬 에이전트의 `state` 값이다 (plan 18 K4-2). 이름만으로 뜻을 알기 어려운 값이라서 설명을 싣는다.
    pub agent_state: Vec<TermEntry>,
    /// 명령의 `completion` 값이다 (plan 18 K4-4).
    pub completion: Vec<TermEntry>,
    /// 연산의 `state` 값이다 (plan 18 K4-4).
    pub operation_state: Vec<TermEntry>,
}

/// 메서드가 답을 받는 요청인가, 커널이 보내는 알림인가이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodDirection {
    Request,
    Notification,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MethodEntry {
    pub name: &'static str,
    pub kind: MethodDirection,
    pub summary: &'static str,
    /// 요청 또는 알림의 `params` 이다. 없는 메서드는 비어 있다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandEntry {
    pub name: &'static str,
    pub scope: CommandScope,
    pub state: CommandState,
    pub summary: &'static str,
    pub effect: CommandEffect,
    pub idempotency: CommandIdempotency,
    /// 커널이 이 명령을 바로 거절할 때 낼 수 있는 이유 코드이다. 뜻은 `reasons` 에 있다.
    pub rejections: Vec<&'static str>,
    /// 이 명령이 언제 끝나는가이다 (plan 18 K4-4). 뜻은 `terms.completion` 에 있다.
    pub completion: CommandCompletion,
    /// `akra/kernel/command` 의 `command` 값이다.
    pub input: Value,
    /// 모든 명령의 답은 같은 모양이다.
    pub output: Value,
    /// 성공한 저장 명령이 답의 `result` 에 싣는 값이다 (plan 18 K4-9d). 결과가 없는 명령은 이 항목이 없다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryEntry {
    pub name: &'static str,
    pub summary: &'static str,
    /// `akra/kernel/query` 의 `query` 값이다.
    pub input: Value,
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventEntry {
    pub name: &'static str,
    pub summary: &'static str,
    /// `akra/kernel/event` 의 `event` 값이다.
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordEntry {
    pub name: &'static str,
    pub summary: &'static str,
    /// `recorded` 이벤트와 `history` 조회의 `event` 값이다. 커널이 되돌릴 때만 쓰는 값은 싣지 않는다.
    pub payload: Value,
}

/// 이름과 뜻이다. `terms` 의 값이 이 모양이다.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TermEntry {
    pub name: &'static str,
    pub summary: &'static str,
}

/// 이유 코드 하나이다 (plan 18 K4-3). `next` 는 그 이유 뒤에 클라이언트가 차례대로 할 단계이다.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReasonCodeEntry {
    pub name: &'static str,
    pub summary: &'static str,
    pub next: Vec<NextStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorEntry {
    pub name: &'static str,
    /// JSON-RPC 오류 코드이다.
    pub code: i64,
    pub summary: &'static str,
    pub next: Vec<NextStep>,
}

/// `akra/kernel/describe` 의 답이다.
pub fn describe() -> KernelSpec {
    let mut schemas = Schemas::default();
    let commands = schemas.variants::<KernelCommand>("type");
    let command_output = schemas.reference::<CommandResponse>();
    let command_results = schemas.variants::<CommandResult>("type");
    let query_inputs = schemas.variants::<KernelQuery>("type");
    let query_outputs = schemas.variants::<KernelQueryAnswer>("type");
    let events = schemas.variants::<KernelEvent>("type");
    let records = schemas.variants::<KernelRecord>("kind");
    let methods = KernelMethodKind::ALL
        .iter()
        .map(|method| {
            let (params, result) = match method {
                KernelMethodKind::Attach => (
                    Some(schemas.reference::<AttachRequest>()),
                    Some(schemas.reference::<AttachResponse>()),
                ),
                KernelMethodKind::Command => (
                    Some(schemas.reference::<CommandRequest>()),
                    Some(command_output.clone()),
                ),
                KernelMethodKind::Query => (
                    Some(schemas.reference::<QueryRequest>()),
                    Some(schemas.reference::<KernelQueryAnswer>()),
                ),
                KernelMethodKind::Event => (Some(schemas.reference::<EventNotification>()), None),
                KernelMethodKind::Describe | KernelMethodKind::Detach => (None, None),
            };
            MethodEntry {
                name: method.name(),
                kind: match method {
                    KernelMethodKind::Event => MethodDirection::Notification,
                    _ => MethodDirection::Request,
                },
                summary: method.summary(),
                params,
                result,
            }
        })
        .collect();
    let term = |name, summary| TermEntry { name, summary };
    KernelSpec {
        schema_dialect: SCHEMA_DIALECT,
        protocol_version: PROTOCOL_VERSION,
        terms: Terms {
            state: CommandState::ALL
                .iter()
                .map(|state| term(state.name(), state.summary()))
                .collect(),
            scope: CommandScope::ALL
                .iter()
                .map(|scope| term(scope.name(), scope.summary()))
                .collect(),
            effect: CommandEffect::ALL
                .iter()
                .map(|effect| term(effect.name(), effect.summary()))
                .collect(),
            idempotency: CommandIdempotency::ALL
                .iter()
                .map(|idempotency| term(idempotency.name(), idempotency.summary()))
                .collect(),
            agent_state: ParallelModeAgentRosterState::ALL
                .iter()
                .map(|state| term(state.name(), state.summary()))
                .collect(),
            completion: CommandCompletion::ALL
                .iter()
                .map(|completion| term(completion.name(), completion.summary()))
                .collect(),
            operation_state: OperationState::ALL
                .iter()
                .map(|state| term(state.name(), state.summary()))
                .collect(),
        },
        methods,
        commands: KernelCommandKind::ALL
            .iter()
            .map(|kind| {
                let spec = kind.spec();
                CommandEntry {
                    name: kind.name(),
                    scope: spec.scope,
                    state: spec.state,
                    summary: kind.summary(),
                    effect: spec.effect,
                    idempotency: spec.idempotency,
                    rejections: spec.rejections.iter().map(|reason| reason.name()).collect(),
                    completion: spec.completion,
                    input: schema_named(&commands, kind.name()),
                    output: command_output.clone(),
                    result: CommandResult::name_for(*kind)
                        .map(|name| schema_named(&command_results, name)),
                }
            })
            .collect(),
        queries: KernelQueryKind::ALL
            .iter()
            .map(|kind| QueryEntry {
                name: kind.name(),
                summary: kind.summary(),
                input: schema_named(&query_inputs, kind.name()),
                output: schema_named(&query_outputs, kind.name()),
            })
            .collect(),
        events: KernelEventKind::ALL
            .iter()
            .map(|kind| EventEntry {
                name: kind.name(),
                summary: kind.summary(),
                payload: schema_named(&events, kind.name()),
            })
            .collect(),
        records: KernelRecordKind::ALL
            .iter()
            .map(|kind| RecordEntry {
                name: kind.name(),
                summary: kind.summary(),
                payload: schema_named(&records, kind.name()),
            })
            .collect(),
        reasons: ReasonCode::ALL
            .iter()
            .map(|&reason| ReasonCodeEntry {
                name: reason.name(),
                summary: reason.summary(),
                next: reason_next(reason),
            })
            .collect(),
        errors: KernelErrorKind::ALL
            .iter()
            .map(|error| ErrorEntry {
                name: error.name(),
                code: error.code(),
                summary: error.summary(),
                next: error_next(*error),
            })
            .collect(),
        definitions: schemas.into_definitions(),
    }
}

/// 이름으로 찾은 스키마이다. 표와 타입이 어긋나면 `null` 이고, 시험이 그것을 잡는다.
fn schema_named(schemas: &BTreeMap<String, Value>, name: &str) -> Value {
    schemas.get(name).cloned().unwrap_or(Value::Null)
}

/// 전송 타입에서 JSON Schema 를 만든다. 여러 타입이 함께 쓰는 정의는 한 번만 담는다.
struct Schemas {
    generator: SchemaGenerator,
}

impl Default for Schemas {
    fn default() -> Self {
        Self {
            generator: SchemaSettings::draft2020_12().into_generator(),
        }
    }
}

impl Schemas {
    /// 그 타입을 가리키는 `$ref` 이다. 타입의 정의는 `$defs` 에 들어간다.
    fn reference<T: JsonSchema>(&mut self) -> Value {
        without_code_comments(self.generator.subschema_for::<T>().to_value())
    }

    /// tag 가 있는 enum 의 variant 마다 스키마를 준다. 키는 tag 의 값(전송되는 이름)이다.
    fn variants<T: JsonSchema>(&mut self, tag: &str) -> BTreeMap<String, Value> {
        self.reference::<T>();
        let name = T::schema_name();
        let definition = self
            .generator
            .definitions()
            .get(name.as_ref())
            .cloned()
            .unwrap_or(Value::Null);
        definition["oneOf"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|variant| {
                let name = variant["properties"][tag]["const"].as_str()?;
                Some((name.to_string(), without_code_comments(variant.clone())))
            })
            .collect()
    }

    fn into_definitions(mut self) -> Map<String, Value> {
        self.generator
            .take_definitions(true)
            .into_iter()
            .map(|(name, definition)| (name, without_code_comments(definition)))
            .collect()
    }
}

/*
 * 스키마에서 `description` 과 `title` 을 뺀다. schemars 는 Rust 의 문서 주석을 그 자리에 넣는다. 이 저장소의 주석은
 * 개발자가 읽는 글이고 계획 번호를 담는다. 스펙의 뜻은 표의 `summary` 만 싣는다. `properties` 바로 아래의 키는
 * 필드 이름이므로 지우지 않는다 (`title` 이라는 필드가 있다). `default`, `const`, `enum`, `examples` 의 값은
 * 스키마가 아니라 데이터이므로 건드리지 않는다.
 */
fn without_code_comments(schema: Value) -> Value {
    match schema {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(keyword, _)| keyword != "description" && keyword != "title")
                .map(|(keyword, value)| {
                    let value = match value {
                        data if matches!(
                            keyword.as_str(),
                            "default" | "const" | "enum" | "examples"
                        ) =>
                        {
                            data
                        }
                        Value::Object(properties) if keyword == "properties" => Value::Object(
                            properties
                                .into_iter()
                                .map(|(field, schema)| (field, without_code_comments(schema)))
                                .collect(),
                        ),
                        other => without_code_comments(other),
                    };
                    (keyword, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_code_comments).collect()),
        scalar => scalar,
    }
}
