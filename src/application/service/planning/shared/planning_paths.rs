use std::path::{Component, Path};

pub(crate) fn is_valid_planning_markdown_path(path: &str, required_prefix: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized.contains("/..")
        || Path::new(&normalized)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }

    let Some(suffix) = normalized.strip_prefix(required_prefix) else {
        return false;
    };

    suffix.starts_with('/') && suffix.len() > 1 && normalized.ends_with(".md")
}
