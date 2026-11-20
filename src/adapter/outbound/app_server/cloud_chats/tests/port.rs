use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::*;

struct FixtureDir(PathBuf);

impl Drop for FixtureDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("fixture cleanup failed: {error}");
        }
    }
}

#[test]
fn a_limited_detail_read_makes_the_list_stale_for_the_same_account() {
    let root = std::env::temp_dir().join(format!(
        "akra-cloud-chats-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    fs::create_dir(&root).expect("fixture directory");
    let fixture = FixtureDir(root);
    let auth_path = fixture.0.join("auth.json");
    let curl = fixture.0.join("curl");
    fs::write(
        &curl,
        r#"#!/bin/sh
case "$(cat)" in
    *'/conversation/c-1'*) printf 'HTTP/2 429\r\nRetry-After: 120\r\n\r\n{}\n429' ;;
    *) printf 'HTTP/2 200\r\n\r\n{"items":[{"id":"c-1"}]}\n200' ;;
esac
"#,
    )
    .expect("write curl fixture");
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).expect("make fixture executable");
    fs::write(
        &auth_path,
        r#"{"tokens":{"access_token":"fixture","account_id":"account-a"}}"#,
    )
    .expect("write credentials");
    let adapter = CloudChatsRestAdapter {
        auth_path,
        curl: Ok(curl),
        cache: Mutex::default(),
    };
    let previous = adapter.list_chats().expect("prime last-good list");
    assert!(matches!(
        adapter.read_chat("c-1"),
        Err(ChatsError::RateLimited {
            retry_after_seconds: Some(120)
        })
    ));

    let limited = adapter.list_chats().expect("serve last-good list");

    assert_eq!(limited.value, previous.value);
    assert!(
        limited.limited.is_some(),
        "a second upstream list would be fresh"
    );
}
