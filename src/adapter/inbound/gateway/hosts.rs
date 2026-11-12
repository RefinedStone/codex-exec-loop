//! Gateway 가 중계하는 app-server 호스트의 목록이다 (plan 25 S1a). 접속은 WebSocket 요청 경로 `/host/<id>` 로
//! 호스트를 고른다. 경로가 없으면 `local` 이다. 요청마다 라우팅하지 않으므로 중계 코드는 호스트를 모른다.

use std::sync::Arc;

use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamSnapshot,
};

/// 이 컴퓨터의 codex app-server 이다. kernel 과 기록은 이 호스트에만 적용한다.
pub(super) const LOCAL_HOST_ID: &str = "local";
const HOST_PATH_PREFIX: &str = "/host/";

#[derive(Clone)]
pub(super) struct GatewayHosts {
    /// 첫 항목은 항상 `local` 이다.
    hosts: Vec<(String, Arc<dyn GatewayUpstreamPort>)>,
}

impl GatewayHosts {
    pub(super) fn new(
        local: Arc<dyn GatewayUpstreamPort>,
        others: Vec<(String, Arc<dyn GatewayUpstreamPort>)>,
    ) -> Self {
        let mut hosts = vec![(LOCAL_HOST_ID.to_string(), local)];
        hosts.extend(others);
        Self { hosts }
    }

    pub(super) fn find(&self, id: &str) -> Option<Arc<dyn GatewayUpstreamPort>> {
        self.hosts
            .iter()
            .find(|(host_id, _)| host_id == id)
            .map(|(_, upstream)| upstream.clone())
    }

    pub(super) fn snapshots(&self) -> Vec<(String, GatewayUpstreamSnapshot)> {
        self.hosts
            .iter()
            .map(|(id, upstream)| (id.clone(), upstream.snapshot()))
            .collect()
    }

    pub(super) fn shutdown(&self) {
        for (_, upstream) in &self.hosts {
            upstream.shutdown();
        }
    }
}

/// 요청 경로가 고른 호스트 id 이다. `/host/` 뒤가 비어 있으면 빈 id 이고, 목록에 없으므로 접속이 닫힌다.
pub(super) fn host_id_from_path(path: &str) -> &str {
    match path.strip_prefix(HOST_PATH_PREFIX) {
        Some(rest) => rest.split('/').next().unwrap_or_default(),
        None => LOCAL_HOST_ID,
    }
}

#[cfg(test)]
mod tests {
    use super::host_id_from_path;

    #[test]
    fn the_path_selects_the_host_and_other_paths_mean_local() {
        assert_eq!(host_id_from_path("/"), "local");
        assert_eq!(host_id_from_path("/rpc"), "local");
        assert_eq!(host_id_from_path("/host/local"), "local");
        assert_eq!(host_id_from_path("/host/cloud"), "cloud");
        assert_eq!(host_id_from_path("/host/cloud/rpc"), "cloud");
        assert_eq!(host_id_from_path("/host/"), "");
    }
}
