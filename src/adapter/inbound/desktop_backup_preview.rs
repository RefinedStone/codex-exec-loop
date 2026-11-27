//! Private, read-only inherited-pipe helper. It does not open a Vault, key store, or Gateway session.
use crate::application::port::outbound::evidence_vault_backup_port::VaultRecoveryKey;
use serde::Serialize;
use std::io::{self, Read, Write};
use zeroize::Zeroizing;

const LIMIT: usize = 8 * 1024 * 1024 + 129;
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Reply {
    Verified { version: u32, projects: Vec<String> },
    Rejected,
}

fn inspect(wire: &[u8]) -> Option<Reply> {
    let separator = wire.iter().take(129).position(|byte| *byte == b'\n')?;
    let key = VaultRecoveryKey::parse(std::str::from_utf8(&wire[..separator]).ok()?)?;
    let archive = std::str::from_utf8(&wire[separator + 1..]).ok()?;
    let (version, projects) =
        crate::composition::desktop_restore::inspect_archive(archive, &key).ok()?;
    if projects.iter().any(|p| {
        p.workspace_dir.is_empty()
            || p.workspace_dir.len() > 4096
            || p.workspace_dir.chars().any(char::is_control)
    }) {
        return None;
    }
    Some(Reply::Verified {
        version,
        projects: projects.into_iter().map(|p| p.workspace_dir).collect(),
    })
}
pub(super) fn run() -> anyhow::Result<()> {
    let mut input = Zeroizing::new(Vec::new());
    io::stdin()
        .lock()
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| anyhow::anyhow!("archive preview input unavailable"))?;
    let reply = (input.len() <= LIMIT)
        .then(|| inspect(&input))
        .flatten()
        .unwrap_or(Reply::Rejected);
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &reply)
        .map_err(|_| anyhow::anyhow!("archive preview output unavailable"))?;
    output.flush()?;
    Ok(())
}
