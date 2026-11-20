/*
 * Evidence Vault 의 잠금 해제 서비스이다 (#2422). 신뢰된 로컬 UI 의 사용자 확인을 받아, 이슈 한 건과 열람 세션
 * 한 개에 묶인 권한(`VaultUnlockGrant`)을 준다. 권한은 세션이 끝나면 무효이다. 세션은 이 프로세스의 메모리에만 있다.
 *
 * 이 서비스만 `VaultUserConfirmation` 을 만든다. 잠금 해제를 못 하면 이유(`VaultSharedCopyReason`)를 돌려주고,
 * 부르는 쪽은 공유본만 보여준다. 이 서비스는 암호문 레코드를 읽거나 쓰지 않는다. 그래서 취소와 실패는 레코드를
 * 바꾸지 않는다.
 *
 * 이 파일의 타입은 `Serialize` 를 구현하지 않는다. Kernel 이벤트와 Gateway 응답에 넣지 않는다.
 */
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::application::port::outbound::evidence_vault_port::{
    VaultKey, VaultRecordKey, VaultUserConfirmation,
};
use crate::application::port::outbound::vault_key_store_port::{
    VaultKeyStoreError, VaultKeyStorePort,
};
use crate::domain::issue_work::IssueReference;

/// ChaCha20-Poly1305 의 키 길이이다.
const VAULT_KEY_LEN: usize = 32;
const KEY_ID_DOMAIN: &[u8] = b"akra-evidence-vault-key-id";
/// 여러 서비스 인스턴스도 같은 프로세스의 OS 키 항목을 공유한다.
static KEY_INITIALIZATION: Mutex<()> = Mutex::new(());

/// 로컬 복원본을 여는 열람 세션 한 개이다. 이 프로세스 안에서만 뜻이 있다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultViewingSessionId(u64);

/// 신뢰된 로컬 UI 에서 사용자가 고른 답이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultUserDecision {
    Confirmed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultUnlockRequest {
    pub issue: IssueReference,
    pub session: VaultViewingSessionId,
    pub decision: VaultUserDecision,
}

/// 잠금 해제를 못 한 이유이다. 부르는 쪽은 공유본만 보여준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultSharedCopyReason {
    /// 이 환경에는 OS 키 저장소가 없다.
    KeyStoreUnsupported,
    /// 사용자가 확인이나 OS 인증을 취소했다.
    AuthenticationCancelled,
    /// 열람 세션이 끝났거나, 이 세션이 이 이슈의 권한을 받은 적이 없다.
    SessionExpired,
    /// 키 저장소에 키가 없다. 이 장치에서 만든 레코드는 복원할 수 없다.
    KeyLost,
    /// 키 저장소가 요청에 실패했다. 다시 시도할 수 있다.
    KeyStoreUnavailable,
}

/// 이슈 한 건과 열람 세션 한 개에 묶인 잠금 해제 권한이다. 키는 없다. 이 서비스만 만든다.
#[derive(Debug, PartialEq, Eq)]
pub struct VaultUnlockGrant {
    issue: IssueReference,
    session: VaultViewingSessionId,
}

impl VaultUnlockGrant {
    pub fn issue(&self) -> &IssueReference {
        &self.issue
    }

    pub fn session(&self) -> &VaultViewingSessionId {
        &self.session
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VaultUnlockOutcome {
    Unlocked(VaultUnlockGrant),
    SharedCopyOnly(VaultSharedCopyReason),
}

/// 살아 있는 권한으로 얻는 값이다: 복원 계약의 확인과 레코드 키이다.
#[derive(Debug)]
pub struct VaultUnlockedAccess {
    pub confirmation: VaultUserConfirmation,
    pub key: VaultRecordKey,
}

pub struct EvidenceVaultUnlockService {
    key_store: Arc<dyn VaultKeyStorePort>,
    next_session: AtomicU64,
    /// 살아 있는 세션과, 세션마다 권한을 받은 이슈이다.
    sessions: Mutex<HashMap<VaultViewingSessionId, HashSet<IssueReference>>>,
}

impl EvidenceVaultUnlockService {
    pub fn new(key_store: Arc<dyn VaultKeyStorePort>) -> Self {
        Self {
            key_store,
            next_session: AtomicU64::new(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// 열람 세션을 연다. 로컬 복원본 화면을 열 때 부른다.
    pub fn begin_session(&self) -> VaultViewingSessionId {
        let session = VaultViewingSessionId(self.next_session.fetch_add(1, Ordering::Relaxed));
        self.sessions().insert(session.clone(), HashSet::new());
        session
    }

    /// 열람 세션을 끝낸다. 이 세션의 권한은 모두 무효이다.
    pub fn end_session(&self, session: &VaultViewingSessionId) {
        self.sessions().remove(session);
    }

    /// 사용자의 답으로 이슈 한 건을 잠금 해제한다. 취소하면 키 저장소를 부르지 않는다.
    pub fn unlock(&self, request: VaultUnlockRequest) -> VaultUnlockOutcome {
        if !self.sessions().contains_key(&request.session) {
            return VaultUnlockOutcome::SharedCopyOnly(VaultSharedCopyReason::SessionExpired);
        }
        if request.decision == VaultUserDecision::Cancelled {
            return VaultUnlockOutcome::SharedCopyOnly(
                VaultSharedCopyReason::AuthenticationCancelled,
            );
        }
        if let Err(reason) = self.existing_key() {
            return VaultUnlockOutcome::SharedCopyOnly(reason);
        }
        // 키 저장소를 기다리는 동안 세션이 끝났을 수 있다.
        match self.sessions().get_mut(&request.session) {
            Some(issues) => {
                issues.insert(request.issue.clone());
                VaultUnlockOutcome::Unlocked(VaultUnlockGrant {
                    issue: request.issue,
                    session: request.session,
                })
            }
            None => VaultUnlockOutcome::SharedCopyOnly(VaultSharedCopyReason::SessionExpired),
        }
    }

    /// 살아 있는 권한으로 확인과 키를 얻는다. 키는 부를 때마다 키 저장소에서 읽고 이 서비스에 남기지 않는다.
    pub fn access(
        &self,
        grant: &VaultUnlockGrant,
    ) -> Result<VaultUnlockedAccess, VaultSharedCopyReason> {
        let granted = self
            .sessions()
            .get(&grant.session)
            .is_some_and(|issues| issues.contains(&grant.issue));
        if !granted {
            return Err(VaultSharedCopyReason::SessionExpired);
        }
        let key = self.existing_key()?;
        // OS 키 저장소 응답을 기다리는 동안 화면이 닫혔으면 권한을 새로 내주지 않는다.
        let granted = self
            .sessions()
            .get(&grant.session)
            .is_some_and(|issues| issues.contains(&grant.issue));
        if !granted {
            return Err(VaultSharedCopyReason::SessionExpired);
        }
        Ok(VaultUnlockedAccess {
            confirmation: VaultUserConfirmation::for_issue(grant.issue.clone()),
            key,
        })
    }

    /// 새 레코드를 암호화할 키를 준다. 키 저장소에 키가 없으면 새 키를 만들어 저장한다. 평문을 돌려주지 않으므로
    /// 사용자 확인이 필요 없다. 키를 잃은 뒤에 만든 새 키는 이전 레코드를 열지 못한다 (키 식별자가 다르다).
    pub fn sealing_key(&self) -> Result<VaultRecordKey, VaultSharedCopyReason> {
        // 조회, 최초 생성, 저장을 한 구간으로 묶는다. 키 저장 실패도 이 잠금을 놓은 뒤 재시도한다.
        let _initialization = KEY_INITIALIZATION
            .lock()
            .map_err(|_| VaultSharedCopyReason::KeyStoreUnavailable)?;
        if let Some(key) = self.key_store.load_key().map_err(shared_copy_reason)? {
            return Ok(record_key(key));
        }
        // 서로 다른 프로세스의 키 생성은 OS 키 저장소의 원자적 get-or-create 지원이 필요하다.
        let mut bytes = vec![0u8; VAULT_KEY_LEN];
        OsRng.fill_bytes(&mut bytes);
        let key = VaultKey::new(bytes);
        self.key_store.store_key(&key).map_err(shared_copy_reason)?;
        Ok(record_key(key))
    }

    fn existing_key(&self) -> Result<VaultRecordKey, VaultSharedCopyReason> {
        self.key_store
            .load_key()
            .map_err(shared_copy_reason)?
            .map(record_key)
            .ok_or(VaultSharedCopyReason::KeyLost)
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<VaultViewingSessionId, HashSet<IssueReference>>> {
        // 잠금 안에서 panic 할 코드가 없다. 그래도 poison 이면 값은 그대로 쓴다.
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn shared_copy_reason(error: VaultKeyStoreError) -> VaultSharedCopyReason {
    match error {
        VaultKeyStoreError::Unsupported => VaultSharedCopyReason::KeyStoreUnsupported,
        VaultKeyStoreError::AuthenticationCancelled => {
            VaultSharedCopyReason::AuthenticationCancelled
        }
        VaultKeyStoreError::Unavailable(_) => VaultSharedCopyReason::KeyStoreUnavailable,
    }
}

/// 키 식별자는 키의 SHA-256 지문의 앞 64 bit 이다. 레코드에 평문으로 남아도 키를 알려주지 않는다.
fn record_key(key: VaultKey) -> VaultRecordKey {
    let mut hasher = Sha256::new();
    hasher.update(KEY_ID_DOMAIN);
    hasher.update(key.expose());
    let mut digest = hasher.finalize();
    let key_id = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    digest.zeroize();
    VaultRecordKey {
        key_id: format!("device-{key_id}"),
        key,
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn issue(issue_id: &str) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: issue_id.to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn service_with_key() -> (Arc<FakeVaultKeyStoreAdapter>, EvidenceVaultUnlockService) {
        let store = Arc::new(FakeVaultKeyStoreAdapter::with_key(VaultKey::new(
            KEY.to_vec(),
        )));
        let service = EvidenceVaultUnlockService::new(store.clone());
        (store, service)
    }

    fn confirm(
        service: &EvidenceVaultUnlockService,
        session: &VaultViewingSessionId,
        issue_id: &str,
    ) -> VaultUnlockOutcome {
        service.unlock(VaultUnlockRequest {
            issue: issue(issue_id),
            session: session.clone(),
            decision: VaultUserDecision::Confirmed,
        })
    }

    fn grant(outcome: VaultUnlockOutcome) -> VaultUnlockGrant {
        match outcome {
            VaultUnlockOutcome::Unlocked(grant) => grant,
            VaultUnlockOutcome::SharedCopyOnly(reason) => panic!("not unlocked: {reason:?}"),
        }
    }

    fn shared_copy_only(reason: VaultSharedCopyReason) -> VaultUnlockOutcome {
        VaultUnlockOutcome::SharedCopyOnly(reason)
    }

    #[test]
    fn confirmed_unlock_gives_a_grant_bound_to_the_issue_and_session() {
        let (_store, service) = service_with_key();
        let session = service.begin_session();

        let grant = grant(confirm(&service, &session, "I_1"));
        let access = service.access(&grant).expect("the grant is live");

        assert_eq!(grant.issue(), &issue("I_1"));
        assert_eq!(grant.session(), &session);
        assert_eq!(access.confirmation.issue(), &issue("I_1"));
        assert_eq!(access.key.key.expose(), KEY);
        assert!(access.key.key_id.starts_with("device-"));
    }

    #[test]
    fn a_grant_does_not_open_another_issue_or_another_session() {
        let (_store, service) = service_with_key();
        let session = service.begin_session();
        let other_session = service.begin_session();
        grant(confirm(&service, &session, "I_1"));

        let other_issue = VaultUnlockGrant {
            issue: issue("I_2"),
            session: session.clone(),
        };
        let moved_session = VaultUnlockGrant {
            issue: issue("I_1"),
            session: other_session,
        };

        assert_eq!(
            service.access(&other_issue).unwrap_err(),
            VaultSharedCopyReason::SessionExpired
        );
        assert_eq!(
            service.access(&moved_session).unwrap_err(),
            VaultSharedCopyReason::SessionExpired
        );
    }

    #[test]
    fn a_grant_expires_when_its_session_ends() {
        let (_store, service) = service_with_key();
        let session = service.begin_session();
        let grant = grant(confirm(&service, &session, "I_1"));

        service.end_session(&session);

        assert_eq!(
            service.access(&grant).unwrap_err(),
            VaultSharedCopyReason::SessionExpired
        );
        assert_eq!(
            confirm(&service, &session, "I_1"),
            shared_copy_only(VaultSharedCopyReason::SessionExpired)
        );
    }

    #[test]
    fn access_rejects_a_session_ended_during_key_lookup() {
        struct EndingStore {
            on_load: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        }
        impl VaultKeyStorePort for EndingStore {
            fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
                if let Some(on_load) = self.on_load.lock().expect("hook lock").take() {
                    on_load();
                }
                Ok(Some(VaultKey::new(KEY.to_vec())))
            }
            fn store_key(&self, _key: &VaultKey) -> Result<(), VaultKeyStoreError> {
                panic!("access never stores a key");
            }
        }
        let store = Arc::new(EndingStore {
            on_load: Mutex::new(None),
        });
        let service = Arc::new(EvidenceVaultUnlockService::new(store.clone()));
        let session = service.begin_session();
        let grant = grant(confirm(&service, &session, "I_1"));
        let weak = Arc::downgrade(&service);
        *store.on_load.lock().expect("hook lock") = Some(Box::new(move || {
            weak.upgrade().expect("service").end_session(&session);
        }));

        assert_eq!(
            service.access(&grant).unwrap_err(),
            VaultSharedCopyReason::SessionExpired
        );
    }

    #[test]
    fn a_cancelled_confirmation_does_not_reach_the_key_store() {
        let (store, service) = service_with_key();
        let session = service.begin_session();

        let outcome = service.unlock(VaultUnlockRequest {
            issue: issue("I_1"),
            session,
            decision: VaultUserDecision::Cancelled,
        });

        assert_eq!(
            outcome,
            shared_copy_only(VaultSharedCopyReason::AuthenticationCancelled)
        );
        assert_eq!(store.load_count(), 0);
        assert_eq!(store.stored_key(), Some(VaultKey::new(KEY.to_vec())));
    }

    #[test]
    fn each_key_store_failure_gives_its_own_shared_copy_reason() {
        let cases = [
            (
                VaultKeyStoreError::Unsupported,
                VaultSharedCopyReason::KeyStoreUnsupported,
            ),
            (
                VaultKeyStoreError::AuthenticationCancelled,
                VaultSharedCopyReason::AuthenticationCancelled,
            ),
            (
                VaultKeyStoreError::Unavailable("locked".to_string()),
                VaultSharedCopyReason::KeyStoreUnavailable,
            ),
        ];
        for (failure, reason) in cases {
            let (store, service) = service_with_key();
            let session = service.begin_session();
            store.fail_with(Some(failure));

            assert_eq!(
                confirm(&service, &session, "I_1"),
                shared_copy_only(reason.clone())
            );
            assert_eq!(service.sealing_key().unwrap_err(), reason);
            // 실패는 저장된 키를 바꾸지 않는다.
            assert_eq!(store.stored_key(), Some(VaultKey::new(KEY.to_vec())));
        }
    }

    #[test]
    fn a_lost_key_gives_key_lost_and_revokes_live_grants() {
        let (store, service) = service_with_key();
        let session = service.begin_session();
        let grant = grant(confirm(&service, &session, "I_1"));

        store.lose_key();

        assert_eq!(
            service.access(&grant).unwrap_err(),
            VaultSharedCopyReason::KeyLost
        );
        assert_eq!(
            confirm(&service, &session, "I_2"),
            shared_copy_only(VaultSharedCopyReason::KeyLost)
        );
        // 잠금 해제는 새 키를 만들지 않는다.
        assert_eq!(store.stored_key(), None);
    }

    #[test]
    fn sealing_key_creates_one_device_key_and_reuses_it() {
        let store = Arc::new(FakeVaultKeyStoreAdapter::new());
        let service = EvidenceVaultUnlockService::new(store.clone());

        let first = service.sealing_key().expect("the key store works");
        let second = service.sealing_key().expect("the key store works");

        assert_eq!(first.key.expose().len(), VAULT_KEY_LEN);
        assert_eq!(first, second);
        assert_eq!(store.stored_key(), Some(first.key.clone()));
    }

    #[test]
    fn concurrent_sealing_requests_share_one_initial_key() {
        use std::sync::Barrier;
        use std::sync::atomic::AtomicUsize;

        struct CountingStore {
            key: Mutex<Option<VaultKey>>,
            stores: AtomicUsize,
        }
        impl VaultKeyStorePort for CountingStore {
            fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
                Ok(self.key.lock().expect("key lock").clone())
            }
            fn store_key(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
                self.stores.fetch_add(1, Ordering::SeqCst);
                *self.key.lock().expect("key lock") = Some(key.clone());
                Ok(())
            }
        }
        let store = Arc::new(CountingStore {
            key: Mutex::new(None),
            stores: AtomicUsize::new(0),
        });
        let start = Arc::new(Barrier::new(9));
        let workers = (0..8)
            .map(|_| {
                // Gateway 재조립으로 여러 서비스 인스턴스가 생겨도 같은 키를 사용한다.
                let service = EvidenceVaultUnlockService::new(store.clone());
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    service.sealing_key().expect("sealing key")
                })
            })
            .collect::<Vec<_>>();
        start.wait();
        let keys = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .collect::<Vec<_>>();
        assert!(keys.iter().all(|key| key == &keys[0]));
        assert_eq!(store.stores.load(Ordering::SeqCst), 1);
        assert_eq!(
            *store.key.lock().expect("key lock"),
            Some(keys[0].key.clone())
        );
    }

    #[test]
    fn a_key_made_after_a_loss_has_a_different_key_id() {
        let store = Arc::new(FakeVaultKeyStoreAdapter::new());
        let service = EvidenceVaultUnlockService::new(store.clone());
        let before = service.sealing_key().expect("the key store works");

        store.lose_key();
        let after = service.sealing_key().expect("the key store works");

        assert_ne!(before.key_id, after.key_id);
    }

    // 권한과 접근 값은 직렬화할 수 없다: Kernel 이벤트와 Gateway 응답에 넣을 수 없다.
    trait NotSerialize<Marker> {
        fn proven() {}
    }
    impl<T: ?Sized> NotSerialize<()> for T {}
    struct IsSerialize;
    impl<T: ?Sized + Serialize> NotSerialize<IsSerialize> for T {}

    #[test]
    fn unlock_types_are_not_serializable_and_hide_the_key() {
        <VaultUnlockGrant as NotSerialize<_>>::proven();
        <VaultUnlockedAccess as NotSerialize<_>>::proven();
        <VaultViewingSessionId as NotSerialize<_>>::proven();

        let (_store, service) = service_with_key();
        let session = service.begin_session();
        let access = service
            .access(&grant(confirm(&service, &session, "I_1")))
            .expect("the grant is live");

        assert!(!format!("{access:?}").contains("0123456789abcdef"));
    }
}
