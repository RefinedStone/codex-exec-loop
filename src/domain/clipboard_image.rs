use std::path::PathBuf;

/*
 * Clipboard image attachment values are pure data shared between the outbound
 * clipboard adapter, the composition-owned probe worker, and the TUI composer.
 * Keeping them in domain lets the inbound adapter consume probe results without
 * touching process/thread capabilities.
 */

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImageAttachment {
    pub path: PathBuf,
    pub display_name: String,
}

/*
 * Probe outcome contract: `NoImage` means the clipboard simply holds no image
 * (the composer stays untouched), while `Failed` carries operator-facing copy
 * for a probe that could not be completed.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardImageProbeOutcome {
    Attached(ClipboardImageAttachment),
    NoImage,
    Failed(String),
}
