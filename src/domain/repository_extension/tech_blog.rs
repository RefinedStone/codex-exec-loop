//! Machine schemas for the bundled Python studio's public editing requests.
// Keep permissive extra fields and optional defaults from tech_blog_studio.py.
// Runtime existence, revision conflicts, evidence paths and PNG content remain worker checks.
use crate::domain::kernel_extension::{ExtensionOperation, ExtensionOperationKind};
use serde_json::{Value, json};

pub(crate) fn operations() -> Vec<ExtensionOperation> {
    let id = text(64, false);
    let revision = json!({"type":"integer","minimum":1});
    let article_key = json!({"id":id,"revision":revision});
    let mut entries = vec![
        operation(
            "catalog",
            ExtensionOperationKind::Query,
            json!({}),
            &[],
            object(),
        ),
        operation(
            "configure",
            ExtensionOperationKind::Command,
            json!({"enabled":{"type":"boolean"}}),
            &["enabled"],
            object(),
        ),
        operation(
            "list",
            ExtensionOperationKind::Query,
            json!({}),
            &[],
            json!({"type":"array"}),
        ),
        operation(
            "open",
            ExtensionOperationKind::Query,
            json!({"id":id}),
            &["id"],
            object(),
        ),
        operation(
            "create",
            ExtensionOperationKind::Command,
            json!({
                "topic":text(300,false),
                "paths":{"type":"array","maxItems":4,"items":{"type":"string"}},
                "style":{"enum":["PROBLEM_SOLVING","TECHNICAL_DEEP_DIVE"]},
                "mode":{"enum":["PROJECT_RETROSPECTIVE","TECH_EXPLORATION"]}
            }),
            &["topic", "style", "mode"],
            object(),
        ),
        operation(
            "accept",
            ExtensionOperationKind::Command,
            article_key.clone(),
            &["id", "revision"],
            object(),
        ),
    ];
    let mut save = article_key.clone();
    save["draft"] = text(100_000, true);
    entries.push(operation(
        "save",
        ExtensionOperationKind::Command,
        save,
        &["id", "revision"],
        object(),
    ));
    let mut figure = article_key.clone();
    figure["png"] = text(2_800_000, false);
    figure["caption"] = text(300, false);
    entries.push(operation(
        "figure",
        ExtensionOperationKind::Command,
        figure,
        &["id", "revision", "png", "caption"],
        object(),
    ));
    let mut model = article_key;
    model["model"] = json!({
        "type":"string","minLength":1,"maxLength":100,
        "pattern":"^[A-Za-z0-9][A-Za-z0-9._:/-]*$"
    });
    model["instruction"] = text(4000, true);
    for action in ["write", "review"] {
        // These legacy commands return the article containing its already-created job.
        entries.push(operation(
            action,
            ExtensionOperationKind::Command,
            model.clone(),
            &["id", "revision", "model"],
            object(),
        ));
    }
    entries
}

fn text(limit: usize, optional: bool) -> Value {
    let mut schema = json!({"type":"string","maxLength":limit});
    if !optional {
        schema["minLength"] = json!(1);
        schema["pattern"] = json!("\\S");
    }
    schema
}
fn object() -> Value {
    json!({"type":"object"})
}
fn operation(
    name: &str,
    kind: ExtensionOperationKind,
    properties: Value,
    required: &[&str],
    output_schema: Value,
) -> ExtensionOperation {
    ExtensionOperation {
        name: name.into(),
        kind,
        input_schema: json!({"type":"object","properties":properties,"required":required}),
        output_schema,
        progress_schema: None,
    }
}
