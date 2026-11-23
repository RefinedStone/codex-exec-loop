use super::*;
pub(super) const VIEWER_QUERY: &str = "query AkraLinearViewer { viewer { id name } }";
pub(super) const ISSUES_QUERY: &str = r#"query AkraLinearAssignedIssues {
  viewer {
    assignedIssues(first:30, orderBy:updatedAt, includeArchived:false,
      filter:{state:{type:{nin:["completed","canceled","duplicate"]}}}) {
      nodes {
        id identifier title url priority updatedAt branchName
        state { id name type } team { id key name } project { id name }
        cycle { id number name isActive }
      }
    }
  }
}"#;
pub(super) const DETAIL_QUERY: &str = r#"query AkraLinearIssue($id: String!) {
  issue(id:$id) {
    id identifier title url priority updatedAt branchName
    state { id name type } team { id key name } project { id name }
    cycle { id number name isActive } description
    comments(first:10, orderBy:createdAt) {
      nodes { body createdAt user { name } } pageInfo { hasNextPage }
    }
  }
}"#;
pub(super) fn id(value: &Value) -> CredentialResult<String> {
    let value = value
        .as_str()
        .filter(|value| valid_linear_id(value))
        .ok_or(ProviderCredentialError::Corrupt)?;
    Ok(value.to_owned())
}
fn timestamp(value: &Value) -> CredentialResult<String> {
    let original = bounded_text(value, 64)?;
    let parsed = chrono::DateTime::parse_from_rfc3339(&original)
        .map_err(|_| ProviderCredentialError::Corrupt)?;
    if parsed.timestamp_subsec_nanos() >= 1_000_000_000 {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(original)
}
fn issue(value: &Value) -> CredentialResult<LinearIssue> {
    object(value)?;
    let identifier = bounded_text(&value["identifier"], 128)?;
    if !valid_linear_identifier(&identifier) {
        return Err(ProviderCredentialError::Corrupt);
    }
    let url = bounded_text(&value["url"], 2048)?;
    if !valid_linear_issue_url(&url) {
        return Err(ProviderCredentialError::Corrupt);
    }
    let priority = value["priority"]
        .as_u64()
        .filter(|value| *value <= 4)
        .ok_or(ProviderCredentialError::Corrupt)? as u8;
    let state_type = bounded_text(&value["state"]["type"], 32)?;
    if ![
        "triage",
        "backlog",
        "unstarted",
        "started",
        "completed",
        "canceled",
        "duplicate",
    ]
    .contains(&state_type.as_str())
    {
        return Err(ProviderCredentialError::Corrupt);
    }
    let project = match &value["project"] {
        Value::Null => None,
        project => Some(NamedEntity {
            id: id(&project["id"])?,
            name: bounded_text(&project["name"], 1000)?,
        }),
    };
    let cycle = match &value["cycle"] {
        Value::Null => None,
        cycle => Some(LinearCycle {
            id: id(&cycle["id"])?,
            number: cycle["number"]
                .as_u64()
                .filter(|value| *value > 0 && *value <= MAX_SAFE_REVISION)
                .ok_or(ProviderCredentialError::Corrupt)?,
            name: nullable_text(&cycle["name"], 1000)?,
            is_active: cycle["isActive"]
                .as_bool()
                .ok_or(ProviderCredentialError::Corrupt)?,
        }),
    };
    Ok(LinearIssue {
        id: id(&value["id"])?,
        identifier,
        title: bounded_text(&value["title"], 2000)?,
        url,
        priority,
        updated_at: timestamp(&value["updatedAt"])?,
        branch_name: nullable_text(&value["branchName"], 2000)?,
        state: LinearState {
            id: id(&value["state"]["id"])?,
            name: bounded_text(&value["state"]["name"], 1000)?,
            r#type: state_type,
        },
        team: LinearTeam {
            id: id(&value["team"]["id"])?,
            key: bounded_text(&value["team"]["key"], 128)?,
            name: bounded_text(&value["team"]["name"], 1000)?,
        },
        project,
        cycle,
    })
}
fn nodes(value: &Value, maximum: usize) -> CredentialResult<&Vec<Value>> {
    value["nodes"]
        .as_array()
        .filter(|values| values.len() <= maximum)
        .ok_or(ProviderCredentialError::Corrupt)
}
pub(super) fn assigned_issues(value: &Value) -> CredentialResult<Vec<LinearIssue>> {
    let mut issues = nodes(&value["viewer"]["assignedIssues"], ISSUE_LIMIT)?
        .iter()
        .map(issue)
        .collect::<CredentialResult<Vec<_>>>()?;
    let unique: std::collections::HashSet<_> = issues.iter().map(|item| &item.id).collect();
    if unique.len() != issues.len() {
        return Err(ProviderCredentialError::Corrupt);
    }
    issues.retain(|item| {
        !["completed", "canceled", "duplicate"].contains(&item.state.r#type.as_str())
    });
    Ok(issues)
}
pub(super) fn prompt(value: &Value) -> CredentialResult<String> {
    let item = issue(value)?;
    let description = match &value["description"] {
        Value::Null => "(No description)",
        Value::String(text) => text,
        _ => return Err(ProviderCredentialError::Corrupt),
    };
    let mut lines = vec![
        "Work on this Linear issue.".to_owned(),
        format!("Issue: {} {}", item.identifier, item.title),
        format!("URL: {}", item.url),
        format!(
            "Branch: {}",
            item.branch_name.as_deref().unwrap_or("(none)")
        ),
    ];
    if item.branch_name.is_some() {
        lines.push("Create the working branch with this suggested name.".to_owned());
    }
    lines.extend([
        format!("State: {}", item.state.name),
        format!("Team: {}", item.team.name),
        format!(
            "Project: {}",
            item.project
                .as_ref()
                .map_or("(none)", |project| project.name.as_str())
        ),
        format!(
            "Cycle: {}",
            item.cycle.as_ref().map_or_else(
                || "(none)".to_owned(),
                |cycle| format!(
                    "{} {}",
                    cycle.number,
                    cycle.name.as_deref().unwrap_or_default()
                )
            )
        ),
        format!("Priority: {}", item.priority),
        String::new(),
        "Description:".to_owned(),
        clip(description, MAX_DESCRIPTION_CHARS),
    ]);
    let comments = nodes(&value["comments"], 10)?;
    if !comments.is_empty() {
        lines.extend([String::new(), "Recent comments (newest first):".to_owned()]);
        for comment in comments {
            let body = comment["body"]
                .as_str()
                .ok_or(ProviderCredentialError::Corrupt)?;
            let author = if comment["user"].is_null() {
                "Unknown author".to_owned()
            } else {
                bounded_text(&comment["user"]["name"], 1000)?
            };
            lines.push(clip(
                &format!("{author} ({}):\n{body}", timestamp(&comment["createdAt"])?),
                MAX_COMMENT_CHARS,
            ));
        }
    }
    if value["comments"]["pageInfo"]["hasNextPage"]
        .as_bool()
        .ok_or(ProviderCredentialError::Corrupt)?
    {
        lines.push("[Older comments omitted]".to_owned());
    }
    Ok(clip(&lines.join("\n"), MAX_PROMPT_CHARS))
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;
    #[test]
    fn linear_timestamps_preserve_precision_and_reject_leap_seconds() {
        let original = "2026-11-23T13:04:05.123456Z";
        assert_eq!(
            timestamp(&Value::String(original.to_owned())).unwrap(),
            original
        );
        assert!(timestamp(&Value::String("2026-11-23T13:04:60Z".to_owned())).is_err());
    }
}
