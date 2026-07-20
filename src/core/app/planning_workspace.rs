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
pub enum PlanningEditorStageTarget {
    PlanningManual,
    DirectionDetail { direction_id: String },
    QueueIdlePrompt,
}

impl PlanningEditorStageTarget {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::PlanningManual => "planning manual editor",
            Self::DirectionDetail { .. } => "direction detail editor",
            Self::QueueIdlePrompt => "queue-idle prompt editor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningEditorMutationAction {
    Save,
    Promote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningEditorMutationTarget {
    Planning,
    Directions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningEditorMutationIdentity {
    pub action: PlanningEditorMutationAction,
    pub target: PlanningEditorMutationTarget,
    pub draft_name: String,
    pub source_session: PlanningEditorSessionIdentity,
    pub buffer_revision: u64,
    pub source_planning_revision: Option<i64>,
}

impl PlanningEditorMutationIdentity {
    pub fn new(
        action: PlanningEditorMutationAction,
        target: PlanningEditorMutationTarget,
        draft_name: impl Into<String>,
        source_session: PlanningEditorSessionIdentity,
        buffer_revision: u64,
    ) -> Self {
        Self {
            action,
            target,
            draft_name: draft_name.into(),
            source_session,
            buffer_revision,
            source_planning_revision: None,
        }
    }

    pub fn with_source_planning_revision(mut self, source_planning_revision: i64) -> Self {
        self.source_planning_revision = Some(source_planning_revision);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkspaceOperationKind {
    Reset {
        target: PlanningWorkspaceResetTarget,
    },
    StageSimpleDraft,
    StageEditor {
        target: PlanningEditorStageTarget,
    },
    MutateEditor {
        identity: PlanningEditorMutationIdentity,
    },
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
            Self::StageEditor { target } => target.label(),
            Self::MutateEditor { .. } => "editor mutation",
            Self::LoadSimpleEditor { .. } => "simple draft editor loading",
            Self::PromoteSimpleDraft { .. } => "simple draft promotion",
        }
    }

    pub fn draft_name(&self) -> Option<&str> {
        match self {
            Self::MutateEditor { identity } => Some(identity.draft_name.as_str()),
            Self::LoadSimpleEditor { draft_name, .. }
            | Self::PromoteSimpleDraft { draft_name, .. } => Some(draft_name),
            Self::Reset { .. } | Self::StageSimpleDraft | Self::StageEditor { .. } => None,
        }
    }

    pub fn source_session(&self) -> Option<&PlanningEditorSessionIdentity> {
        match self {
            Self::MutateEditor { identity } => Some(&identity.source_session),
            Self::LoadSimpleEditor { source_session, .. }
            | Self::PromoteSimpleDraft { source_session, .. } => Some(source_session),
            Self::Reset { .. } | Self::StageSimpleDraft | Self::StageEditor { .. } => None,
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

    pub fn stage_editor(
        workspace_directory: impl Into<String>,
        target: PlanningEditorStageTarget,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            operation: PlanningWorkspaceOperationKind::StageEditor { target },
        }
    }

    pub fn mutate_editor(
        workspace_directory: impl Into<String>,
        identity: PlanningEditorMutationIdentity,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            operation: PlanningWorkspaceOperationKind::MutateEditor { identity },
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

    pub fn editor_stage_target(&self) -> Option<&PlanningEditorStageTarget> {
        match &self.operation {
            PlanningWorkspaceOperationKind::StageEditor { target } => Some(target),
            _ => None,
        }
    }

    pub fn editor_mutation_identity(&self) -> Option<&PlanningEditorMutationIdentity> {
        match &self.operation {
            PlanningWorkspaceOperationKind::MutateEditor { identity } => Some(identity),
            _ => None,
        }
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
    pub source_planning_revision: Option<i64>,
}

impl fmt::Debug for PlanningEditorSessionSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanningEditorSessionSnapshot")
            .field("session_identity", &self.session_identity)
            .field("draft_directory", &self.draft_directory)
            .field("editable_file_count", &self.editable_files.len())
            .field("validation_report", &self.validation_report)
            .field("source_planning_revision", &self.source_planning_revision)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlanningEditorMutationRequest {
    pub identity: PlanningEditorMutationIdentity,
    pub editable_files: Vec<PlanningEditorFileSnapshot>,
}

impl fmt::Debug for PlanningEditorMutationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanningEditorMutationRequest")
            .field("identity", &self.identity)
            .field("editable_file_count", &self.editable_files.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningEditorMutationResult {
    Saved {
        identity: PlanningEditorMutationIdentity,
        draft_name: String,
        validation_report: PlanningValidationReport,
    },
    Promoted {
        identity: PlanningEditorMutationIdentity,
        draft_name: String,
        promoted_file_count: usize,
        validation_report: PlanningValidationReport,
        committed_planning_revision: Option<i64>,
    },
}

impl PlanningEditorMutationResult {
    pub fn identity(&self) -> &PlanningEditorMutationIdentity {
        match self {
            Self::Saved { identity, .. } | Self::Promoted { identity, .. } => identity,
        }
    }

    pub fn draft_name(&self) -> &str {
        match self {
            Self::Saved { draft_name, .. } | Self::Promoted { draft_name, .. } => draft_name,
        }
    }

    pub const fn action(&self) -> PlanningEditorMutationAction {
        match self {
            Self::Saved { .. } => PlanningEditorMutationAction::Save,
            Self::Promoted { .. } => PlanningEditorMutationAction::Promote,
        }
    }

    pub fn validation_report(&self) -> &PlanningValidationReport {
        match self {
            Self::Saved {
                validation_report, ..
            }
            | Self::Promoted {
                validation_report, ..
            } => validation_report,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningEditorStageSnapshot {
    pub target: PlanningEditorStageTarget,
    pub session: PlanningEditorSessionSnapshot,
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

    fn mutation_identity(
        action: PlanningEditorMutationAction,
        target: PlanningEditorMutationTarget,
        draft_name: &str,
        source: PlanningEditorSessionIdentity,
        buffer_revision: u64,
    ) -> PlanningEditorMutationIdentity {
        PlanningEditorMutationIdentity::new(action, target, draft_name, source, buffer_revision)
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
    fn editor_stage_only_coalesces_the_exact_workspace_and_target() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let intent = PlanningWorkspaceOperationIntent::stage_editor(
            "/workspace",
            PlanningEditorStageTarget::DirectionDetail {
                direction_id: "direction-a".to_string(),
            },
        );
        let active = started(coordinator.begin(intent.clone()));
        assert_eq!(
            coordinator.begin(intent),
            PlanningWorkspaceOperationAdmission::Coalesced {
                correlation: active.clone(),
            }
        );

        for requested in [
            PlanningWorkspaceOperationIntent::stage_editor(
                "/workspace",
                PlanningEditorStageTarget::DirectionDetail {
                    direction_id: "direction-b".to_string(),
                },
            ),
            PlanningWorkspaceOperationIntent::stage_editor(
                "/workspace",
                PlanningEditorStageTarget::QueueIdlePrompt,
            ),
            PlanningWorkspaceOperationIntent::stage_editor(
                "/other",
                PlanningEditorStageTarget::DirectionDetail {
                    direction_id: "direction-a".to_string(),
                },
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
    fn editor_mutation_coalesces_only_the_exact_identity_and_rejects_aba_completion() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let source = session(9, "/workspace", "draft-a");
        let identity = mutation_identity(
            PlanningEditorMutationAction::Save,
            PlanningEditorMutationTarget::Planning,
            "draft-a",
            source.clone(),
            3,
        );
        let intent =
            PlanningWorkspaceOperationIntent::mutate_editor("/workspace", identity.clone());
        let first = started(coordinator.begin(intent.clone()));
        assert_eq!(
            coordinator.begin(intent.clone()),
            PlanningWorkspaceOperationAdmission::Coalesced {
                correlation: first.clone(),
            }
        );

        for requested in [
            PlanningWorkspaceOperationIntent::mutate_editor("/other", identity.clone()),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    PlanningEditorMutationAction::Promote,
                    identity.target,
                    "draft-a",
                    source.clone(),
                    3,
                ),
            ),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    identity.action,
                    PlanningEditorMutationTarget::Directions,
                    "draft-a",
                    source.clone(),
                    3,
                ),
            ),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    identity.action,
                    identity.target,
                    "draft-b",
                    source.clone(),
                    3,
                ),
            ),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    identity.action,
                    identity.target,
                    "draft-a",
                    session(9, "/other", "draft-a"),
                    3,
                ),
            ),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    identity.action,
                    identity.target,
                    "draft-a",
                    session(9, "/workspace", "draft-b"),
                    3,
                ),
            ),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    identity.action,
                    identity.target,
                    "draft-a",
                    session(10, "/workspace", "draft-a"),
                    3,
                ),
            ),
            PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    identity.action,
                    identity.target,
                    "draft-a",
                    source.clone(),
                    4,
                ),
            ),
        ] {
            assert!(matches!(
                coordinator.begin(requested),
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation,
                    ..
                } if active_correlation == first
            ));
        }

        assert!(coordinator.accept(&first));
        let second = started(
            coordinator.begin(PlanningWorkspaceOperationIntent::mutate_editor(
                "/workspace",
                mutation_identity(
                    PlanningEditorMutationAction::Promote,
                    PlanningEditorMutationTarget::Planning,
                    "draft-a",
                    source,
                    3,
                ),
            )),
        );
        assert!(coordinator.accept(&second));
        let third = started(coordinator.begin(intent));
        assert!(!coordinator.accept(&first));
        assert!(coordinator.accept(&third));
        assert!(third.generation > first.generation);
    }

    #[test]
    fn editor_mutation_debug_never_contains_file_bodies() {
        let marker = "SENSITIVE-EDITOR-BODY-MARKER";
        let request = PlanningEditorMutationRequest {
            identity: mutation_identity(
                PlanningEditorMutationAction::Save,
                PlanningEditorMutationTarget::Planning,
                "draft-a",
                session(1, "/workspace", "draft-a"),
                0,
            ),
            editable_files: vec![PlanningEditorFileSnapshot {
                active_path: "active.md".to_string(),
                staged_path: "staged.md".to_string(),
                body: marker.to_string(),
            }],
        };
        let rendered = format!("{request:?}");
        assert!(!rendered.contains(marker));
        assert!(rendered.contains("editable_file_count"));
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
            source_planning_revision: Some(7),
        };

        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("secret operator payload"));
        assert!(debug.contains("editable_file_count"));
        assert!(debug.contains("source_planning_revision"));
    }

    #[test]
    fn editor_mutation_revision_guard_is_opt_in() {
        let identity = mutation_identity(
            PlanningEditorMutationAction::Promote,
            PlanningEditorMutationTarget::Directions,
            "draft-a",
            session(1, "/workspace", "draft-a"),
            3,
        );
        assert_eq!(identity.source_planning_revision, None);
        assert_eq!(
            identity
                .with_source_planning_revision(11)
                .source_planning_revision,
            Some(11)
        );
    }
}
