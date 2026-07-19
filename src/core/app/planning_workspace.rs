use crate::domain::planning::PlanningValidationReport;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkspaceResetTarget {
    Queue,
    Directions,
    All,
}

impl PlanningWorkspaceResetTarget {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetIntent {
    pub workspace_directory: String,
    pub target: PlanningWorkspaceResetTarget,
}

impl PlanningWorkspaceResetIntent {
    pub fn new(
        workspace_directory: impl Into<String>,
        target: PlanningWorkspaceResetTarget,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningEditorSessionIdentity {
    pub generation: u64,
    pub workspace_directory: String,
    pub draft_name: String,
}

impl PlanningEditorSessionIdentity {
    pub fn new(
        generation: u64,
        workspace_directory: impl Into<String>,
        draft_name: impl Into<String>,
    ) -> Self {
        Self {
            generation,
            workspace_directory: workspace_directory.into(),
            draft_name: draft_name.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkspaceOperationKind {
    Reset {
        target: PlanningWorkspaceResetTarget,
    },
    StageSimpleDraft,
    LoadSimpleEditor {
        draft_name: String,
        source_session: PlanningEditorSessionIdentity,
    },
    PromoteSimpleDraft {
        draft_name: String,
        source_session: PlanningEditorSessionIdentity,
    },
}

impl PlanningWorkspaceOperationKind {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Reset { .. } => "reset",
            Self::StageSimpleDraft => "simple draft staging",
            Self::LoadSimpleEditor { .. } => "simple draft editor loading",
            Self::PromoteSimpleDraft { .. } => "simple draft promotion",
        }
    }

    pub fn draft_name(&self) -> Option<&str> {
        match self {
            Self::LoadSimpleEditor { draft_name, .. }
            | Self::PromoteSimpleDraft { draft_name, .. } => Some(draft_name),
            Self::Reset { .. } | Self::StageSimpleDraft => None,
        }
    }

    pub fn source_session(&self) -> Option<&PlanningEditorSessionIdentity> {
        match self {
            Self::LoadSimpleEditor { source_session, .. }
            | Self::PromoteSimpleDraft { source_session, .. } => Some(source_session),
            Self::Reset { .. } | Self::StageSimpleDraft => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceOperationIntent {
    pub workspace_directory: String,
    pub operation: PlanningWorkspaceOperationKind,
}

impl PlanningWorkspaceOperationIntent {
    pub fn reset(intent: PlanningWorkspaceResetIntent) -> Self {
        Self {
            workspace_directory: intent.workspace_directory,
            operation: PlanningWorkspaceOperationKind::Reset {
                target: intent.target,
            },
        }
    }

    pub fn stage_simple_draft(workspace_directory: impl Into<String>) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            operation: PlanningWorkspaceOperationKind::StageSimpleDraft,
        }
    }

    pub fn load_simple_editor(
        workspace_directory: impl Into<String>,
        draft_name: impl Into<String>,
        source_session: PlanningEditorSessionIdentity,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            operation: PlanningWorkspaceOperationKind::LoadSimpleEditor {
                draft_name: draft_name.into(),
                source_session,
            },
        }
    }

    pub fn promote_simple_draft(
        workspace_directory: impl Into<String>,
        draft_name: impl Into<String>,
        source_session: PlanningEditorSessionIdentity,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            operation: PlanningWorkspaceOperationKind::PromoteSimpleDraft {
                draft_name: draft_name.into(),
                source_session,
            },
        }
    }

    pub fn label(&self) -> &'static str {
        self.operation.label()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceOperationCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
    pub operation: PlanningWorkspaceOperationKind,
}

impl PlanningWorkspaceOperationCorrelation {
    fn from_intent(generation: u64, intent: PlanningWorkspaceOperationIntent) -> Self {
        Self {
            generation,
            workspace_directory: intent.workspace_directory,
            operation: intent.operation,
        }
    }

    fn matches_intent(&self, intent: &PlanningWorkspaceOperationIntent) -> bool {
        self.workspace_directory == intent.workspace_directory && self.operation == intent.operation
    }

    pub fn reset_target(&self) -> Option<PlanningWorkspaceResetTarget> {
        match self.operation {
            PlanningWorkspaceOperationKind::Reset { target } => Some(target),
            _ => None,
        }
    }

    pub fn draft_name(&self) -> Option<&str> {
        self.operation.draft_name()
    }

    pub fn source_session(&self) -> Option<&PlanningEditorSessionIdentity> {
        self.operation.source_session()
    }

    pub fn label(&self) -> &'static str {
        self.operation.label()
    }

    pub fn editor_session_identity(
        &self,
        draft_name: impl Into<String>,
    ) -> PlanningEditorSessionIdentity {
        PlanningEditorSessionIdentity::new(
            self.generation,
            self.workspace_directory.clone(),
            draft_name,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkspaceOperationAdmission {
    Started {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    Coalesced {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    Busy {
        active_correlation: PlanningWorkspaceOperationCorrelation,
        requested: PlanningWorkspaceOperationIntent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetSnapshot {
    pub target: PlanningWorkspaceResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningSimpleDraftStageSnapshot {
    pub session_identity: PlanningEditorSessionIdentity,
    pub staged_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlanningEditorFileSnapshot {
    pub active_path: String,
    pub staged_path: String,
    pub body: String,
}

impl fmt::Debug for PlanningEditorFileSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanningEditorFileSnapshot")
            .field("active_path", &self.active_path)
            .field("staged_path", &self.staged_path)
            .field("body", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlanningEditorSessionSnapshot {
    pub session_identity: PlanningEditorSessionIdentity,
    pub draft_directory: String,
    pub editable_files: Vec<PlanningEditorFileSnapshot>,
    pub validation_report: PlanningValidationReport,
}

impl fmt::Debug for PlanningEditorSessionSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanningEditorSessionSnapshot")
            .field("session_identity", &self.session_identity)
            .field("draft_directory", &self.draft_directory)
            .field("editable_file_count", &self.editable_files.len())
            .field("validation_report", &self.validation_report)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningSimpleDraftPromotionSnapshot {
    pub draft_name: String,
    pub promoted_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone)]
pub(crate) struct PlanningWorkspaceOperationCoordinator {
    next_generation: u64,
    active: Option<PlanningWorkspaceOperationCorrelation>,
}

impl PlanningWorkspaceOperationCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
        }
    }

    pub(crate) fn begin(
        &mut self,
        intent: PlanningWorkspaceOperationIntent,
    ) -> PlanningWorkspaceOperationAdmission {
        if let Some(active) = self.active.as_ref() {
            return if active.matches_intent(&intent) {
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: active.clone(),
                }
            } else {
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation: active.clone(),
                    requested: intent,
                }
            };
        }

        let correlation =
            PlanningWorkspaceOperationCorrelation::from_intent(self.take_generation(), intent);
        self.active = Some(correlation.clone());
        PlanningWorkspaceOperationAdmission::Started { correlation }
    }

    pub(crate) fn accept(&mut self, correlation: &PlanningWorkspaceOperationCorrelation) -> bool {
        if self.active.as_ref() != Some(correlation) {
            return false;
        }
        self.active = None;
        true
    }

    fn take_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .expect("planning workspace operation generation exhausted");
        generation
    }
}

impl Default for PlanningWorkspaceOperationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset(
        workspace_directory: &str,
        target: PlanningWorkspaceResetTarget,
    ) -> PlanningWorkspaceOperationIntent {
        PlanningWorkspaceOperationIntent::reset(PlanningWorkspaceResetIntent::new(
            workspace_directory,
            target,
        ))
    }

    fn session(
        generation: u64,
        workspace_directory: &str,
        draft_name: &str,
    ) -> PlanningEditorSessionIdentity {
        PlanningEditorSessionIdentity::new(generation, workspace_directory, draft_name)
    }

    fn started(
        admission: PlanningWorkspaceOperationAdmission,
    ) -> PlanningWorkspaceOperationCorrelation {
        let PlanningWorkspaceOperationAdmission::Started { correlation } = admission else {
            panic!("expected a started planning workspace operation");
        };
        correlation
    }

    #[test]
    fn exact_duplicate_operations_coalesce_without_consuming_a_generation() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let intent = PlanningWorkspaceOperationIntent::load_simple_editor(
            "/workspace",
            "draft-a",
            session(7, "/workspace", "draft-a"),
        );
        let first = started(coordinator.begin(intent.clone()));

        assert_eq!(
            coordinator.begin(intent.clone()),
            PlanningWorkspaceOperationAdmission::Coalesced {
                correlation: first.clone(),
            }
        );
        assert!(coordinator.accept(&first));

        let second = started(coordinator.begin(intent));
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
    }

    #[test]
    fn a_different_kind_target_draft_session_or_workspace_is_busy() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let source = session(7, "/workspace-a", "draft-a");
        let active = started(coordinator.begin(
            PlanningWorkspaceOperationIntent::load_simple_editor(
                "/workspace-a",
                "draft-a",
                source.clone(),
            ),
        ));

        for requested in [
            reset("/workspace-a", PlanningWorkspaceResetTarget::All),
            PlanningWorkspaceOperationIntent::load_simple_editor(
                "/workspace-b",
                "draft-a",
                source.clone(),
            ),
            PlanningWorkspaceOperationIntent::load_simple_editor(
                "/workspace-a",
                "draft-b",
                source.clone(),
            ),
            PlanningWorkspaceOperationIntent::load_simple_editor(
                "/workspace-a",
                "draft-a",
                session(8, "/workspace-a", "draft-a"),
            ),
            PlanningWorkspaceOperationIntent::promote_simple_draft(
                "/workspace-a",
                "draft-a",
                source,
            ),
        ] {
            assert_eq!(
                coordinator.begin(requested.clone()),
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation: active.clone(),
                    requested,
                }
            );
        }
    }

    #[test]
    fn only_the_exact_completion_settles_and_old_generation_cannot_win_aba() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let first = started(coordinator.begin(reset(
            "/workspace",
            PlanningWorkspaceResetTarget::Directions,
        )));
        let mut stale_generation = first.clone();
        stale_generation.generation += 1;
        let mut stale_workspace = first.clone();
        stale_workspace.workspace_directory = "/other".to_string();
        let mut stale_target = first.clone();
        stale_target.operation = PlanningWorkspaceOperationKind::Reset {
            target: PlanningWorkspaceResetTarget::All,
        };

        assert!(!coordinator.accept(&stale_generation));
        assert!(!coordinator.accept(&stale_workspace));
        assert!(!coordinator.accept(&stale_target));
        assert!(coordinator.accept(&first));
        assert!(!coordinator.accept(&first));

        let second = started(coordinator.begin(reset(
            "/workspace",
            PlanningWorkspaceResetTarget::Directions,
        )));
        assert_ne!(first.generation, second.generation);
        assert!(!coordinator.accept(&first));
        assert!(coordinator.accept(&second));
    }

    #[test]
    fn editor_debug_output_redacts_buffer_payloads() {
        let snapshot = PlanningEditorSessionSnapshot {
            session_identity: session(1, "/workspace", "draft-a"),
            draft_directory: "/workspace/drafts/draft-a".to_string(),
            editable_files: vec![PlanningEditorFileSnapshot {
                active_path: "result-output.md".to_string(),
                staged_path: "drafts/draft-a/result-output.md".to_string(),
                body: "secret operator payload".to_string(),
            }],
            validation_report: PlanningValidationReport::default(),
        };

        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("secret operator payload"));
        assert!(debug.contains("editable_file_count"));
    }
}
