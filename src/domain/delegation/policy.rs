use std::collections::BTreeMap;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Effect, EndReason};

pub(crate) const CHAT: &str = "chatgpt_chat";
pub(crate) const CODEX: &str = "codex_thread";
pub(crate) const OPENAI_TEXT: &str = "openai_text";
pub(crate) const DEFAULT_DEADLINE_MS: i64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Route {
    pub revision: String,
    pub backends: Vec<Backend>,
    pub fall_back_on: Vec<EndReason>,
    pub deadline_ms: i64,
    pub effect: Effect,
    pub max_running: u32,
    pub max_started_per_day: u32,
    #[serde(default)]
    pub requires_defensive_isolation: bool,
    #[serde(default)]
    pub pr_comments_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Backend {
    pub name: String,
    pub kind: String,
    pub model: Option<String>,
    pub max_concurrent: u32,
    #[serde(default)]
    pub max_resumes: Option<u32>,
}

impl Default for Route {
    fn default() -> Self {
        Self {
            revision: "builtin-v1".into(),
            backends: vec![Backend {
                name: CHAT.into(),
                kind: CHAT.into(),
                model: None,
                max_concurrent: 1,
                max_resumes: None,
            }],
            fall_back_on: vec![
                EndReason::Unavailable,
                EndReason::Quota,
                EndReason::Transient,
                EndReason::Interrupted,
            ],
            deadline_ms: DEFAULT_DEADLINE_MS,
            effect: Effect::ReadOnly,
            max_running: 2,
            max_started_per_day: 20,
            requires_defensive_isolation: false,
            pr_comments_allowed: false,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyFile {
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub backend: BTreeMap<String, BackendFile>,
    #[serde(default)]
    pub feature: BTreeMap<String, FeatureFile>,
    #[serde(default)]
    pub budget: BudgetFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BackendFile {
    pub kind: Option<String>,
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub tool_access: Option<ToolAccess>,
    pub session: Option<Session>,
    pub sandbox: Option<String>,
    pub max_concurrent: Option<u32>,
    pub max_resumes: Option<u32>,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolAccess {
    None,
    LocalRead,
    Tunnel,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Session {
    New,
    Temporary,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Off,
    Observe,
    On,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeatureFile {
    pub mode: Option<Mode>,
    pub route: Option<Vec<String>>,
    pub fall_back_on: Option<Vec<EndReason>>,
    pub deadline: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BudgetFile {
    pub max_running: Option<u32>,
    pub max_started_per_day: Option<u32>,
}

pub(crate) fn resolve(
    user: PolicyFile,
    repo: PolicyFile,
    feature: &str,
    revision: String,
) -> Result<Route> {
    let mut route = Route {
        requires_defensive_isolation: feature == "pr_review",
        ..Route::default()
    };
    for file in [&user, &repo] {
        ensure!(
            file.schema_version.is_none_or(|version| version == 1),
            "unsupported schema_version"
        );
        for (name, backend) in &file.backend {
            let kind = backend
                .kind
                .as_deref()
                .or_else(|| {
                    user.backend
                        .get(name)
                        .and_then(|backend| backend.kind.as_deref())
                })
                .unwrap_or(name);
            ensure!(
                matches!(kind, CHAT | CODEX | OPENAI_TEXT),
                "unknown backend kind"
            );
            ensure!(
                backend
                    .tool_access
                    .as_ref()
                    .is_none_or(|access| match kind {
                        CODEX => *access == ToolAccess::LocalRead,
                        _ => *access == ToolAccess::None,
                    }),
                "tool access is unsupported"
            );
            ensure!(
                backend
                    .session
                    .as_ref()
                    .is_none_or(|session| *session == Session::New),
                "temporary sessions are unsupported"
            );
            ensure!(
                backend
                    .sandbox
                    .as_deref()
                    .is_none_or(|sandbox| sandbox == "read-only"),
                "only the read-only sandbox is supported"
            );
            ensure!(
                backend.max_resumes.is_none_or(|limit| limit <= 2),
                "recovery limit exceeds backend maximum"
            );
        }
    }
    route.revision = revision;
    let user_feature = user.feature.get(feature);
    let repo_feature = repo.feature.get(feature);
    let user_mode = user_feature.and_then(|f| f.mode).unwrap_or(Mode::On);
    let mode = repo_feature.and_then(|f| f.mode).unwrap_or(user_mode);
    route.pr_comments_allowed = feature == "pr_review" && mode == Mode::On;
    ensure!(
        mode <= user_mode
            && (mode == Mode::On || (feature == "pr_review" && mode == Mode::Observe)),
        "feature cannot dispatch"
    );
    let draft = feature == super::ISSUE_DRAFT_REVIEW_FEATURE;
    let defaults = [if draft { CODEX } else { CHAT }.to_owned()];
    let user_order = user_feature
        .and_then(|f| f.route.as_deref())
        .unwrap_or(&defaults);
    let order = repo_feature
        .and_then(|f| f.route.as_deref())
        .unwrap_or(user_order);
    ensure!(order.len() <= 16, "route may contain at most 16 backends");
    route.backends.clear();
    for name in order {
        let user_backend = user.backend.get(name);
        let repo_backend = repo.backend.get(name);
        let enabled = user_backend
            .and_then(|b| b.enabled)
            .unwrap_or(name == CHAT || draft && name == CODEX);
        ensure!(
            !repo_backend.and_then(|b| b.enabled).unwrap_or(false) || enabled,
            "repository cannot enable a backend"
        );
        ensure!(
            user_order.contains(name),
            "repository cannot add a backend to the user route"
        );
        if !enabled || repo_backend.and_then(|b| b.enabled) == Some(false) {
            continue;
        }
        let kind = user_backend.and_then(|b| b.kind.as_deref()).unwrap_or(name);
        ensure!(
            kind != OPENAI_TEXT || feature == "pr_review",
            "model-only review backend is scoped to PR review"
        );
        ensure!(!draft || kind == CODEX, "issue draft review requires Codex");
        ensure!(
            repo_backend
                .and_then(|b| b.kind.as_deref())
                .is_none_or(|value| value == kind),
            "repository cannot change backend kind"
        );
        let max_concurrent = user_backend.and_then(|b| b.max_concurrent).unwrap_or(1);
        let model = user_backend
            .and_then(|b| b.model.clone())
            .filter(|value| !value.is_empty());
        ensure!(
            kind != OPENAI_TEXT
                || model.as_ref().is_some_and(|v| v.len() <= 128
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))),
            "model-only review requires an explicit model ID"
        );
        ensure!(
            repo_backend
                .and_then(|b| b.model.as_ref())
                .is_none_or(|value| Some(value) == model.as_ref()),
            "repository cannot change backend model"
        );
        ensure!(
            !route.backends.iter().any(|backend| backend.name == *name),
            "duplicate route backend"
        );
        route.backends.push(Backend {
            name: name.clone(),
            kind: kind.into(),
            model,
            max_concurrent: cap(max_concurrent, repo_backend.and_then(|b| b.max_concurrent))?,
            max_resumes: repo_backend
                .and_then(|b| b.max_resumes)
                .map(|limit| {
                    cap(
                        user_backend.and_then(|b| b.max_resumes).unwrap_or(2),
                        Some(limit),
                    )
                })
                .transpose()?
                .or_else(|| user_backend.and_then(|b| b.max_resumes)),
        });
    }
    ensure!(!route.backends.is_empty(), "no enabled route");
    route.max_running = cap(
        user.budget.max_running.unwrap_or(route.max_running),
        repo.budget.max_running,
    )?;
    route.max_started_per_day = cap(
        user.budget
            .max_started_per_day
            .unwrap_or(route.max_started_per_day),
        repo.budget.max_started_per_day,
    )?;
    if let Some(reasons) = repo_feature
        .and_then(|f| f.fall_back_on.clone())
        .or_else(|| user_feature.and_then(|f| f.fall_back_on.clone()))
    {
        route.fall_back_on = reasons;
    }
    if let Some(deadline) = repo_feature
        .and_then(|f| f.deadline.as_deref())
        .or_else(|| user_feature.and_then(|f| f.deadline.as_deref()))
    {
        let (number, factor) = if let Some(n) = deadline.strip_suffix('m') {
            (n, 60_000)
        } else if let Some(n) = deadline.strip_suffix('s') {
            (n, 1000)
        } else if let Some(n) = deadline.strip_suffix('h') {
            (n, 3_600_000)
        } else {
            anyhow::bail!("deadline must end in s, m, or h");
        };
        route.deadline_ms = number
            .parse::<i64>()?
            .checked_mul(factor)
            .filter(|value| (1000..=86_400_000).contains(value))
            .ok_or_else(|| anyhow::anyhow!("deadline must be between one second and one day"))?;
    }
    Ok(route)
}

fn cap(user: u32, repo: Option<u32>) -> Result<u32> {
    let effective = repo.unwrap_or(user);
    ensure!(
        effective <= user,
        "repository cannot increase a user budget"
    );
    Ok(effective)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> PolicyFile {
        toml::from_str(raw).unwrap()
    }

    #[test]
    fn delegation_repository_policy_may_reduce_user_limits_only() {
        let user = r#"
            [backend.remote]
            kind = "codex_thread"
            enabled = true
            [feature.manual]
            route = ["remote", "chatgpt_chat"]
            [budget]
            max_running = 3
        "#;
        let repo = r#"
            [feature.manual]
            route = ["remote"]
            deadline = "45s"
            [budget]
            max_running = 1
        "#;
        let route = resolve(parse(user), parse(repo), "manual", "v1".into()).unwrap();
        assert_eq!(route.backends[0].kind, CODEX);
        assert_eq!(route.max_running, 1);
        assert_eq!(route.deadline_ms, 45_000);
        assert_eq!(route.revision, "v1");
        for repo in [
            "[budget]\nmax_running = 4",
            "[backend.chatgpt_chat]\ntool_access = \"tunnel\"",
            "[feature.manual]\nroute = [\"codex_thread\"]",
        ] {
            assert!(resolve(parse(user), parse(repo), "manual", "v2".into()).is_err());
        }
    }

    #[test]
    fn delegation_pr_review_cannot_remove_its_isolation_requirement() {
        let source = "[feature.pr_review]\nmode = \"on\"\nroute = [\"chatgpt_chat\"]";
        let route = resolve(parse(source), parse(""), "pr_review", "v1".into()).unwrap();
        assert!(route.requires_defensive_isolation);
    }
}
