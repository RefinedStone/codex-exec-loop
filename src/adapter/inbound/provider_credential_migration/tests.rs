use super::*;
use crate::application::port::outbound::provider_credentials_port::NativeOperationOwner;
use crate::domain::provider_credentials::*;
use crate::private_ipc::credential_frames as frames;
use protocol::{Hello, Request, Response, VERSION};
use std::io::{Cursor, Read};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::AsyncReadExt;

const CANARY: &[u8] = b"synthetic-migration-canary-token";
#[derive(Default)]
struct FakeService {
    receipt: Mutex<Option<OperationReceipt>>,
    prepared: AtomicUsize,
    imported: AtomicUsize,
    cancelled: AtomicUsize,
    block_prepare: AtomicBool,
    lose_import_ack: AtomicBool,
    fail_prepare_with_pending: AtomicBool,
}
impl FakeService {
    fn receipt(offer: &LegacyImportOffer, status: CredentialOperationStatus) -> OperationReceipt {
        OperationReceipt {
            operation_id: offer.operation_id.clone(),
            profile_id: offer.profile_id.clone(),
            provider: offer.provider,
            expected_revision: offer.expected_revision,
            kind: CredentialOperationKind::LegacyImport,
            status,
            connection: if status == CredentialOperationStatus::Committed {
                Some(ConnectionRef {
                    profile_id: offer.profile_id.clone(),
                    provider: offer.provider,
                    connection_id: "b".repeat(32),
                    revision: 1,
                })
            } else {
                None
            },
            cleanup_pending: false,
            legacy_snapshot_id: Some(offer.snapshot_id.clone()),
            legacy_cleanup_allowed: status == CredentialOperationStatus::Committed,
            error: None,
        }
    }
}
impl ProviderCredentialMigrationPort for FakeService {
    fn prepare_import(
        &self,
        offer: LegacyImportOffer,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt> {
        self.prepared.fetch_add(1, Ordering::SeqCst);
        while self.block_prepare.load(Ordering::SeqCst) && owner.is_live() {
            std::thread::sleep(Duration::from_millis(1));
        }
        if !owner.is_live() {
            return Err(ProviderCredentialError::Cancelled);
        }
        let receipt = Self::receipt(&offer, CredentialOperationStatus::Prepared);
        *self.receipt.lock().unwrap() = Some(receipt.clone());
        if self.fail_prepare_with_pending.load(Ordering::SeqCst) {
            Err(ProviderCredentialError::Unavailable)
        } else {
            Ok(receipt)
        }
    }
    fn import_secret(
        &self,
        offer: LegacyImportOffer,
        token: SecretToken,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt> {
        if !owner.is_live() {
            return Err(ProviderCredentialError::Cancelled);
        }
        assert_eq!(token.expose(), CANARY);
        self.imported.fetch_add(1, Ordering::SeqCst);
        let receipt = Self::receipt(&offer, CredentialOperationStatus::Committed);
        *self.receipt.lock().unwrap() = Some(receipt.clone());
        if self.lose_import_ack.load(Ordering::SeqCst) {
            Err(ProviderCredentialError::OutcomeUnknown)
        } else {
            Ok(receipt)
        }
    }
    fn migration_operation(
        &self,
        request: CredentialOperationRequest,
    ) -> CredentialResult<OperationReceipt> {
        self.receipt
            .lock()
            .unwrap()
            .clone()
            .filter(|receipt| {
                receipt.profile_id == request.profile_id
                    && receipt.provider == request.provider
                    && receipt.operation_id == request.operation_id
            })
            .ok_or(ProviderCredentialError::NotFound)
    }
    fn acknowledge_legacy_cleanup(
        &self,
        request: LegacyCleanupRequest,
    ) -> CredentialResult<OperationReceipt> {
        let mut store = self.receipt.lock().unwrap();
        let receipt = store.as_mut().ok_or(ProviderCredentialError::NotFound)?;
        if receipt.profile_id != request.profile_id
            || receipt.operation_id != request.operation_id
            || receipt.provider != request.provider
            || receipt.legacy_snapshot_id.as_deref() != Some(request.snapshot_id.as_str())
        {
            return Err(ProviderCredentialError::Conflict);
        }
        receipt.legacy_cleanup_allowed = false;
        Ok(receipt.clone())
    }
    fn cancel_owner(&self, owner: &NativeOperationOwner) -> CredentialResult<()> {
        owner.cancel();
        self.cancelled.fetch_add(1, Ordering::SeqCst);
        if let Some(receipt) = self.receipt.lock().unwrap().as_mut()
            && receipt.status == CredentialOperationStatus::Prepared
        {
            receipt.status = CredentialOperationStatus::Cancelled;
        }
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    profile: String,
    service: Arc<FakeService>,
    server: Option<MigrationServer>,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-pcred-{:08x}", rand::random::<u32>()));
        let profile = format!("{:064x}", rand::random::<u128>());
        let service = Arc::new(FakeService::default());
        let expected = profile.clone();
        let validate: LinkValidator = Arc::new(move |reference, url| {
            if reference.profile_id != expected
                || reference.provider != Provider::Linear
                || reference.connection_id != "b".repeat(32)
                || reference.revision != 1
                || url != "https://linear.app/acme/issue/ACME-1"
            {
                return Err(ProviderCredentialError::Conflict);
            }
            Ok(url.into())
        });
        let server = start(profile.clone(), root.clone(), service.clone(), validate).unwrap();
        Self {
            root,
            profile,
            service,
            server: Some(server),
        }
    }
    fn offer(&self) -> Request {
        Request::Offer {
            version: VERSION,
            id: "correlation-1".into(),
            profile_id: self.profile.clone(),
            operation_id: "operation-1".into(),
            provider: Provider::Linear,
            expected_revision: 0,
            configuration: ConnectionConfiguration::default(),
            snapshot_id: "c".repeat(64),
        }
    }
    async fn connection(&self) -> crate::private_ipc::PrivateIpcClientConnection {
        crate::private_ipc::connect_owner_checked(
            &endpoint(&self.profile, &self.root).unwrap(),
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap()
    }
    async fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            tokio::time::timeout(Duration::from_secs(2), server.shutdown())
                .await
                .unwrap();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.server.take());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn secret_input() -> Cursor<Vec<u8>> {
    let token = SecretToken::from_bytes(Provider::Linear, CANARY.to_vec()).unwrap();
    let mut bytes = Vec::new();
    frames::write_secret(&mut bytes, &token).unwrap();
    Cursor::new(bytes)
}
struct MustNotRead;
impl Read for MustNotRead {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("secret input must not be read before a prepared offer")
    }
}
async fn wait_count(count: &AtomicUsize, minimum: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::SeqCst) < minimum {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(3)
}

#[tokio::test]
async fn owner_checked_private_endpoint_imports_once_queries_receipt_and_acknowledges_cleanup() {
    let mut fixture = Fixture::new();
    let request = fixture.offer();
    let mut pipe = fixture.connection().await;
    let mut public_output = Vec::new();
    let response = client::bridge(
        &fixture.profile,
        &request,
        secret_input(),
        &mut public_output,
        &mut pipe,
        deadline(),
    )
    .await
    .unwrap();
    assert!(
        matches!(&response,Response::Committed{receipt,..} if receipt.legacy_cleanup_allowed
        &&receipt.legacy_snapshot_id.as_deref()==Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"))
    );
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 1);
    frames::write_header(&mut public_output, &response).unwrap();
    assert!(
        !public_output
            .windows(CANARY.len())
            .any(|bytes| bytes == CANARY)
    );
    let query = Request::Operation {
        version: 1,
        id: "query".into(),
        profile_id: fixture.profile.clone(),
        operation_id: "operation-1".into(),
        provider: Provider::Linear,
    };
    let mut pipe = fixture.connection().await;
    let receipt = client::bridge(
        &fixture.profile,
        &query,
        MustNotRead,
        &mut Vec::new(),
        &mut pipe,
        deadline(),
    )
    .await
    .unwrap();
    assert!(matches!(receipt, Response::Committed { .. }));
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 1);
    let cleanup = Request::Cleanup {
        version: 1,
        id: "cleanup".into(),
        profile_id: fixture.profile.clone(),
        operation_id: "operation-1".into(),
        provider: Provider::Linear,
        snapshot_id: "c".repeat(64),
    };
    let mut pipe = fixture.connection().await;
    let receipt = client::bridge(
        &fixture.profile,
        &cleanup,
        MustNotRead,
        &mut Vec::new(),
        &mut pipe,
        deadline(),
    )
    .await
    .unwrap();
    assert!(matches!(receipt,Response::Operation{receipt,..} if !receipt.legacy_cleanup_allowed));
    fixture.stop().await;
}
#[tokio::test]
async fn profile_mismatch_is_rejected_before_offer_or_secret_input() {
    let mut fixture = Fixture::new();
    let request = fixture.offer();
    let (mut client, mut server) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        frames::write_header_async(
            &mut server,
            &Hello {
                version: 1,
                profile_id: "d".repeat(64),
            },
        )
        .await
        .unwrap();
        let mut received = Vec::new();
        server.read_to_end(&mut received).await.unwrap();
        received
    });
    let result = client::bridge(
        &fixture.profile,
        &request,
        MustNotRead,
        &mut Vec::new(),
        &mut client,
        deadline(),
    )
    .await;
    assert_eq!(result.err(), Some(ProviderCredentialError::Unauthorized));
    drop(client);
    assert!(task.await.unwrap().is_empty());
    assert_eq!(fixture.service.prepared.load(Ordering::SeqCst), 0);
    fixture.stop().await;
}
#[tokio::test]
async fn server_rejects_wrong_profile_without_calling_service() {
    let mut fixture = Fixture::new();
    let mut pipe = fixture.connection().await;
    let _: Hello = frames::read_header_async(&mut pipe).await.unwrap();
    let mut request = fixture.offer();
    if let Request::Offer { profile_id, .. } = &mut request {
        *profile_id = "d".repeat(64);
    }
    frames::write_header_async(&mut pipe, &request)
        .await
        .unwrap();
    let response: Response = frames::read_header_async(&mut pipe).await.unwrap();
    assert!(matches!(
        response,
        Response::Held {
            reason: ProviderCredentialError::InvalidRequest,
            ..
        }
    ));
    assert_eq!(fixture.service.prepared.load(Ordering::SeqCst), 0);
    fixture.stop().await;
}
#[tokio::test]
async fn oversize_secret_cancels_the_prepared_owner_without_importing() {
    let mut fixture = Fixture::new();
    let mut pipe = fixture.connection().await;
    let _: Hello = frames::read_header_async(&mut pipe).await.unwrap();
    frames::write_header_async(&mut pipe, &fixture.offer())
        .await
        .unwrap();
    assert!(matches!(
        frames::read_header_async::<_, Response>(&mut pipe)
            .await
            .unwrap(),
        Response::Prepared { .. }
    ));
    use tokio::io::AsyncWriteExt;
    pipe.write_u32(frames::MAX_SECRET_BYTES as u32 + 1)
        .await
        .unwrap();
    drop(pipe);
    wait_count(&fixture.service.cancelled, 1).await;
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .service
            .receipt
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .status,
        CredentialOperationStatus::Cancelled
    );
    fixture.stop().await;
}
#[tokio::test]
async fn shutdown_cancels_a_blocked_native_confirmation_and_releases_the_endpoint() {
    let mut fixture = Fixture::new();
    fixture.service.block_prepare.store(true, Ordering::SeqCst);
    let mut pipe = fixture.connection().await;
    let _: Hello = frames::read_header_async(&mut pipe).await.unwrap();
    frames::write_header_async(&mut pipe, &fixture.offer())
        .await
        .unwrap();
    wait_count(&fixture.service.prepared, 1).await;
    fixture.stop().await;
    wait_count(&fixture.service.cancelled, 1).await;
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 0);
    assert!(
        crate::private_ipc::connect_owner_checked(
            &endpoint(&fixture.profile, &fixture.root).unwrap(),
            tokio::time::Instant::now() + Duration::from_millis(50)
        )
        .await
        .is_err()
    );
}
#[tokio::test]
async fn importing_error_recovers_the_same_committed_receipt_without_resending() {
    let mut fixture = Fixture::new();
    fixture
        .service
        .lose_import_ack
        .store(true, Ordering::SeqCst);
    let mut pipe = fixture.connection().await;
    let response = client::bridge(
        &fixture.profile,
        &fixture.offer(),
        secret_input(),
        &mut Vec::new(),
        &mut pipe,
        deadline(),
    )
    .await
    .unwrap();
    assert!(
        matches!(response,Response::Committed{receipt,..} if receipt.status==CredentialOperationStatus::Committed)
    );
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 1);
    fixture.stop().await;
}
#[tokio::test]
async fn eof_after_secret_transfer_returns_unknown_and_never_retries() {
    let mut fixture = Fixture::new();
    let request = fixture.offer();
    let offer = request.offer().unwrap();
    let server_offer = offer.clone();
    let (mut client, mut server) = tokio::io::duplex(8192);
    let task = tokio::spawn(async move {
        frames::write_header_async(
            &mut server,
            &Hello {
                version: 1,
                profile_id: server_offer.profile_id.clone(),
            },
        )
        .await
        .unwrap();
        let request: Request = frames::read_header_async(&mut server).await.unwrap();
        frames::write_header_async(
            &mut server,
            &Response::Prepared {
                version: 1,
                id: request.id().into(),
                profile_id: server_offer.profile_id,
                operation_id: server_offer.operation_id,
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
        let token = frames::read_secret_async(&mut server, Provider::Linear)
            .await
            .unwrap();
        assert_eq!(token.expose(), CANARY);
        // The synthetic service has written; the transport closes before any terminal acknowledgement.
    });
    let mut headers = Vec::new();
    let response = client::bridge(
        &fixture.profile,
        &request,
        secret_input(),
        &mut headers,
        &mut client,
        deadline(),
    )
    .await
    .unwrap();
    assert!(
        matches!(response,Response::Unknown{operation_id,..} if operation_id==offer.operation_id)
    );
    task.await.unwrap();
    assert!(!headers.windows(CANARY.len()).any(|bytes| bytes == CANARY));
    fixture.stop().await;
}
#[tokio::test]
async fn external_link_uses_the_current_native_reference_and_never_reads_a_secret() {
    let mut fixture = Fixture::new();
    let request = Request::ValidateLink {
        version: 1,
        id: "link".into(),
        reference: ConnectionRef {
            profile_id: fixture.profile.clone(),
            provider: Provider::Linear,
            connection_id: "b".repeat(32),
            revision: 1,
        },
        url: "https://linear.app/acme/issue/ACME-1".into(),
    };
    let mut pipe = fixture.connection().await;
    let response = client::bridge(
        &fixture.profile,
        &request,
        MustNotRead,
        &mut Vec::new(),
        &mut pipe,
        deadline(),
    )
    .await
    .unwrap();
    assert!(
        matches!(response,Response::ValidatedLink{url,..} if url=="https://linear.app/acme/issue/ACME-1")
    );
    let mut stale = request;
    if let Request::ValidateLink { reference, .. } = &mut stale {
        reference.revision = 0;
    }
    let mut pipe = fixture.connection().await;
    assert!(matches!(
        client::bridge(
            &fixture.profile,
            &stale,
            MustNotRead,
            &mut Vec::new(),
            &mut pipe,
            deadline()
        )
        .await
        .unwrap(),
        Response::Held {
            reason: ProviderCredentialError::Conflict,
            ..
        }
    ));
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 0);
    fixture.stop().await;
}
#[test]
fn public_headers_reject_token_fields_and_bound_unknown_commands() {
    let raw=b"{\"version\":1,\"id\":\"test\",\"command\":\"operation\",\"profileId\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"operationId\":\"operation-1\",\"provider\":\"linear\",\"token\":\"synthetic-migration-canary-token\"}\n";
    let result = frames::read_header::<_, Request>(&mut Cursor::new(raw));
    assert_eq!(result.err(), Some(ProviderCredentialError::InvalidRequest));
    let mut bytes = vec![b'x'; frames::MAX_HEADER_BYTES];
    bytes.push(b'\n');
    assert_eq!(
        frames::read_header::<_, Request>(&mut Cursor::new(bytes)).err(),
        Some(ProviderCredentialError::TooLarge)
    );
}
#[test]
fn prepared_and_receipt_correlation_cannot_be_substituted() {
    let request = Request::Offer {
        version: 1,
        id: "id".into(),
        profile_id: "a".repeat(64),
        operation_id: "operation".into(),
        provider: Provider::Linear,
        expected_revision: 0,
        configuration: ConnectionConfiguration::default(),
        snapshot_id: "c".repeat(64),
    };
    let mismatch = Response::Prepared {
        version: 1,
        id: "id".into(),
        profile_id: "a".repeat(64),
        operation_id: "other".into(),
        expected_revision: 0,
    };
    assert_eq!(
        mismatch.validate(&request),
        Err(ProviderCredentialError::InvalidRequest)
    );
    let mut receipt = FakeService::receipt(
        &request.offer().unwrap(),
        CredentialOperationStatus::Committed,
    );
    receipt.legacy_snapshot_id = Some("d".repeat(64));
    assert_eq!(
        Response::from_receipt("id", receipt).validate(&request),
        Err(ProviderCredentialError::InvalidRequest)
    );
}

#[tokio::test]
async fn a_pending_fallback_receipt_is_cancelled_when_its_private_session_ends() {
    let mut fixture = Fixture::new();
    fixture
        .service
        .fail_prepare_with_pending
        .store(true, Ordering::SeqCst);
    let mut pipe = fixture.connection().await;
    let response = client::bridge(
        &fixture.profile,
        &fixture.offer(),
        MustNotRead,
        &mut Vec::new(),
        &mut pipe,
        deadline(),
    )
    .await
    .unwrap();
    assert!(matches!(response, Response::Operation { receipt, .. }
        if receipt.status == CredentialOperationStatus::Prepared));
    wait_count(&fixture.service.cancelled, 1).await;
    assert_eq!(
        fixture
            .service
            .receipt
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .status,
        CredentialOperationStatus::Cancelled,
    );
    assert_eq!(fixture.service.imported.load(Ordering::SeqCst), 0);
    fixture.stop().await;
}
