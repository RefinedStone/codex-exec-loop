//! 완성된 페이지 집합만 반환한다. 일부 페이지로 부재나 유일성을 확정하지 않는다.
use std::collections::HashSet;

use anyhow::{Result, anyhow, bail};

use super::graphql::Connection;

#[derive(Default)]
pub(super) struct Pages {
    total: Option<usize>,
    ids: HashSet<String>,
    cursors: HashSet<String>,
}

impl Pages {
    pub(super) fn accept<T>(
        &mut self,
        page: Connection<T>,
        node_id: impl Fn(&T) -> &str,
    ) -> Result<(Vec<T>, Option<String>)> {
        if self.total.is_some_and(|total| total != page.total_count) {
            bail!("GitHub feedback pagination changed while it was being read")
        }
        self.total = Some(page.total_count);
        let nodes = page
            .nodes
            .ok_or_else(|| anyhow!("GitHub returned incomplete feedback pagination"))?;
        let nodes = nodes
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| anyhow!("GitHub returned incomplete feedback pagination"))?;
        if nodes.len() > 100 {
            bail!("GitHub returned inconsistent feedback pagination")
        }
        for node in &nodes {
            let id = node_id(node);
            if id.is_empty() || !self.ids.insert(id.to_string()) {
                bail!("GitHub returned repeated feedback pagination")
            }
        }
        if self.ids.len() > page.total_count {
            bail!("GitHub returned inconsistent feedback pagination")
        }
        if !page.page_info.has_next_page {
            if self.ids.len() != page.total_count {
                bail!("GitHub returned incomplete feedback pagination")
            }
            return Ok((nodes, None));
        }
        let cursor = page
            .page_info
            .end_cursor
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or_else(|| anyhow!("GitHub returned incomplete feedback pagination"))?;
        if nodes.is_empty()
            || self.ids.len() == page.total_count
            || !self.cursors.insert(cursor.clone())
        {
            bail!("GitHub returned repeated feedback pagination")
        }
        Ok((nodes, Some(cursor)))
    }
}
