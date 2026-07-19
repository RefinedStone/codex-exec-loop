use super::{
    ConversationLoadCorrelation, ParallelPeekLoadCorrelation, SessionCatalogLoadCorrelation,
    SessionRenameCorrelation, StartupCheckCorrelation, TurnSteerCorrelation,
    TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::domain::conversation::ConversationTurnSteerRequest;
use crate::domain::planning::{ManualPromptRequest, PostTurnRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    RunStartupChecks {
        correlation: StartupCheckCorrelation,
    },
    LoadSessionCatalog {
        correlation: SessionCatalogLoadCorrelation,
        limit: usize,
        workspace_directory: String,
    },
    RenameSession {
        correlation: SessionRenameCorrelation,
    },
    LoadConversation {
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    },
    LoadParallelPeekConversation {
        correlation: ParallelPeekLoadCorrelation,
    },
    PrepareManualPrompt(Box<ManualPromptRequest>),
    SubmitTurn {
        correlation: TurnSubmissionCorrelation,
        request: TurnSubmissionRequest,
    },
    SteerTurn {
        correlation: TurnSteerCorrelation,
        request: ConversationTurnSteerRequest,
    },
    EvaluatePostTurn(Box<PostTurnRequest>),
}
