use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy)]
struct TemporaryAllowance {
    path_suffix: &'static str,
    pattern: &'static str,
    reason: &'static str,
}

#[derive(Clone, Copy)]
struct BoundaryRule {
    name: &'static str,
    root: &'static str,
    forbidden_patterns: &'static [&'static str],
    temporary_allowances: &'static [TemporaryAllowance],
}

struct SourceLine {
    number: usize,
    text: String,
}

struct BoundaryViolation {
    rule: &'static str,
    path: String,
    line: usize,
    pattern: &'static str,
    text: String,
}

#[test]
fn domain_layer_has_no_application_or_adapter_dependencies() {
    assert_no_forbidden_references(BoundaryRule {
        name: "domain must stay independent from application and adapters",
        root: "src/domain",
        forbidden_patterns: &["crate::application::", "crate::adapter::"],
        temporary_allowances: &[],
    });
}

#[test]
fn application_layer_has_no_concrete_adapter_dependencies() {
    assert_no_forbidden_references(BoundaryRule {
        name: "application must depend on ports and domain, not concrete adapters",
        root: "src/application",
        forbidden_patterns: &["crate::adapter::"],
        temporary_allowances: &[],
    });
}

#[test]
fn inbound_adapters_only_wire_outbound_adapters_in_explicit_composition_roots() {
    assert_no_forbidden_references(BoundaryRule {
        name: "inbound adapters must not pull outbound adapters outside explicit composition roots",
        root: "src/adapter/inbound",
        forbidden_patterns: &["crate::adapter::outbound::"],
        temporary_allowances: &[
            TemporaryAllowance {
                path_suffix: "src/adapter/inbound/cli.rs",
                pattern: "crate::adapter::outbound::",
                reason: "CLI still owns production wiring until application composition is centralized.",
            },
            TemporaryAllowance {
                path_suffix: "src/adapter/inbound/admin_api/mod.rs",
                pattern: "crate::adapter::outbound::",
                reason: "Admin API still owns production wiring until application composition is centralized.",
            },
            TemporaryAllowance {
                path_suffix: "src/adapter/inbound/telegram_bot/mod.rs",
                pattern: "crate::adapter::outbound::",
                reason: "Telegram bot still owns production wiring until application composition is centralized.",
            },
            TemporaryAllowance {
                path_suffix: "src/adapter/inbound/tui/app/shell_entrypoint.rs",
                pattern: "crate::adapter::outbound::",
                reason: "Native TUI shell entrypoint is the current production wiring boundary.",
            },
            TemporaryAllowance {
                path_suffix: "src/adapter/inbound/tui/app/github_polling.rs",
                pattern: "crate::adapter::outbound::",
                reason: "GitHub review polling still builds its adapter from the TUI edge.",
            },
        ],
    });
}

#[test]
fn outbound_adapters_do_not_depend_on_inbound_adapters() {
    assert_no_forbidden_references(BoundaryRule {
        name: "outbound adapters must not depend on inbound adapters",
        root: "src/adapter/outbound",
        forbidden_patterns: &["crate::adapter::inbound::"],
        temporary_allowances: &[],
    });
}

#[test]
fn outbound_port_modules_follow_port_naming_contract() {
    let repo_root = repo_root();
    let port_root = repo_root.join("src/application/port/outbound");
    let mut violations = Vec::new();

    for path in rust_files_under(&port_root) {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name == "mod.rs" || is_test_only_path(&path) {
            continue;
        }
        if !file_name.ends_with("_port.rs") {
            violations.push(format!(
                "{}: outbound port modules must use the *_port.rs suffix",
                relative_path(&repo_root, &path)
            ));
            continue;
        }

        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        if !source.contains("trait ") || !source.contains("Port") {
            violations.push(format!(
                "{}: outbound port module must define a Port trait contract",
                relative_path(&repo_root, &path)
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "outbound port naming contract violations:\n{}",
        violations.join("\n")
    );
}

fn assert_no_forbidden_references(rule: BoundaryRule) {
    let repo_root = repo_root();
    let root = repo_root.join(rule.root);
    let mut violations = Vec::new();

    for path in rust_files_under(&root) {
        if is_test_only_path(&path) {
            continue;
        }

        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let relative_path = relative_path(&repo_root, &path);

        for source_line in production_lines(&source) {
            if is_comment_only_line(&source_line.text) {
                continue;
            }

            for pattern in rule.forbidden_patterns {
                if source_line.text.contains(pattern)
                    && !is_temporarily_allowed(rule, &relative_path, pattern)
                {
                    violations.push(BoundaryViolation {
                        rule: rule.name,
                        path: relative_path.clone(),
                        line: source_line.number,
                        pattern,
                        text: source_line.text.trim().to_string(),
                    });
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        format_violations(&violations)
    );
}

fn is_temporarily_allowed(rule: BoundaryRule, path: &str, pattern: &str) -> bool {
    rule.temporary_allowances.iter().any(|allowance| {
        debug_assert!(!allowance.reason.trim().is_empty());
        path.ends_with(allowance.path_suffix) && pattern == allowance.pattern
    })
}

fn format_violations(violations: &[BoundaryViolation]) -> String {
    violations
        .iter()
        .map(|violation| {
            format!(
                "{}:{}: {} matched `{}` in rule `{}`",
                violation.path, violation.line, violation.text, violation.pattern, violation.rule
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn production_lines(source: &str) -> Vec<SourceLine> {
    let mut lines = Vec::new();
    let mut skip_cfg_test_item = false;
    let mut skipping_cfg_test_block = false;
    let mut cfg_test_block_depth = 0isize;

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        if skipping_cfg_test_block {
            cfg_test_block_depth += brace_delta(line);
            if cfg_test_block_depth <= 0 {
                skipping_cfg_test_block = false;
                cfg_test_block_depth = 0;
            }
            continue;
        }

        if skip_cfg_test_item {
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }

            if line.contains('{') {
                skipping_cfg_test_block = true;
                cfg_test_block_depth = brace_delta(line);
                if cfg_test_block_depth <= 0 {
                    skipping_cfg_test_block = false;
                    cfg_test_block_depth = 0;
                }
            } else if trimmed.ends_with(';') {
                skip_cfg_test_item = false;
            }
            continue;
        }

        if is_cfg_test_attribute(trimmed) {
            skip_cfg_test_item = true;
            continue;
        }

        lines.push(SourceLine {
            number: index + 1,
            text: line.to_string(),
        });
    }

    lines
}

fn is_cfg_test_attribute(line: &str) -> bool {
    line.starts_with("#[cfg(test")
        || line.starts_with("#[cfg_attr(test")
        || line.contains("cfg(test)")
}

fn brace_delta(line: &str) -> isize {
    line.chars()
        .fold(0isize, |depth, character| match character {
            '{' => depth + 1,
            '}' => depth - 1,
            _ => depth,
        })
}

fn is_comment_only_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*')
}

fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    if !path.exists() {
        return;
    }

    if path.is_file() {
        if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path.to_path_buf());
        }
        return;
    }

    for entry in fs::read_dir(path).unwrap_or_else(|error| {
        panic!("failed to read directory {}: {error}", path.display());
    }) {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to read directory entry in {}: {error}",
                path.display()
            );
        });
        collect_rust_files(&entry.path(), files);
    }
}

fn is_test_only_path(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    file_name == "tests.rs"
        || file_name == "test_helpers.rs"
        || file_name == "fixtures.rs"
        || file_name == "tui_testkit.rs"
        || file_name == "contract_tests.rs"
        || file_name.ends_with("_tests.rs")
        || path.components().any(|component| match component {
            Component::Normal(value) => {
                value == "tests"
                    || value == "snapshots"
                    || value == "fixtures"
                    || value.to_string_lossy().ends_with("_tests")
            }
            _ => false,
        })
}

fn relative_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
