use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct GraphFilter {
    pub branch: BranchScope,
    pub author: String,
    pub path: String,
    pub message: String,
    pub hide: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BranchScope {
    #[default]
    All,
    Local,
    Thread,
    Branch {
        name: String,
    },
}
impl GraphFilter {
    pub fn active(&self) -> bool {
        self.branch != BranchScope::All
            || !self.author.is_empty()
            || !self.path.is_empty()
            || !self.message.is_empty()
    }
    pub fn validate(&self) -> Result<()> {
        if [&self.author, &self.path, &self.message]
            .iter()
            .any(|v| v.len() > 1024 || v.contains('\0'))
        {
            bail!("invalid graph filter text");
        }
        if self.path.starts_with('/')
            || self.path.contains('\\')
            || self.path.split('/').any(|p| p == "..")
        {
            bail!("the path filter must be relative to the repository root");
        }
        if let BranchScope::Branch { name } = &self.branch
            && !(name.starts_with("refs/heads/") || name.starts_with("refs/remotes/"))
        {
            bail!("pick a local or remote branch");
        }
        Ok(())
    }
}
