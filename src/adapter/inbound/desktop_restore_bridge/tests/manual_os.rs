//! Explicitly opt-in real desktop/credential probe. CI must never interact
//! with a real user's OS prompt, keyring or live GitHub issues.
use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::vault_key_store::ManualCredentialFixture;
use std::path::PathBuf;

struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-vault-manual-fixture-{:032x}",
            rand::random::<u128>()
        ));
        crate::private_directory::ensure_private_directory(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct RealDialog {
    #[cfg(windows)]
    executable: PathBuf,
}
impl RealDialog {
    fn from_environment() -> Self {
        Self {
            #[cfg(windows)]
            executable: PathBuf::from(
                std::env::var_os("AKRA_MANUAL_GATEWAY_EXE")
                    .expect("absolute akra-gateway.exe built from this revision"),
            ),
        }
    }
}
impl VaultNativeConfirmationPort for RealDialog {
    fn confirm(
        &self,
        request: &VaultNativeConfirmationRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> VaultNativeConfirmationOutcome {
        #[cfg(windows)]
        {
            crate::adapter::outbound::native_vault_confirmation::windows::confirm_for_manual_fixture(
                &self.executable,
                request,
                cancelled,
                deadline,
            )
        }
        #[cfg(not(windows))]
        {
            crate::adapter::outbound::native_vault_confirmation::NativeVaultConfirmationAdapter
                .confirm(request, cancelled, deadline)
        }
    }
}

async fn manual_response(client: &mut BufReader<DuplexStream>) -> Value {
    let mut line = String::new();
    let count = tokio::time::timeout(Duration::from_secs(120), client.read_line(&mut line))
        .await
        .expect("manual dialog timed out")
        .expect("private pipe");
    assert!(
        count > 0,
        "manual private session ended before its response"
    );
    serde_json::from_str(&line).expect("private response")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit operator in a disposable interactive OS profile"]
async fn real_os_confirmation_and_credential_restore_roundtrip() {
    assert_eq!(
        std::env::var("AKRA_MANUAL_VAULT_PROFILE").as_deref(),
        Ok("isolated-disposable"),
        "run only in a disposable interactive OS profile; see docs/validation/vault-real-os-manual.md"
    );
    let folder = Folder::new();
    let workspace = folder.0.join("workspace");
    let vault_root = folder.0.join("vault");
    let runtime = folder.0.join("key-runtime");
    for path in [&workspace, &vault_root, &runtime] {
        crate::private_directory::ensure_private_directory(path).unwrap();
    }
    let reference = IssueReference {
        project_scope: IssueProjectScope {
            workspace_dir: std::fs::canonicalize(&workspace)
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
        },
        ..issue()
    };
    let dialog = Arc::new(RealDialog::from_environment());
    let credentials =
        ManualCredentialFixture::new(runtime).expect("disposable OS credential namespace");
    let key_store = credentials.adapter();
    assert!(
        key_store
            .load_key()
            .expect("query fixture namespace")
            .is_none()
    );
    let initial = EvidenceVaultUnlockService::new(Arc::new(key_store))
        .sealing_key()
        .expect("create fixture key");
    let reopened = credentials.adapter();
    assert_eq!(
        reopened.load_key().expect("reopen fixture namespace"),
        Some(initial.key.clone())
    );

    let store = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(vault_root));
    let fields = seed_with(store.as_ref(), reference.clone(), initial);
    let remote = Arc::new(Remote {
        fields: Mutex::new(fields.clone()),
        reference: Mutex::new(reference.clone()),
        reads: AtomicUsize::new(0),
        resolves: AtomicUsize::new(0),
        gate: Mutex::new(None),
    });
    let services = || {
        let unlock = Arc::new(EvidenceVaultUnlockService::new(Arc::new(
            credentials.adapter(),
        )));
        DesktopVaultRestoreDependencies {
            supported: true,
            snapshots: remote.clone(),
            unlock: unlock.clone(),
            restore: Arc::new(EvidenceVaultRestoreService::new(
                remote.clone(),
                store.clone(),
                unlock,
            )),
            confirmation: dialog.clone(),
            workspace: Arc::new(Workspace(Some(
                reference.project_scope.workspace_dir.clone(),
            ))),
        }
    };
    let mut prepare = prepare_request();
    prepare["issue"]["project_scope"]["workspace_dir"] =
        json!(reference.project_scope.workspace_dir);

    eprintln!("Step 1/4: cancel the native viewing confirmation");
    let (mut client, task) = start(services(), SESSION_LIFETIME);
    send(&mut client, &prepare).await;
    let cancelled = manual_response(&mut client).await;
    assert_eq!(cancelled["result"]["kind"], "held");
    assert_eq!(cancelled["result"]["reason"], "confirmation_cancelled");
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();

    eprintln!("Step 2/4: approve viewing the synthetic issue");
    let (mut client, task) = start(services(), SESSION_LIFETIME);
    send(&mut client, &prepare).await;
    assert_eq!(
        manual_response(&mut client).await["result"]["kind"],
        "prepared"
    );
    send(&mut client, &restore_request()).await;
    let restored = manual_response(&mut client).await;
    assert_eq!(restored["result"]["title"], TITLE);
    assert_eq!(restored["result"]["body"], BODY);
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();

    *remote.fields.lock().unwrap() = (
        format!("changed {}", fields.0),
        format!("changed {}", fields.1),
    );
    eprintln!("Step 3/4: approve viewing; remote synthetic copy now differs");
    let (mut client, task) = start(services(), SESSION_LIFETIME);
    send(&mut client, &prepare).await;
    assert_eq!(
        manual_response(&mut client).await["result"]["kind"],
        "prepared"
    );
    send(&mut client, &restore_request()).await;
    let held = manual_response(&mut client).await;
    assert_eq!(held["result"]["kind"], "held");
    assert_eq!(held["result"]["can_reconnect"], true);
    send(
        &mut client,
        &json!({"version":2,"id":"test-session","command":"review"}),
    )
    .await;
    let review = manual_response(&mut client).await;
    assert_eq!(review["result"]["kind"], "review");
    let result = &review["result"]["review"];
    let mappings:Vec<_>=result["markers"].as_array().expect("review markers").iter().map(|marker| {
        let selected=&marker["candidates"][0];
        json!({"marker":marker["marker"],"field":selected["field"],"position":selected["position"]})
    }).collect();
    assert_eq!(mappings.len(), 2);
    assert!(!review.to_string().contains("synthetic-body-secret"));

    eprintln!("Step 4/4: approve the separate native binding-save confirmation");
    send(&mut client,&json!({"version":2,"id":"test-session","command":"reconnect","reviewed_hash":result["reviewed_hash"],"mappings":mappings})).await;
    assert_eq!(
        manual_response(&mut client).await["result"]["kind"],
        "reconnected"
    );
    send(&mut client, &restore_request()).await;
    let restored = manual_response(&mut client).await;
    assert_eq!(restored["result"]["title"], format!("changed {TITLE}"));
    assert_eq!(restored["result"]["body"], format!("changed {BODY}"));
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();

    credentials
        .delete()
        .expect("delete only the generated test namespace");
    assert!(
        credentials
            .adapter()
            .load_key()
            .expect("verify fixture cleanup")
            .is_none()
    );
    eprintln!("manual fixture completed; no real issue or model was used");
}
