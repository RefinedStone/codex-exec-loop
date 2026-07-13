use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct AdminGraphicsManifest {
    schema_version: u32,
    route_prefix: String,
    policy: String,
    runtime_assets: Vec<String>,
}

#[derive(Deserialize)]
struct AdminFontManifest {
    schema_version: u32,
    source: String,
    source_version: String,
    license_file: String,
    files: Vec<AdminFontFile>,
}

#[derive(Deserialize)]
struct AdminFontFile {
    name: String,
    sha256: String,
    size: u64,
}

#[test]
fn admin_graphics_manifest_matches_tracked_runtime_assets_and_consumers() {
    let repo = repo_root();
    let graphics_root = repo.join("assets/admin/graphics");
    let manifest: AdminGraphicsManifest =
        serde_json::from_str(&read(&graphics_root.join("manifest.json")))
            .expect("admin graphics manifest must be valid JSON");

    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.route_prefix, "/admin/assets/graphics/");
    assert!(
        manifest
            .policy
            .contains("consumed by the dashboard template")
    );

    let declared = manifest
        .runtime_assets
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared.len(),
        manifest.runtime_assets.len(),
        "admin graphics manifest must not contain duplicate names"
    );
    assert_eq!(
        manifest.runtime_assets,
        declared.iter().cloned().collect::<Vec<_>>(),
        "admin graphics manifest must stay sorted for reviewable diffs"
    );

    let present = fs::read_dir(&graphics_root)
        .expect("admin graphics directory must be readable")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|extension| extension.to_str()) == Some("png"))
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared, present,
        "every admin PNG must be declared runtime input; source/reference art belongs elsewhere"
    );

    let static_assets = read(&repo.join("src/adapter/inbound/admin_api/static_assets.rs"));
    for asset in &manifest.runtime_assets {
        assert!(
            asset.ends_with(".png") && !asset.contains('/') && !asset.contains('\\'),
            "manifest contains an unsafe asset name: {asset}"
        );
        assert!(
            static_assets.matches(asset).count() >= 2,
            "runtime asset must be both embedded and routed by the admin API: {asset}"
        );
    }
    let routed = admin_graphic_names(&static_assets, &manifest.route_prefix);
    assert_eq!(
        declared, routed,
        "the admin graphic route must expose exactly the manifest assets"
    );

    let dashboard_template = read(&repo.join("templates/admin/akra_dashboard.html"));
    let diorama = read(&repo.join("assets/admin/game/src/akra-diorama.ts"));
    let consumed = admin_graphic_names(&dashboard_template, &manifest.route_prefix)
        .into_iter()
        .chain(admin_graphic_names(&diorama, &manifest.route_prefix))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared, consumed,
        "every declared admin graphic must be consumed by the dashboard template or diorama TypeScript"
    );
}

#[test]
fn admin_font_manifest_pins_bytes_license_and_runtime_routes() {
    let repo = repo_root();
    let font_root = repo.join("assets/admin/fonts");
    let manifest: AdminFontManifest = serde_json::from_str(&read(&font_root.join("manifest.json")))
        .expect("admin font manifest must be valid JSON");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.source, "https://github.com/quiple/galmuri");
    assert_eq!(manifest.source_version, "v2.40.3");
    assert_eq!(manifest.license_file, "LICENSE-Galmuri.txt");
    assert!(
        read(&font_root.join(&manifest.license_file)).contains("SIL OPEN FONT LICENSE Version 1.1")
    );

    let names = manifest
        .files
        .iter()
        .map(|file| file.name.clone())
        .collect::<Vec<_>>();
    let mut sorted_names = names.clone();
    sorted_names.sort();
    assert_eq!(names, sorted_names, "font manifest must stay sorted");
    assert_eq!(
        names,
        vec![
            "Galmuri11-Bold.woff2".to_string(),
            "Galmuri11.woff2".to_string(),
        ]
    );

    let static_assets = read(&repo.join("src/adapter/inbound/admin_api/static_assets.rs"));
    let base_template = read(&repo.join("templates/admin/base.html"));
    for file in &manifest.files {
        let bytes = fs::read(font_root.join(&file.name)).expect("font bytes must be readable");
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(bytes.len() as u64, file.size, "{} size drifted", file.name);
        assert_eq!(digest, file.sha256, "{} digest drifted", file.name);
        assert_eq!(
            static_assets.matches(&file.name).count(),
            2,
            "{} must be embedded and allowlisted exactly once",
            file.name
        );
        assert!(
            base_template.contains(&format!("/admin/assets/fonts/{}", file.name)),
            "{} must be loaded by the admin template",
            file.name
        );
    }
}

#[test]
fn historical_terminal_artifacts_are_manifested_plain_text_and_sanitized() {
    let root = repo_root().join("artifacts/terminal-bridge-readiness-2026-04-23");
    let readme = read(&root.join("README.md"));
    assert!(readme.contains("Historical"));
    assert!(readme.contains("not evidence for any required row"));
    assert!(readme.contains("b14949a2bdf265c552654adb3435c7c08053f0ac"));

    for entry in fs::read_dir(&root).expect("historical artifact directory must be readable") {
        let path = entry.expect("artifact entry must be readable").path();
        if path.file_name().and_then(|name| name.to_str()) == Some("README.md") {
            continue;
        }
        assert!(
            path.is_file(),
            "artifact manifest directory must contain files only"
        );
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("artifact file name must be UTF-8");
        assert!(
            readme.contains(&format!("`{file_name}`")),
            "historical capture is missing from its manifest: {file_name}"
        );

        let bytes = fs::read(&path).expect("historical capture must be readable");
        assert!(
            !bytes.contains(&0x1b),
            "historical capture must render terminal escapes as text: {file_name}"
        );
        let text = String::from_utf8(bytes).expect("historical capture must be UTF-8 text");
        for local_identifier in [
            "/dev/pts/",
            "akra-evidence",
            "akra-readiness-20260423",
            "pid=162",
        ] {
            assert!(
                !text.contains(local_identifier),
                "historical capture retains local identifier `{local_identifier}`: {file_name}"
            );
        }
    }
}

#[test]
fn typed_activity_rail_tmux_capture_is_bounded_sanitized_and_non_approval_grade() {
    let path = repo_root().join(
        "docs/competitive/upstream-codex/captures/typed-activity-rail-e2e-v1-linux-tmux.json",
    );
    let source = read(&path);
    let artifact: serde_json::Value =
        serde_json::from_str(&source).expect("typed rail capture must be valid JSON");

    assert_eq!(artifact["schema"], "akra-typed-activity-rail-e2e/v1");
    assert_eq!(artifact["captureRole"], "supplemental-unmatched");
    assert_eq!(
        artifact["reviewer"],
        "Codex /root/e1_e2_terminal_capability"
    );
    assert_eq!(artifact["approvalGrade"], false);
    assert_eq!(artifact["sourceBuild"], true);
    assert_eq!(artifact["syntheticAppServer"], true);
    assert_eq!(artifact["releasedRuntime"], false);
    assert_eq!(
        artifact["candidate"]["commit"],
        "0a5f06ea345bd5c24998527679158eaf8643d058"
    );
    assert_eq!(
        artifact["candidate"]["tree"],
        "7e4c0cd0de3b6609a88786eefe58074d1072d5fd"
    );
    assert_eq!(artifact["candidate"]["cleanTree"], true);
    assert_eq!(artifact["candidate"]["runtimeExitStatus"], 0);
    assert_eq!(
        artifact["candidate"]["cleanChecks"],
        serde_json::json!(["before-build", "after-build", "after-capture"])
    );
    for field in ["binarySha256", "syntheticCodexSha256"] {
        assert_sha256_json(&artifact["candidate"][field], field);
    }

    let environment = &artifact["environment"];
    assert_eq!(environment["environmentClass"], "first-class");
    assert_eq!(environment["terminal"], "tmux 3.4");
    assert_eq!(environment["multiplexer"], "tmux detached PTY");
    assert_eq!(environment["term"], "tmux-256color");
    assert_eq!(environment["frontend"], "inline");
    assert_eq!(environment["inlineHistoryRenderMode"], "HostScrollback");
    assert_eq!(environment["historyInsertionMode"], "StandardScrollRegion");
    assert_eq!(
        environment["overrides"]["CODEX_EXEC_LOOP_INLINE_HISTORY_MODE"],
        "scrollback"
    );
    assert_eq!(
        environment["overrides"]["CODEX_EXEC_LOOP_HISTORY_INSERT_MODE"],
        "standard"
    );
    assert_eq!(environment["overrides"]["WT_SESSION"], "unset");
    assert_eq!(
        environment["processEnvironment"],
        serde_json::json!({
            "inheritance": "env -i",
            "envExecutable": "/usr/bin/env",
            "nodeExecutable": "/usr/bin/node",
            "variableNames": [
                "AKRA_APP_SERVER_PROMPT_LOG",
                "AKRA_CAPTURE_SYNTHETIC",
                "AKRA_HOME",
                "CODEX_EXEC_LOOP_HISTORY_INSERT_MODE",
                "CODEX_EXEC_LOOP_INLINE_HISTORY_MODE",
                "CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART",
                "CODEX_HOME",
                "HOME",
                "LANG",
                "LC_ALL",
                "LOGNAME",
                "PATH",
                "SHELL",
                "TERM",
                "USER",
                "USERPROFILE"
            ],
            "pathEntries": ["synthetic-owner-bin", "/usr/bin", "/bin"]
        })
    );

    let checkpoints = artifact["checkpoints"]
        .as_array()
        .expect("typed rail checkpoints must be an array");
    assert_eq!(
        checkpoints
            .iter()
            .map(|checkpoint| checkpoint["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "startup",
            "active_wide",
            "active_narrow",
            "active_transition_wide",
            "active_narrow_repeat",
            "active_restored",
            "completed",
        ]
    );

    let startup = capture_checkpoint(checkpoints, "startup");
    assert_capture_checkpoint(startup, [160, 24], [2, 9], 0, 2_296);
    assert_eq!(startup["counts"]["currentReadyPrompt"], 1);
    assert_eq!(startup["counts"]["currentActivityRail"], 0);

    let active_wide = capture_checkpoint(checkpoints, "active_wide");
    assert_capture_checkpoint(active_wide, [160, 24], [2, 18], 0, 4_590);
    assert_active_capture(active_wide, 1);

    let active_narrow = capture_checkpoint(checkpoints, "active_narrow");
    let active_narrow_repeat = capture_checkpoint(checkpoints, "active_narrow_repeat");
    for (checkpoint, cursor, history_rows, raw_bytes) in [
        (active_narrow, [2, 15], 6, 5_397),
        (active_narrow_repeat, [2, 14], 11, 7_120),
    ] {
        assert_capture_checkpoint(checkpoint, [48, 18], cursor, history_rows, raw_bytes);
        assert_active_capture(checkpoint, 0);
        let visible = checkpoint["visibleEvidence"]
            .as_array()
            .expect("visible evidence must be an array");
        assert!(
            visible
                .iter()
                .any(|line| { line == "notice: activity: cmd:1 lines | active:command" })
        );
        assert!(visible.iter().all(|line| {
            line.as_str()
                .is_none_or(|line| !line.contains("model:") && !line.contains("task:"))
        }));
    }
    assert_eq!(
        active_narrow["digests"]["semanticCurrentSha256"],
        active_narrow_repeat["digests"]["semanticCurrentSha256"]
    );
    assert_eq!(
        active_narrow["digests"]["semanticHistorySha256"],
        active_narrow_repeat["digests"]["semanticHistorySha256"]
    );
    for field in ["currentSha256", "historySha256", "fullSha256"] {
        assert_ne!(
            active_narrow["digests"][field], active_narrow_repeat["digests"][field],
            "tmux raw reflow must remain distinct for {field}"
        );
    }

    let active_transition_wide = capture_checkpoint(checkpoints, "active_transition_wide");
    assert_capture_checkpoint(active_transition_wide, [80, 18], [2, 16], 6, 6_267);
    assert_active_capture(active_transition_wide, 1);
    assert!(
        active_transition_wide["visibleEvidence"]
            .as_array()
            .expect("visible evidence must be an array")
            .iter()
            .any(|line| line
                .as_str()
                .is_some_and(|line| { line.contains("model:gpt-5.6-synthetic") }))
    );

    let active_restored = capture_checkpoint(checkpoints, "active_restored");
    assert_capture_checkpoint(active_restored, [160, 24], [2, 16], 10, 8_029);
    assert_active_capture(active_restored, 1);

    let completed = capture_checkpoint(checkpoints, "completed");
    assert_capture_checkpoint(completed, [160, 24], [2, 19], 10, 10_320);
    for field in [
        "currentActivityRail",
        "currentActiveCommand",
        "currentWorkingState",
        "currentRunningPrompt",
        "historyActivityRail",
        "historyActiveCommand",
        "historyWorkingState",
        "historyTurnWorking",
        "historyInputStreaming",
        "historyModelFact",
        "historyTaskFact",
        "historyRunningPrompt",
        "fullRawSecret",
    ] {
        assert_eq!(completed["counts"][field], 0, "completed {field}");
    }
    assert_eq!(completed["counts"]["fullCommittedCanary"], 1);

    let raw = &artifact["rawPtyProof"];
    assert_eq!(raw["classification"], "ephemeral-local-observation");
    assert_eq!(raw["rawCaptureRetained"], false);
    assert_eq!(raw["digestRecomputableFromRepository"], false);
    assert_eq!(raw["bytes"], 10_320);
    assert_eq!(raw["committedCanaryOccurrences"], 1);
    assert_eq!(raw["rawSecretOccurrences"], 0);
    assert_eq!(raw["exactAnsiPayloadOccurrences"], 0);
    assert_sha256_json(&raw["sha256"], "raw PTY digest");
    assert!(
        artifact["scenarioResults"].as_object().is_some_and(
            |results| results.len() == 9 && results.values().all(|value| value == "pass")
        )
    );
    assert!(artifact["assertions"].as_object().is_some_and(
        |assertions| assertions.len() == 15 && assertions.values().all(|value| value == true)
    ));

    let non_claims = artifact["nonClaims"]
        .as_array()
        .expect("non-claims must be an array");
    assert!(non_claims.iter().any(|claim| {
        claim.as_str().is_some_and(|claim| {
            claim.contains("ephemeral local observation")
                && claim.contains("not retained for repository recomputation")
        })
    }));
    assert!(non_claims.iter().any(|claim| {
        claim
            .as_str()
            .is_some_and(|claim| claim.contains("not approval-grade E1-E4"))
    }));
    assert!(non_claims.iter().any(|claim| {
        claim
            .as_str()
            .is_some_and(|claim| claim.contains("below the 16-row inline viewport"))
    }));

    for forbidden in [
        "D4_RAW_ACTIVITY_SECRET",
        "\\u001b",
        "/dev/pts/",
        "/tmp/",
        "/home/",
        "C:\\Users\\",
        "@",
    ] {
        assert!(
            !source.contains(forbidden),
            "typed rail capture contains forbidden value: {forbidden}"
        );
    }
    assert!(source.chars().all(|character| {
        character == '\n'
            || (!character.is_control() && !('\u{7f}'..='\u{9f}').contains(&character))
    }));
    assert_sanitized_capture_json(&artifact);
}

#[test]
fn temporary_output_directory_is_ignored_and_has_no_tracked_files() {
    let repo = repo_root();
    let ignore = read(&repo.join(".gitignore"));
    assert!(
        ignore.lines().any(|line| line.trim() == "/tmp/"),
        "repository-local temporary output must be ignored"
    );

    if repo.join(".git").exists() {
        let output = Command::new("git")
            .args(["-C", repo.to_str().expect("repository path must be UTF-8")])
            .args(["ls-files", "--", "tmp"])
            .output()
            .expect("git must be available for repository hygiene tests");
        assert!(output.status.success(), "git ls-files tmp must succeed");
        assert!(
            output.stdout.is_empty(),
            "tmp/ must not contain tracked scratch files: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn tracked_lf_files_are_normalized_in_the_git_index() {
    let repo = repo_root();
    if !repo.join(".git").exists() {
        return;
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["ls-files", "--eol", "-z"])
        .output()
        .expect("git must be available for repository hygiene tests");
    assert!(output.status.success(), "git ls-files --eol must succeed");

    let records = String::from_utf8(output.stdout)
        .expect("tracked repository paths and eol metadata must be UTF-8");
    let non_normalized = records
        .split('\0')
        .filter(|record| record.contains("attr/text eol=lf"))
        .filter_map(|record| {
            let (metadata, path) = record.split_once('\t')?;
            (!metadata.split_whitespace().any(|field| field == "i/lf")).then(|| path.to_string())
        })
        .collect::<Vec<_>>();

    assert!(
        non_normalized.is_empty(),
        "tracked files declared as eol=lf must store normalized LF blobs so fresh pool worktrees stay clean: {non_normalized:?}"
    );
}

#[test]
fn oss_application_metrics_are_explicitly_archival_until_refreshed() {
    let document = read(&repo_root().join("docs/plan/14-codex-for-oss-application.md"));
    assert!(document.contains("Archived Public-Signal Snapshot (Not Current)"));
    assert!(document.contains("Refresh immediately before submission"));
    assert!(!document.contains("## Current Public Signals"));
    assert!(!document.contains("최근 1개월 523 downloads"));
}

fn admin_graphic_names(source: &str, route_prefix: &str) -> BTreeSet<String> {
    source
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|value| value.ends_with(".png"))
        .filter_map(|value| {
            if let Some(asset) = value.strip_prefix(route_prefix) {
                return Some(asset.to_string());
            }
            (!value.contains('/') && !value.contains('\\')).then(|| value.to_string())
        })
        .collect()
}

fn capture_checkpoint<'a>(
    checkpoints: &'a [serde_json::Value],
    name: &str,
) -> &'a serde_json::Value {
    checkpoints
        .iter()
        .find(|checkpoint| checkpoint["name"] == name)
        .unwrap_or_else(|| panic!("missing typed rail checkpoint: {name}"))
}

fn assert_capture_checkpoint(
    checkpoint: &serde_json::Value,
    geometry: [u64; 2],
    cursor: [u64; 2],
    history_rows: u64,
    raw_pty_bytes: u64,
) {
    assert_eq!(checkpoint["geometry"]["width"], geometry[0]);
    assert_eq!(checkpoint["geometry"]["height"], geometry[1]);
    assert_eq!(checkpoint["cursor"]["x"], cursor[0]);
    assert_eq!(checkpoint["cursor"]["y"], cursor[1]);
    assert_eq!(checkpoint["historyRows"], history_rows);
    assert_eq!(checkpoint["rawPtyBytesObserved"], raw_pty_bytes);
    for digest in [
        "currentSha256",
        "historySha256",
        "fullSha256",
        "semanticCurrentSha256",
        "semanticHistorySha256",
    ] {
        assert_sha256_json(&checkpoint["digests"][digest], digest);
    }
}

fn assert_active_capture(checkpoint: &serde_json::Value, model_count: u64) {
    for field in [
        "currentActiveCommand",
        "currentActivityRail",
        "currentWorkingState",
        "currentRunningPrompt",
    ] {
        assert_eq!(checkpoint["counts"][field], 1, "active {field}");
    }
    assert_eq!(checkpoint["counts"]["currentModelFact"], model_count);
    for field in [
        "historyActivityRail",
        "historyActiveCommand",
        "historyWorkingState",
        "historyTurnWorking",
        "historyInputStreaming",
        "historyModelFact",
        "historyTaskFact",
        "historyRunningPrompt",
        "fullRawSecret",
    ] {
        assert_eq!(checkpoint["counts"][field], 0, "active {field}");
    }
}

fn assert_sanitized_capture_json(value: &serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                assert_sanitized_capture_json(value);
            }
        }
        serde_json::Value::Object(entries) => {
            for (key, value) in entries {
                assert_sanitized_capture_text(key);
                assert_sanitized_capture_json(value);
            }
        }
        serde_json::Value::String(value) => assert_sanitized_capture_text(value),
        _ => {}
    }
}

fn assert_sanitized_capture_text(value: &str) {
    for forbidden in [
        "D4_RAW_ACTIVITY_SECRET",
        "/dev/pts/",
        "/tmp/",
        "/home/",
        "C:\\Users\\",
        "@",
    ] {
        assert!(
            !value.contains(forbidden),
            "decoded typed rail capture contains forbidden value: {forbidden}"
        );
    }
    assert!(value
        .chars()
        .all(|character| !character.is_control() && !('\u{7f}'..='\u{9f}').contains(&character)));
}

fn assert_sha256_json(value: &serde_json::Value, label: &str) {
    let digest = value
        .as_str()
        .unwrap_or_else(|| panic!("{label} must be a string"));
    assert_eq!(digest.len(), 64, "{label} length");
    assert!(
        digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} must be lowercase hex"
    );
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
