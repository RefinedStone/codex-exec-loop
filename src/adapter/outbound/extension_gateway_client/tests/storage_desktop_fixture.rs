//! The production CLI adapter on an explicitly supplied test socket, never the user's Gateway.
use super::super::*;

#[test]
#[ignore = "manual CLI adapter for desktop/test/e2e/storage-service-roundtrip.mjs"]
fn storage_desktop_fixture_cli() -> Result<()> {
    let endpoint = std::env::var("AKRA_STORAGE_FIXTURE_ENDPOINT")?;
    ensure!(
        endpoint.contains("akra-fake-"),
        "only the named fixture endpoint is allowed"
    );
    let request: Value = serde_json::from_str(&std::env::var("AKRA_STORAGE_FIXTURE_CALL")?)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let answer = runtime.block_on(async {
        let connected = crate::adapter::outbound::gateway_kernel_client::connect(&endpoint).await?;
        let (mut socket, _) = tokio_tungstenite::client_async("ws://localhost/rpc", connected).await?;
        verify_thread(&mut socket, request["thread_id"].as_str().context("missing fixture thread")?).await?;
        let answer = request_over_socket(&mut socket, "local_user", "fixture", Some(json!({
            "extension":"akra.storage-analysis","version":1,"action":request["action"],"input":request["input"]
        }))).await?;
        let _ = socket.close(None).await;
        Ok::<_, anyhow::Error>(answer)
    })?;
    println!("AKRA_STORAGE_CLI_RESULT {answer}");
    Ok(())
}
