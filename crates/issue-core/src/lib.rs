//! Issue policy, shared copies, masking, validation and template parsing.
//! I/O belongs to source-port implementations in the product root.
//! Validated evidence cannot be constructed or deserialized by consumers.
//!
//! ```compile_fail
//! use akra_issue_core::issue_validation::IssueValidationPassed;
//! use akra_issue_core::issue_work::IssueSharedCopy;
//! let forged = IssueValidationPassed { shared_copy: IssueSharedCopy { title: "x".into(), body: "y".into() } };
//! ```
//!
//! ```compile_fail
//! use akra_issue_core::issue_validation::IssueValidationPassed;
//! let forged: IssueValidationPassed = serde_json::from_str("{}").unwrap();
//! ```
#![deny(unsafe_code)]

pub mod issue_creation;
/// Shared Markdown syntax used by issue validation, templates, and root triage.
pub mod issue_markdown;
pub mod issue_masking;
pub mod issue_policy;
pub mod issue_policy_source_port;
pub mod issue_publish_intent;
pub mod issue_review;
pub mod issue_secret_spans;
pub mod issue_template;
pub mod issue_template_source_port;
pub mod issue_validation;
pub mod issue_work;
pub mod policy;
pub mod template;
