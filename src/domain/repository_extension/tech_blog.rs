//! Machine schemas for the bundled Python studio's public editing requests (version 3).
// Keep permissive extra fields and optional defaults from tech_blog_studio.py.
// Runtime existence, revision conflicts, commit membership and PNG content remain worker checks.
// Activation, job receipts and job cancellation belong to the common extension host.
use crate::domain::kernel_extension::{ExtensionOperation, ExtensionOperationKind};
use serde_json::{Value, json};

pub(crate) const ID: &str = "akra.tech-blog";
pub(crate) const VERSION: u32 = 3;
/// Model jobs: the host runs the worker stages until none remain.
pub(crate) const JOBS: [&str; 4] = ["generate", "resume", "revise", "review"];

pub(crate) fn operations() -> Vec<ExtensionOperation> {
    let id = text(64, false);
    let revision = json!({"type":"integer","minimum":1});
    let article_key = json!({"id":id,"revision":revision});
    let mut entries = vec![
        operation(
            "targets",
            ExtensionOperationKind::Query,
            json!({"limit":{"type":"integer","minimum":1,"maximum":500,"default":50}}),
            &[],
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
            "seen",
            ExtensionOperationKind::Command,
            json!({"id":id}),
            &["id"],
            object(),
        ),
        operation(
            "generate",
            ExtensionOperationKind::Job,
            json!({
                "commits":{
                    "type":"array","minItems":1,"maxItems":30,"uniqueItems":true,
                    "items":{
                        "type":"string","minLength":40,"maxLength":40,
                        "pattern":"^[a-f0-9]{40}$"
                    }
                },
                "topic":text(300,true)
            }),
            &["commits"],
            object(),
        ),
        // Stops the article's running stages. The host `cancel {job_id}` has the same effect.
        operation(
            "stop",
            ExtensionOperationKind::Command,
            json!({"id":id}),
            &["id"],
            object(),
        ),
        operation(
            "resume",
            ExtensionOperationKind::Job,
            article_key.clone(),
            &["id", "revision"],
            object(),
        ),
    ];
    let mut revise = article_key.clone();
    revise["instruction"] = text(1_000, true);
    entries.push(operation(
        "revise",
        ExtensionOperationKind::Job,
        revise,
        &["id", "revision"],
        object(),
    ));
    entries.push(operation(
        "review",
        ExtensionOperationKind::Job,
        article_key.clone(),
        &["id", "revision"],
        object(),
    ));
    let mut save = article_key.clone();
    save["draft"] = text(100_000, true);
    entries.push(operation(
        "save",
        ExtensionOperationKind::Command,
        save,
        &["id", "revision"],
        object(),
    ));
    entries.push(operation(
        "restore",
        ExtensionOperationKind::Command,
        article_key.clone(),
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
    entries.push(operation(
        "export",
        ExtensionOperationKind::Command,
        article_key,
        &["id", "revision"],
        object(),
    ));
    entries.push(operation(
        "delete",
        ExtensionOperationKind::Command,
        json!({"id":id}),
        &["id"],
        object(),
    ));
    entries
}

fn text(limit: usize, optional: bool) -> Value {
    let mut schema = json!({"type":"string","maxLength":limit});
    if optional {
        schema["default"] = json!("");
    } else {
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
    // A job reports the article it drives before its first stage finishes.
    let progress_schema = (kind == ExtensionOperationKind::Job).then(|| {
        json!({"type":"object","required":["article_id"],"properties":{
            "article_id":{"type":"string"},"stage":{"type":["string","null"]}
        }})
    });
    ExtensionOperation {
        name: name.into(),
        kind,
        input_schema: json!({"type":"object","properties":properties,"required":required}),
        output_schema,
        progress_schema,
    }
}
