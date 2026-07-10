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

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
