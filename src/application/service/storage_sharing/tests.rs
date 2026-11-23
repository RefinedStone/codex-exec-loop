use super::*;
use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
use crate::domain::storage_analysis::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

struct Fixture {
    nodes: Vec<StorageNode>,
    barrier: Mutex<Option<(Arc<Barrier>, Arc<Barrier>)>>,
    snapshot_barrier: Mutex<Option<(Arc<Barrier>, Arc<Barrier>)>>,
}
impl Fixture {
    fn contains(&self, node: &str, root: &str) -> bool {
        let mut current = Some(node);
        for _ in 0..32 {
            let Some(id) = current else { return false };
            if id == root {
                return self.nodes.iter().any(|n| n.id == id);
            }
            current = self
                .nodes
                .iter()
                .find(|n| n.id == id)
                .and_then(|n| n.parent_id.as_deref());
        }
        false
    }
    fn read(&self, node: &str, root: Option<&str>) -> Result<StorageNode> {
        if root.is_some_and(|root| !self.contains(node, root)) {
            bail!("private C:/outside");
        }
        self.nodes
            .iter()
            .find(|n| n.id == node)
            .cloned()
            .context("missing private path")
    }
}
impl StorageAnalysisPort for Fixture {
    fn list_volumes(&self) -> Result<Vec<StorageVolume>> {
        Ok(vec![])
    }
    fn scan(
        &self,
        _: &ScanRequest,
        _: &AtomicBool,
        _: &(dyn Fn(ScanProgress) + Sync),
    ) -> Result<StorageSnapshot> {
        bail!("fixture never scans")
    }
    fn snapshot(&self, snapshot: &str) -> Result<StorageSnapshot> {
        let barriers = self.snapshot_barrier.lock().unwrap().take();
        if let Some((entered, release)) = barriers {
            entered.wait();
            release.wait();
        }
        ensure!(snapshot == "snapshot", "snapshot gone");
        Ok(StorageSnapshot {
            schema_version: 1,
            id: snapshot.into(),
            revision: 7,
            volume_id: "volume".into(),
            root_node_id: "volume".into(),
            state: StorageState::Complete,
            started_at: "2026-11-23T00:00:00Z".into(),
            finished_at: Some("2026-11-23T00:00:01Z".into()),
            files_scanned: 3,
            directories_scanned: 3,
            issues: vec![],
        })
    }
    fn get_node(&self, _: &str, node: &str, root: Option<&str>) -> Result<StorageNode> {
        let answer = self.read(node, root)?;
        let barriers = self.barrier.lock().unwrap().take();
        if let Some((entered, release)) = barriers {
            entered.wait();
            release.wait();
        }
        Ok(answer)
    }
    fn list_children(&self, query: &StorageQuery) -> Result<StoragePage<StorageNode>> {
        let offset = query.cursor.as_deref().unwrap_or("0").parse::<usize>()?;
        let nodes = self
            .nodes
            .iter()
            .filter(|n| n.parent_id.as_deref() == Some(&query.node_id))
            .filter(|n| {
                query
                    .scope_root_id
                    .as_ref()
                    .is_none_or(|root| self.contains(&n.id, root))
            })
            .cloned()
            .collect::<Vec<_>>();
        let items = nodes
            .iter()
            .skip(offset)
            .take(query.limit)
            .cloned()
            .collect::<Vec<_>>();
        let next = offset + items.len();
        Ok(StoragePage {
            snapshot_id: query.snapshot_id.clone(),
            revision: 7,
            items,
            total_count: Some(nodes.len() as u64),
            next_cursor: (next < nodes.len()).then(|| next.to_string()),
        })
    }
    fn list_files(&self, query: &StorageQuery) -> Result<StoragePage<StorageNode>> {
        self.list_children(query)
    }
    fn find_hotspots(
        &self,
        query: &StorageQuery,
        _: &HotspotThresholds,
    ) -> Result<StoragePage<StorageHotspot>> {
        let page = self.list_children(query)?;
        Ok(StoragePage {
            snapshot_id: page.snapshot_id,
            revision: page.revision,
            items: page
                .items
                .into_iter()
                .map(|node| StorageHotspot {
                    included_bytes: node.known_allocated_bytes.clone(),
                    provisional: false,
                    node,
                    role: "folder".into(),
                    reason: "leaf".into(),
                })
                .collect(),
            total_count: page.total_count,
            next_cursor: page.next_cursor,
        })
    }
    fn ancestors(&self, _: &str, node: &str, root: Option<&str>) -> Result<Vec<StorageNode>> {
        let mut n = self.read(node, root)?;
        let mut ancestors = vec![];
        while root != Some(n.id.as_str()) {
            let Some(parent) = n.parent_id.as_deref() else {
                break;
            };
            n = self.read(parent, root)?;
            ancestors.push(n.clone());
        }
        ancestors.reverse();
        Ok(ancestors)
    }
    fn is_descendant(&self, _: &str, node: &str, root: &str) -> Result<bool> {
        Ok(self.contains(node, root))
    }
    fn reveal_path(&self, _: &str, _: &str) -> Result<PathBuf> {
        bail!("shared reads never reveal")
    }
    fn cleanup(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

fn node(id: &str, parent: Option<&str>, bytes: u64, parent_bytes: u64) -> StorageNode {
    StorageNode {
        snapshot_id: "snapshot".into(),
        revision: 7,
        id: id.into(),
        parent_id: parent.map(str::to_owned),
        name: id.into(),
        kind: StorageNodeKind::Directory,
        allocated_bytes: Some(bytes.to_string()),
        known_allocated_bytes: bytes.to_string(),
        logical_bytes: bytes.to_string(),
        ratio: StorageRatio::new(Some(bytes), Some(parent_bytes), parent.is_none(), false),
        state: StorageState::Complete,
        issues: vec![],
        protection: vec![],
        classification: vec![],
        allocation_owner_id: None,
    }
}
fn setup() -> (Arc<StorageSharingService>, Arc<Fixture>, StorageShareCaller) {
    let gib = 1024_u64.pow(3);
    let fixture = Arc::new(Fixture {
        nodes: vec![
            node("volume", None, 24 * gib, 24 * gib),
            node("shared", Some("volume"), 12 * gib, 24 * gib),
            node("target", Some("shared"), 9 * gib, 12 * gib),
            node("assets", Some("shared"), 2 * gib, 12 * gib),
            node("direct", Some("shared"), gib, 12 * gib),
            node("outside", Some("volume"), 12 * gib, 24 * gib),
        ],
        barrier: Mutex::new(None),
        snapshot_barrier: Mutex::new(None),
    });
    let service = Arc::new(StorageSharingService::new(StorageAnalysisService::new(
        fixture.clone(),
    )));
    let caller = StorageShareCaller {
        host_id: "host".into(),
        activation_generation: 2,
        connection_id: "connection".into(),
        thread_id: "thread".into(),
        conversation_epoch: 1,
        current_epoch: Arc::new(AtomicU64::new(1)),
    };
    (service, fixture, caller)
}
fn grant(service: &StorageSharingService, caller: &StorageShareCaller) -> StorageShareGrant {
    let preview = service
        .preview(
            caller,
            SharePreviewRequest {
                snapshot_id: "snapshot".into(),
                node_id: "shared".into(),
                thread_id: caller.thread_id.clone(),
            },
        )
        .unwrap();
    service
        .create(
            caller,
            ShareCreateRequest {
                preview_id: preview.id,
                thread_id: caller.thread_id.clone(),
            },
        )
        .unwrap()
}
fn query(grant: &StorageShareGrant, limit: usize, cursor: Option<String>) -> SharedQueryRequest {
    SharedQueryRequest {
        grant_id: grant.id.clone(),
        node_id: grant.root_node_id.clone(),
        limit,
        cursor,
        sort: StorageSort::AllocatedDesc,
        name_filter: None,
        thresholds: None,
    }
}

#[test]
fn explicit_preview_is_required_and_shared_values_match_the_same_index() {
    let (service, _, caller) = setup();
    assert!(
        service
            .create(
                &caller,
                ShareCreateRequest {
                    preview_id: "invented".into(),
                    thread_id: "thread".into()
                }
            )
            .is_err()
    );
    let grant = grant(&service, &caller);
    let root = service
        .get_node(
            &caller,
            SharedNodeRequest {
                grant_id: grant.id.clone(),
                node_id: "shared".into(),
            },
        )
        .unwrap();
    assert!(root.node.parent_id.is_none());
    assert!(root.node.ratio.denominator_bytes.is_none());
    let child = service
        .get_node(
            &caller,
            SharedNodeRequest {
                grant_id: grant.id.clone(),
                node_id: "target".into(),
            },
        )
        .unwrap();
    assert_eq!(
        child.node.ratio.numerator_bytes.as_deref(),
        Some("9663676416")
    );
    assert_eq!(
        child.node.ratio.denominator_bytes.as_deref(),
        Some("12884901888")
    );
    assert_eq!(child.node.id, "target");
    let resolved = service
        .resolve(
            &caller,
            ResolveEvidenceRequest {
                reference: child.reference,
                thread_id: "thread".into(),
            },
        )
        .unwrap();
    assert_eq!(resolved.node_id, "target");
    assert_eq!(
        resolved
            .ancestors
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["shared"]
    );
}

#[test]
fn scope_host_thread_expiry_and_generation_fail_without_disclosing_paths() {
    let (service, _, caller) = setup();
    let grant = grant(&service, &caller);
    let outside = service
        .get_node(
            &caller,
            SharedNodeRequest {
                grant_id: grant.id.clone(),
                node_id: "outside".into(),
            },
        )
        .unwrap_err()
        .to_string();
    let missing = service
        .get_node(
            &caller,
            SharedNodeRequest {
                grant_id: grant.id.clone(),
                node_id: "absent".into(),
            },
        )
        .unwrap_err()
        .to_string();
    assert_eq!(outside, missing);
    assert!(!outside.contains("outside"));
    for altered in ["host", "thread", "generation"] {
        let mut wrong = caller.clone();
        match altered {
            "host" => wrong.host_id = "other".into(),
            "thread" => wrong.thread_id = "other".into(),
            _ => wrong.activation_generation += 1,
        }
        assert!(
            service
                .get_node(
                    &wrong,
                    SharedNodeRequest {
                        grant_id: grant.id.clone(),
                        node_id: "target".into()
                    }
                )
                .is_err()
        );
    }
    service
        .store
        .lock()
        .unwrap()
        .grants
        .get_mut(&grant.id)
        .unwrap()
        .value
        .expires_at_ms = 0;
    assert!(
        service
            .get_node(
                &caller,
                SharedNodeRequest {
                    grant_id: grant.id,
                    node_id: "target".into()
                }
            )
            .is_err()
    );
}

#[test]
fn revoke_during_a_query_is_checked_again_before_returning_data() {
    let (service, fixture, caller) = setup();
    let grant = grant(&service, &caller);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    *fixture.barrier.lock().unwrap() = Some((entered.clone(), release.clone()));
    let worker_service = service.clone();
    let worker_caller = caller.clone();
    let id = grant.id.clone();
    let worker = std::thread::spawn(move || {
        worker_service.get_node(
            &worker_caller,
            SharedNodeRequest {
                grant_id: id,
                node_id: "target".into(),
            },
        )
    });
    entered.wait();
    service.revoke(&caller, &grant.id).unwrap();
    release.wait();
    assert_eq!(worker.join().unwrap().unwrap_err().to_string(), UNAVAILABLE);
}

#[test]
fn signed_cursor_is_bound_to_grant_query_and_revision() {
    let (service, _, caller) = setup();
    let grant_a = grant(&service, &caller);
    let first = service
        .list_children(&caller, query(&grant_a, 1, None))
        .unwrap();
    assert_eq!(first.total_count, Some(3));
    let cursor = first.next_cursor.unwrap();
    let second = service
        .list_children(&caller, query(&grant_a, 1, Some(cursor.clone())))
        .unwrap();
    assert_ne!(first.items[0].node.id, second.items[0].node.id);
    let grant_b = grant(&service, &caller);
    assert!(
        service
            .list_children(&caller, query(&grant_b, 1, Some(cursor.clone())))
            .is_err()
    );
    let mut changed = query(&grant_a, 1, Some(cursor.clone()));
    changed.sort = StorageSort::Name;
    assert!(service.list_children(&caller, changed).is_err());
    assert!(
        service
            .list_children(&caller, query(&grant_a, 1, Some(format!("{cursor}x"))))
            .is_err()
    );
}

#[test]
fn response_budget_does_not_cut_identifiers_or_decimal_bytes() {
    let (_, _, caller) = setup();
    let mut nodes = vec![node("shared", None, u64::MAX, u64::MAX)];
    for i in 0..100 {
        let mut value = node(
            &format!("n{i}"),
            Some("shared"),
            9_007_199_254_740_993,
            u64::MAX,
        );
        value.name = "가".repeat(900);
        value.protection.push(StorageEvidence {
            code: "git_locked".into(),
            detail: "C:/outside/private-repo".into(),
            observed_at: "2026-11-23T00:00:00Z".into(),
        });
        nodes.push(value);
    }
    let fixture = Arc::new(Fixture {
        nodes,
        barrier: Mutex::new(None),
        snapshot_barrier: Mutex::new(None),
    });
    let service = StorageSharingService::new(StorageAnalysisService::new(fixture));
    let grant = grant(&service, &caller);
    let page = service
        .list_children(&caller, query(&grant, 100, None))
        .unwrap();
    assert!(page.response_limited);
    assert_eq!(page.items.len(), 20);
    assert!(page.next_cursor.is_some());
    assert_eq!(
        page.items[0].node.allocated_bytes.as_deref(),
        Some("9007199254740993")
    );
    let json = serde_json::to_string(&page).unwrap();
    assert!(json.len() < MAX_RESPONSE_BYTES);
    assert!(!json.contains("C:/outside"));
    assert!(
        page.items[0]
            .omitted_fields
            .contains(&"name_suffix".to_owned())
    );
}

#[test]
fn navigation_and_rescan_invalidate_old_requests_and_references() {
    let (service, _, caller) = setup();
    let grant = grant(&service, &caller);
    caller.current_epoch.fetch_add(1, Ordering::AcqRel);
    assert!(
        service
            .get_node(
                &caller,
                SharedNodeRequest {
                    grant_id: grant.id.clone(),
                    node_id: "target".into()
                }
            )
            .is_err()
    );
    let mut new_caller = caller.clone();
    new_caller.conversation_epoch = 2;
    service.invalidate();
    assert!(
        service
            .resolve(
                &new_caller,
                ResolveEvidenceRequest {
                    reference: grant.reference,
                    thread_id: "thread".into()
                }
            )
            .is_err()
    );
}

#[test]
fn shared_hotspots_only_count_the_granted_subtree_and_revoke_its_queries() {
    let (service, _, caller) = setup();
    let grant = grant(&service, &caller);
    let result = service
        .find_hotspots(&caller, query(&grant, 100, None))
        .unwrap();
    assert_eq!(result.total_count, Some(3));
    assert!(result.items.iter().all(|item| item.node.id != "outside"));
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item
                .hotspot
                .as_ref()
                .unwrap()
                .included_bytes
                .parse::<u64>()
                .unwrap())
            .sum::<u64>(),
        12 * 1024_u64.pow(3)
    );
    let revoke: ShareRevokeRequest =
        serde_json::from_value(serde_json::json!({"grant_id":grant.id})).unwrap();
    service.revoke(&caller, &revoke.grant_id).unwrap();
    assert!(
        service
            .find_hotspots(&caller, query(&grant, 10, None))
            .is_err()
    );
}

#[test]
fn a_scan_started_during_preview_prevents_late_share_creation() {
    let (service, fixture, caller) = setup();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    *fixture.barrier.lock().unwrap() = Some((entered.clone(), release.clone()));
    let worker_service = service.clone();
    let worker_caller = caller.clone();
    let worker = std::thread::spawn(move || {
        worker_service.preview(
            &worker_caller,
            SharePreviewRequest {
                snapshot_id: "snapshot".into(),
                node_id: "shared".into(),
                thread_id: worker_caller.thread_id.clone(),
            },
        )
    });
    entered.wait();
    service.invalidate();
    release.wait();
    assert!(worker.join().unwrap().is_err());
    assert!(service.store.lock().unwrap().previews.is_empty());
}

#[test]
fn cleaning_an_older_snapshot_does_not_revoke_a_current_share() {
    let (service, _, caller) = setup();
    let grant = grant(&service, &caller);
    service.invalidate_snapshot("an-older-job");
    let request = SharedNodeRequest {
        grant_id: grant.id.clone(),
        node_id: "shared".into(),
    };
    assert!(service.get_node(&caller, request.clone()).is_ok());
    service.invalidate_snapshot("snapshot");
    assert!(service.get_node(&caller, request).is_err());
}

#[test]
fn revoke_during_final_snapshot_validation_prevents_returning_data() {
    let (service, fixture, caller) = setup();
    let grant = grant(&service, &caller);
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    *fixture.snapshot_barrier.lock().unwrap() = Some((entered.clone(), release.clone()));
    let worker_service = service.clone();
    let worker_caller = caller.clone();
    let grant_id = grant.id.clone();
    let worker = std::thread::spawn(move || {
        worker_service.get_node(
            &worker_caller,
            SharedNodeRequest {
                grant_id,
                node_id: "target".into(),
            },
        )
    });
    entered.wait();
    service.revoke(&caller, &grant.id).unwrap();
    release.wait();
    assert_eq!(worker.join().unwrap().unwrap_err().to_string(), UNAVAILABLE);
}

#[test]
fn response_commit_guard_serializes_revoke_and_rechecks_conversation_epoch() {
    use std::sync::mpsc;
    use std::time::Duration;
    let (service, _, caller) = setup();
    let grant = grant(&service, &caller);
    let request = SharedNodeRequest {
        grant_id: grant.id.clone(),
        node_id: "target".into(),
    };
    let answer = serde_json::to_value(service.get_node(&caller, request.clone()).unwrap()).unwrap();
    let input = serde_json::to_value(request).unwrap();
    let guard = service
        .response_guard(&caller, "shared_get_node", &input, &answer)
        .unwrap()
        .unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker_service = service.clone();
    let worker_caller = caller.clone();
    let id = grant.id.clone();
    let revoking = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(worker_service.revoke(&worker_caller, &id))
            .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(40)).is_err());
    guard.validate().unwrap();
    caller.current_epoch.fetch_add(1, Ordering::AcqRel);
    assert!(guard.validate().is_err());
    drop(guard);
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    revoking.join().unwrap();
    assert!(
        service
            .response_guard(&caller, "shared_get_node", &input, &answer)
            .is_err()
    );
}

#[test]
fn hotspot_queries_reject_options_that_would_silently_change_the_partition() {
    let (service, _, caller) = setup();
    let grant = grant(&service, &caller);
    let mut invalid = query(&grant, 10, None);
    invalid.sort = StorageSort::Name;
    assert!(service.find_hotspots(&caller, invalid).is_err());
    let mut invalid = query(&grant, 10, None);
    invalid.name_filter = Some("target".into());
    assert!(service.find_hotspots(&caller, invalid).is_err());
    let result = service
        .find_hotspots(&caller, query(&grant, 10, None))
        .unwrap();
    assert!(
        result
            .items
            .iter()
            .all(|row| row.hotspot.as_ref().unwrap().reason == "leaf")
    );
}
