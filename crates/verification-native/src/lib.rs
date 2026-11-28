//! The fixed host Git recipe and shared host Git isolation policy.
//! Native execution facts do not issue task, lease, or delivery authority.

pub mod git_command;
pub mod git_config;
mod task_verification;

pub use task_verification::GitTaskVerificationRunnerAdapter;

/// Shows the pending decision in a default-cancel native dialog, never in a repository cwd.
/// A missing dialog, timeout, error or dismissal returns false.
pub fn confirm_evidence_review(message: &str) -> bool {
    // Unlike a live kernel's trusted verifier environment, this helper can be
    // launched independently. A caller-selected X/Wayland server cannot
    // authenticate a human review. Keep that platform read-only.
    if cfg!(target_os = "linux") {
        return false;
    }
    let Ok(executable) = std::env::current_exe() else {
        return false;
    };
    let Some(cwd) = executable.parent() else {
        return false;
    };
    task_verification::confirm_text(
        cwd,
        message,
        std::time::Instant::now() + std::time::Duration::from_secs(120),
        &|| false,
    )
    .is_ok()
}

#[cfg(test)]
mod test_support;
