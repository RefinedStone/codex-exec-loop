//! A real WebSocket + recorder + SQLite ledger, with synthetic prompts, key store and provider replies.
use super::*;
use crate::adapter::outbound::db::SqliteProviderCredentialLedgerAdapter;
use crate::application::port::outbound::provider_credentials_port::*;
use crate::application::port::outbound::provider_work_port::ProviderWorkApiPort;
use crate::application::service::provider_credentials::ProviderCredentialService;
use crate::application::service::provider_work::ProviderWorkService;
use crate::composition::provider_credentials::{ProviderProfile, ProviderServices};
use crate::domain::provider_credentials::*;
use crate::domain::provider_work::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use zeroize::Zeroizing;

const SECRET: &str = "gateway-provider-secret-canary-2562";
const COMPANY_TEXT: &str = "gateway-provider-company-text-canary-2562";
#[derive(Default)]
struct KeyStore(Mutex<HashMap<String, Vec<u8>>>);
fn key(part: &KeyringPart) -> String {
    format!(
        "{}:{}:{}:{}",
        part.profile_id,
        part.provider.as_str(),
        part.connection_id,
        part.index
    )
}
impl ProviderKeyStorePort for KeyStore {
    fn read_part(&self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&key(part))
            .cloned()
            .map(Zeroizing::new))
    }
    fn write_part(&self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        self.0.lock().unwrap().insert(key(part), capsule.to_vec());
        Ok(())
    }
    fn delete_part(&self, part: &KeyringPart) -> CredentialResult<()> {
        self.0.lock().unwrap().remove(&key(part));
        Ok(())
    }
}
struct Prompt;
impl NativePromptPort for Prompt {
    fn prompt(
        &self,
        request: &ConnectRequest,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<SecretToken> {
        SecretToken::from_bytes(request.provider, SECRET.as_bytes().to_vec())
    }
    fn confirm_migration(
        &self,
        _request: &LegacyImportOffer,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<bool> {
        Ok(true)
    }
}
struct Api;
impl ProviderAccountVerifierPort for Api {
    fn verify(
        &self,
        request: &ConnectRequest,
        token: &SecretToken,
        _cancelled: &AtomicBool,
        _deadline: Instant,
    ) -> CredentialResult<ConnectionMetadata> {
        assert_eq!(token.as_str(), SECRET);
        Ok(ConnectionMetadata {
            configuration: request.configuration.clone(),
            account_id: "person".to_owned(),
            account_name: "Person".to_owned(),
        })
    }
}
impl ProviderWorkApiPort for Api {
    fn jira_summary(
        &self,
        metadata: &ConnectionMetadata,
        token: &SecretToken,
    ) -> CredentialResult<JiraSummary> {
        assert_eq!(token.as_str(), SECRET);
        let site = metadata.configuration.site_url.as_ref().unwrap();
        Ok(JiraSummary {
            site_url: site.clone(),
            issues: vec![JiraIssue {
                key: "APP-42".to_owned(),
                summary: COMPANY_TEXT.to_owned(),
                url: format!("{site}/browse/APP-42"),
                status: None,
                status_category: None,
                issue_type: None,
                priority: None,
                updated_at: "2026-11-23T01:02:03Z".to_owned(),
            }],
        })
    }
    fn jira_issue_prompt(
        &self,
        _metadata: &ConnectionMetadata,
        _token: &SecretToken,
        _key: &str,
    ) -> CredentialResult<String> {
        Ok(COMPANY_TEXT.to_owned())
    }
    fn jira_current_version(
        &self,
        _metadata: &ConnectionMetadata,
        _token: &SecretToken,
        _key: &str,
    ) -> CredentialResult<String> {
        Ok("2026-11-23T01:02:03.000Z".to_owned())
    }
    fn linear_assigned_issues(&self, _token: &SecretToken) -> CredentialResult<Vec<LinearIssue>> {
        Ok(vec![])
    }
    fn linear_issue_prompt(&self, _token: &SecretToken, _id: &str) -> CredentialResult<String> {
        Ok(COMPANY_TEXT.to_owned())
    }
}
fn services(folder: &KernelTestFolder) -> (ProviderServices, Arc<KeyStore>) {
    let profile = ProviderProfile {
        id: "a".repeat(64),
        runtime: folder.path.join("runtime"),
    };
    let owner = crate::composition::provider_credentials::acquire_owner(&profile).unwrap();
    let runtime = profile.runtime.clone();
    let ledger = Arc::new(
        SqliteProviderCredentialLedgerAdapter::new(profile.id.clone(), move || Ok(runtime.clone()))
            .unwrap(),
    );
    let keys = Arc::new(KeyStore::default());
    let api = Arc::new(Api);
    let credentials = Arc::new(
        ProviderCredentialService::new(
            profile.id.clone(),
            ledger,
            keys.clone(),
            Arc::new(Prompt),
            api.clone(),
        )
        .unwrap(),
    );
    let work = Arc::new(ProviderWorkService::new(credentials.clone(), api));
    (
        ProviderServices {
            profile,
            owner,
            credentials,
            work,
        },
        keys,
    )
}
async fn client(
    services: ProviderServices,
    recorder: Option<Arc<super::super::recorder::GatewayRecorder>>,
    remote: bool,
) -> (
    WebSocketStream<DuplexStream>,
    tokio::task::JoinHandle<Result<()>>,
) {
    let (client, gateway) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
    let session = tokio::spawn(run_session(
        gateway,
        GatewayHosts::new(
            EchoUpstream::new(true),
            vec![("cloud".to_owned(), EchoUpstream::new(true))],
        ),
        Arc::new(crate::composition::repository_kernel::RepositoryKernelRegistry::default()),
        super::super::session::GatewayServices {
            providers: Some(services),
            ..Default::default()
        },
        recorder,
    ));
    let url = if remote {
        "ws://localhost/host/cloud"
    } else {
        "ws://localhost/"
    };
    (
        tokio_tungstenite::client_async_with_config(url, client, Some(websocket_config()))
            .await
            .unwrap()
            .0,
        session,
    )
}
#[tokio::test(flavor = "multi_thread")]
async fn provider_credentials_and_work_content_never_reach_recording_or_upstream() {
    let folder = KernelTestFolder::new("provider-credential-privacy");
    let recording = folder.path.join("recordings");
    let recorder = Arc::new(super::super::recorder::GatewayRecorder::create(&recording).unwrap());
    let (services, keys) = services(&folder);
    let credentials = services.credentials.clone();
    let (mut client, session) = client(services, Some(recorder), false).await;
    let state = kernel_request(
        &mut client,
        "state",
        "akra/providerCredentials/state",
        json!({"provider":"jira"}),
    )
    .await;
    assert_eq!(state["result"]["revision"], 0);
    let connect = json!({"operationId":"connect-first","profileId":"a".repeat(64),"provider":"jira","expectedRevision":0,
        "configuration":{"siteUrl":"https://jira.example","deployment":"cloud","email":"person@example.invalid"}});
    let connected = kernel_request(
        &mut client,
        "connect",
        "akra/providerCredentials/connect",
        connect.clone(),
    )
    .await;
    assert_eq!(connected["result"]["status"], "committed", "{connected}");
    assert!(!connected.to_string().contains(SECRET));
    let reference = connected["result"]["connection"].clone();
    let summary = kernel_request(
        &mut client,
        "summary",
        "akra/providers/jira/summary",
        json!({"connection":reference}),
    )
    .await;
    assert_eq!(summary["result"]["issues"][0]["summary"], COMPANY_TEXT);
    for method in [
        "akra/providerCredentials/connect",
        "akra/providerCredentials/importSecret",
        "akra/providers/jira/summary",
    ] {
        let mut bad = connect.clone();
        bad["token"] = json!(SECRET);
        let response = kernel_request(&mut client, "rejected", method, bad).await;
        assert!(response["error"].is_object());
        assert!(!response.to_string().contains(SECRET));
    }
    let escaped = format!(
        r#"{{"jsonrpc":"2.0","id":"escaped","method":"\u0061kra/providerCredentials/connect","params":{{"token":"{SECRET}"}}}}"#
    );
    client.send(Message::text(escaped)).await.unwrap();
    let rejected = next_json_matching(&mut client, |message| message["id"] == "escaped").await;
    assert_eq!(rejected["error"]["code"], -32602);
    client
        .send(Message::text(format!(
            r#"{{"method":"akra/providerCredentials/connect","token":"{SECRET}""#
        )))
        .await
        .unwrap();
    let rejected = next_json_matching(&mut client, |message| {
        message["error"]["code"] == -32602 && message["id"].is_null()
    })
    .await;
    assert!(!rejected.to_string().contains(SECRET));

    for escaped in [
        format!(
            r#"{{"jsonrpc":"2.0","id":"escaped","method":"akra\/providerCredentials\/connect","params":{{"token":"{SECRET}"}}}}"#
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":"escaped","method":"akra\u002fproviderCredentials\u002fconnect","params":{{"token":"{SECRET}"}}}}"#
        ),
    ] {
        client.send(Message::text(escaped)).await.unwrap();
        let rejected = next_json_matching(&mut client, |message| message["id"] == "escaped").await;
        assert_eq!(rejected["error"]["code"], -32602);
    }
    for invalid in [
        format!(
            r#"{{"params":{{"token":"{SECRET}",}},"method":"akra\/providers\/jira\/summary"}}"#
        ),
        format!(
            r#"{{"params":{{"token":"{SECRET}",}},"method":"akra/\u0070roviderCredentials/connect"}}"#
        ),
        format!(
            r#"{{"id":1,"method":"akra/providerCredentials/connect","method":"thread/list","params":{{"token":"{SECRET}"}}}}"#
        ),
        format!(
            r#"{{"id":1,"id":2,"method":"akra/providerCredentials/connect","params":{{"token":"{SECRET}"}}}}"#
        ),
        format!(
            r#"[{{"id":1,"method":"akra/providerCredentials/connect","params":{{"token":"{SECRET}"}}}}]"#
        ),
    ] {
        client.send(Message::text(invalid)).await.unwrap();
        let rejected = next_json_matching(&mut client, |message| {
            message["error"]["code"] == -32602 && message["id"].is_null()
        })
        .await;
        assert!(!rejected.to_string().contains(SECRET));
    }
    let disconnected=kernel_request(&mut client,"disconnect","akra/providerCredentials/disconnect",
        json!({"operationId":"disconnect-first","profileId":"a".repeat(64),"provider":"jira","expectedRevision":1})).await;
    assert_eq!(
        disconnected["result"]["status"], "committed",
        "{disconnected}"
    );
    assert!(keys.0.lock().unwrap().is_empty());
    assert!(
        credentials
            .state(Provider::Jira)
            .unwrap()
            .active_connection
            .is_none()
    );
    kernel_request(
        &mut client,
        "recording-marker",
        "akra/gateway/info",
        json!({}),
    )
    .await;
    client.close(None).await.unwrap();
    drop(client);
    tokio::time::timeout(TEST_TIMEOUT, session)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let recorded: String = std::fs::read_dir(&recording)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    assert!(recorded.contains("akra/gateway/info"));
    assert!(!recorded.contains(SECRET));
    assert!(!recorded.contains(COMPANY_TEXT));
    assert!(!recorded.contains("akra/providerCredentials/"));
    assert!(!recorded.contains("akra/providers/"));
    // The durable ledger stores public connection/receipt state only.
    for entry in std::fs::read_dir(folder.path.join("runtime")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(path).unwrap();
            assert!(
                !bytes
                    .windows(SECRET.len())
                    .any(|part| part == SECRET.as_bytes())
            );
        }
    }
}
#[tokio::test]
async fn provider_credentials_are_local_only_and_never_forwarded_to_remote_hosts() {
    let folder = KernelTestFolder::new("provider-credential-remote");
    let (services, keys) = services(&folder);
    let credentials = services.credentials.clone();
    let (mut client, session) = client(services, None, true).await;
    for method in [
        "akra/providerCredentials/state",
        "akra/providerCredentials/connect",
        "akra/providers/jira/summary",
    ] {
        let response = kernel_request(
            &mut client,
            "remote",
            method,
            json!({"provider":"jira","token":SECRET}),
        )
        .await;
        assert_eq!(response["error"]["code"], -32601);
        assert!(!response.to_string().contains(SECRET));
    }
    client
        .send(Message::text("normal-after-rejection"))
        .await
        .unwrap();
    assert_eq!(next_text(&mut client).await, "echo:normal-after-rejection");
    assert!(keys.0.lock().unwrap().is_empty());
    assert_eq!(credentials.state(Provider::Jira).unwrap().revision, 0);
    client.close(None).await.unwrap();
    drop(client);
    tokio::time::timeout(TEST_TIMEOUT, session)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
