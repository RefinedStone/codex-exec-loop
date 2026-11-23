//! A headless trusted extension exercising the same registry as product packages.
use crate::application::service::kernel_extensions::registry::{
    ExtensionHandler, ExtensionRegistration,
};
use crate::application::service::kernel_extensions::{ExtensionContext, ExtensionJobContext};
use crate::domain::kernel_extension::{
    EXTENSION_API_VERSION, ExtensionDescriptor, ExtensionOperation, ExtensionOperationKind,
    ExtensionScope,
};
use anyhow::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EchoInput {
    #[schemars(length(max = 1024))]
    text: String,
}
#[derive(Serialize, JsonSchema)]
struct EchoOutput {
    text: String,
    repository: bool,
    thread_bound: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CountInput {
    #[schemars(range(min = 1, max = 100))]
    steps: u32,
    #[schemars(range(max = 25))]
    delay_ms: u64,
}
#[derive(Serialize, JsonSchema)]
struct CountOutput {
    completed: u32,
}
fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema")
}
pub(super) fn registration(scope: ExtensionScope) -> ExtensionRegistration {
    ExtensionRegistration {
        manifest: ExtensionDescriptor {
            id: "akra.example".into(),
            version: 1,
            api_version: EXTENSION_API_VERSION,
            scope,
            label: "화면 없는 예제".into(),
            panel: None,
            permissions: vec!["example.run".into()],
            operations: vec![
                ExtensionOperation {
                    name: "echo".into(),
                    kind: ExtensionOperationKind::Query,
                    input_schema: schema::<EchoInput>(),
                    output_schema: schema::<EchoOutput>(),
                    progress_schema: None,
                },
                ExtensionOperation {
                    name: "count".into(),
                    kind: ExtensionOperationKind::Job,
                    input_schema: schema::<CountInput>(),
                    output_schema: schema::<CountOutput>(),
                    progress_schema: Some(schema::<CountOutput>()),
                },
            ],
        },
        handler: Arc::new(ExampleHandler),
    }
}
struct ExampleHandler;
impl ExtensionHandler for ExampleHandler {
    fn operations(&self) -> &'static [&'static str] {
        &["echo", "count"]
    }
    fn invoke(&self, context: &ExtensionContext, _action: &str, input: Value) -> Result<Value> {
        let input: EchoInput = serde_json::from_value(input)?;
        Ok(serde_json::to_value(EchoOutput {
            text: input.text,
            repository: context.repository_key.is_some(),
            thread_bound: context.verified_thread.is_some(),
        })?)
    }
    fn run(&self, context: &ExtensionJobContext, _action: &str, input: Value) -> Result<Value> {
        let input: CountInput = serde_json::from_value(input)?;
        for completed in 1..=input.steps {
            ensure!(
                !context
                    .cancel_flag()
                    .load(std::sync::atomic::Ordering::Acquire),
                "example job cancelled"
            );
            if input.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(input.delay_ms));
            }
            context.report_progress(json!({"completed":completed}))?;
        }
        Ok(json!({"completed":input.steps}))
    }
    fn dedupe_key(&self, action: &str, input: &Value) -> Result<Option<String>> {
        Ok(Some(format!("{action}:{}", serde_json::to_string(input)?)))
    }
}
