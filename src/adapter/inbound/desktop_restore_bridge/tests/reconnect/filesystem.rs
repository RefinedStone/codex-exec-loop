use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn ciphertexts(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(ciphertexts(&path));
        } else {
            files.push((path.clone(), std::fs::read(&path).unwrap()));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[tokio::test]
async fn actual_encrypted_binding_is_unchanged_on_cancel_and_reconnects_through_the_private_pipe() {
    let fixture = changed_fixture();
    let temporary = Temporary(crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-native-reconnect-{}-{}",
        std::process::id(),
        rand::random::<u64>(),
    )));
    crate::private_directory::ensure_private_directory(&temporary.0).unwrap();
    let store = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(
        temporary.0.clone(),
    ));
    seed(store.as_ref());
    let before = ciphertexts(&temporary.0);
    *fixture.confirmation.write_outcome.lock().unwrap() =
        Some(VaultNativeConfirmationOutcome::Cancelled);
    let (mut client, task, review) =
        open_review(fixture.services_with_store(store.clone()), SESSION_LIFETIME).await;
    send(&mut client, &reconnect_request(&review)).await;
    expect_terminal_hold(&mut client, task, "confirmation_cancelled").await;
    assert_eq!(ciphertexts(&temporary.0), before);

    *fixture.confirmation.write_outcome.lock().unwrap() =
        Some(VaultNativeConfirmationOutcome::Confirmed);
    let (mut client, task, review) =
        open_review(fixture.services_with_store(store.clone()), SESSION_LIFETIME).await;
    send(&mut client, &reconnect_request(&review)).await;
    assert_eq!(response(&mut client).await["result"]["kind"], "reconnected");
    send(&mut client, &restore_request()).await;
    let restored = response(&mut client).await;
    assert_eq!(restored["result"]["title"], format!("제목 수정 {TITLE}"));
    assert_eq!(restored["result"]["body"], format!("본문 수정 {BODY}"));
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    let committed = ciphertexts(&temporary.0);
    assert_ne!(committed, before);
    for (_, bytes) in &committed {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("synthetic-title-secret"));
        assert!(!text.contains("synthetic-body-secret"));
    }

    // 별도 서비스/pipe를 열어도 이미 확정된 연결을 읽으며 추가 파일 쓰기는 없다.
    let (mut client, task) = start(fixture.services_with_store(store), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    assert_eq!(response(&mut client).await["result"]["kind"], "prepared");
    send(&mut client, &restore_request()).await;
    assert_eq!(
        response(&mut client).await["result"]["body"],
        format!("본문 수정 {BODY}")
    );
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    assert_eq!(ciphertexts(&temporary.0), committed);
    assert_eq!(fixture.keys.stores.load(Ordering::SeqCst), 0);
}
