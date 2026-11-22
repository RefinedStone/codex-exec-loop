//! Immutable, publication-safe work-order reference carried into the delivery queue.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorkLinkReference {
    pub url: String,
    #[serde(default)]
    pub close_on_merge: bool,
}

impl PlanningWorkLinkReference {
    pub fn validate(&self) -> Result<()> {
        if self.url.len() > 4096
            || self
                .url
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
            || self.url.contains(['<', '>', '(', ')', '[', ']'])
        {
            bail!("invalid publication link reference");
        }
        let Some((host, path)) = self
            .url
            .strip_prefix("https://")
            .and_then(|rest| rest.split_once('/'))
        else {
            bail!("invalid publication link reference");
        };
        if !host.contains('.')
            || host.split('.').any(|part| {
                part.is_empty()
                    || part.starts_with('-')
                    || part.ends_with('-')
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            || path.is_empty()
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_./~".contains(&byte))
            || path.split('/').any(|part| matches!(part, "." | ".."))
        {
            bail!("invalid publication link reference");
        }
        if self.close_on_merge {
            let Some(path) = self.url.strip_prefix("https://github.com/") else {
                bail!("provider_link requires a canonical GitHub issue URL");
            };
            let parts: Vec<_> = path.split('/').collect();
            let segment = |value: &str| {
                !value.is_empty()
                    && !matches!(value, "." | "..")
                    && value
                        .bytes()
                        .all(|ch| ch.is_ascii_alphanumeric() || b"-_.".contains(&ch))
            };
            if parts.len() != 4
                || !segment(parts[0])
                || !segment(parts[1])
                || parts[2] != "issues"
                || parts[3]
                    .parse::<u64>()
                    .ok()
                    .is_none_or(|number| number == 0 || number.to_string() != parts[3])
            {
                bail!("provider_link requires a canonical GitHub issue URL");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_link_rejects_instruction_injection_and_noncanonical_closing_targets() {
        for url in [
            "https://github.com/acme/project/issues/7\nCloses #9",
            "https://github.com/acme/project/issues/7?token=secret",
            "https://user@github.com/acme/project/issues/7",
            "https://github.com/acme/project/pull/7",
            "https://github.com/acme/project/issues/007",
            "https://github.com/acme/project/issues/0",
            "https://github.com/acme/project/issues/7/",
            "https://example.com/acme/project/issues/7",
        ] {
            assert!(
                PlanningWorkLinkReference {
                    url: url.into(),
                    close_on_merge: true
                }
                .validate()
                .is_err(),
                "{url}"
            );
        }
        assert!(
            PlanningWorkLinkReference {
                url: "https://github.com/acme/project/issues/7".into(),
                close_on_merge: true,
            }
            .validate()
            .is_ok()
        );
    }
}
