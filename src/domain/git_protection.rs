//! Extra Akra destination rules; these do not change or replace server-side protections.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedBranchPolicy {
    pub reviewed: bool,
    pub rules: Vec<String>,
    pub disabled_automatic: Vec<String>,
}
impl ProtectedBranchPolicy {
    pub fn check(&self, branch: &str, automatic: &[String]) -> Result<()> {
        ensure!(
            self.reviewed,
            "the operator must review Akra branch protection first"
        );
        ensure!(
            !automatic
                .iter()
                .any(|b| b == branch && !self.disabled_automatic.contains(b)),
            "destination {branch} is automatically protected"
        );
        ensure!(
            !self.rules.iter().any(|r| matches_pattern(r, branch)),
            "destination {branch} matches a protected branch rule"
        );
        Ok(())
    }
}
pub fn validate_rule(rule: &str) -> Result<()> {
    ensure!(
        !rule.is_empty()
            && rule.len() <= 255
            && !rule.starts_with('-')
            && !rule.starts_with('/')
            && !rule.contains("..")
            && rule
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '*')),
        "invalid branch protection pattern"
    );
    Ok(())
}
fn matches_pattern(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let prefix = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let mut parts = parts.peekable();
    if parts.peek().is_none() {
        return rest.is_empty();
    }
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some((_, after)) = rest.split_once(part) else {
            return false;
        };
        rest = after;
    }
    true
}
