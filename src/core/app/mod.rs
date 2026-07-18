/*
 * The app module owns core-facing contracts. Inbound adapters send AppCommand
 * through CoreInput, then read AppEvent/AppSnapshot without depending on TUI
 * state or terminal framework types.
 */
pub mod command;
pub mod controller;
pub mod conversation;
pub mod effect;
pub mod event;
pub mod projection;
pub mod request;
pub mod session;
pub mod snapshot;
pub mod startup;
pub mod state;
pub mod turn_steer;
pub mod turn_stream;
pub mod turn_submission;

pub use command::AppCommand;
pub use controller::{CoreController, CoreDispatchOutcome};
pub use conversation::{
    ConversationReadySnapshot, ConversationSnapshot, ConversationState,
    ConversationThreadReviewSnapshot,
};
pub use effect::CoreEffect;
pub use event::{AppEvent, CoreEffectCompletion, CoreInput, SessionRenameAcceptedSnapshot};
pub use projection::{ParallelModeProjection, PlanningParallelProjection};
pub use request::{
    ConversationLoadCorrelation, SessionCatalogLoadCorrelation, SessionRenameCorrelation,
    StartupCheckCorrelation,
};
pub use session::{SessionCatalogReadySnapshot, SessionCatalogSnapshot, SessionCatalogState};
pub use snapshot::AppSnapshot;
pub use startup::{
    StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot,
    StartupState,
};
pub use state::AppState;
pub use turn_steer::{TurnSteerAdmission, TurnSteerCorrelation};
pub use turn_stream::{
    TurnStreamEvent, TurnStreamProgressiveActivityUpdate, TurnStreamRuntimeEnvelopeRejection,
    TurnStreamSnapshot, TurnStreamState, TurnStreamTerminalSnapshot, TurnStreamUpdate,
};
pub use turn_submission::{
    CorePromptOrigin, TurnSubmissionAdmission, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
