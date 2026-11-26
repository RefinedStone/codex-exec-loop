/*
 * MCP hub 도구의 쓰기 정책이다 (#2830, plan 21 "쓰기 정책"). hub 폴더의 `write-policy.toml` 이 서버마다 쓰기 도구를
 * 적는다. 쓰기 도구는 Desktop 승인 없이 실행하지 않는다. 적지 않은 도구는 읽기이고 지금처럼 바로 실행한다.
 *
 * ```toml
 * [servers.notion]
 * write = ["notion-create-pages", "notion-update-page"]
 *
 * [servers.linear]
 * write = ["*"]          # 이 서버의 모든 도구가 쓰기이다
 * read = ["list_issues"] # "*" 에서 뺄 읽기 도구
 * ```
 *
 * 이 모듈은 I/O 없이 파일 글을 해석하고 도구의 종류를 정한다.
 */
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

/// hub 폴더 안의 정책 파일 이름이다. Codex 의 `config.toml` 과 따로 둔다 (Codex 는 모르는 키를 받지 않을 수 있다).
pub const MCP_WRITE_POLICY_FILE: &str = "write-policy.toml";
const ALL_TOOLS: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerRule {
    #[serde(default)]
    write: Vec<String>,
    #[serde(default)]
    read: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpWritePolicy {
    #[serde(default)]
    servers: BTreeMap<String, ServerRule>,
}

impl McpWritePolicy {
    /// 파일이 없으면 `None` 을 받는다. 그때는 모든 도구가 읽기이다 (지금 동작).
    pub fn parse(text: Option<&str>) -> Result<Self> {
        match text {
            None => Ok(Self::default()),
            Some(text) => toml::from_str(text).context("the MCP write policy is not valid"),
        }
    }

    pub fn access(&self, server: &str, tool: &str) -> McpToolAccess {
        let Some(rule) = self.servers.get(server) else {
            return McpToolAccess::Read;
        };
        let named = |names: &[String]| names.iter().any(|name| name == tool);
        if named(&rule.read) {
            return McpToolAccess::Read;
        }
        if named(&rule.write) || rule.write.iter().any(|name| name == ALL_TOOLS) {
            McpToolAccess::Write
        } else {
            McpToolAccess::Read
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_tools_are_writes_and_read_overrides_a_wildcard() {
        let policy = McpWritePolicy::parse(Some(
            "[servers.notion]\nwrite = [\"notion-update-page\"]\n[servers.linear]\nwrite = [\"*\"]\nread = [\"list_issues\"]\n",
        ))
        .unwrap();
        assert_eq!(
            policy.access("notion", "notion-update-page"),
            McpToolAccess::Write
        );
        assert_eq!(
            policy.access("notion", "notion-search"),
            McpToolAccess::Read
        );
        assert_eq!(
            policy.access("linear", "create_issue"),
            McpToolAccess::Write
        );
        assert_eq!(policy.access("linear", "list_issues"), McpToolAccess::Read);
        assert_eq!(policy.access("other", "anything"), McpToolAccess::Read);
    }

    #[test]
    fn a_missing_file_keeps_every_tool_readable_and_a_broken_file_is_an_error() {
        assert_eq!(
            McpWritePolicy::parse(None)
                .unwrap()
                .access("notion", "notion-update-page"),
            McpToolAccess::Read
        );
        assert!(McpWritePolicy::parse(Some("[servers.notion]\nwrite = \"oops\"")).is_err());
        assert!(McpWritePolicy::parse(Some("[servers.notion]\nwrites = []")).is_err());
    }
}
