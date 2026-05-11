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
pub mod session;
pub mod snapshot;
pub mod startup;
pub mod state;
pub mod turn_submission;

pub use command::AppCommand;
pub use controller::{CoreController, CoreDispatchOutcome};
pub use conversation::{ConversationReadySnapshot, ConversationSnapshot, ConversationState};
pub use effect::CoreEffect;
pub use event::{AppEvent, CoreEffectCompletion, CoreInput};
pub use session::{SessionCatalogReadySnapshot, SessionCatalogSnapshot, SessionCatalogState};
pub use snapshot::AppSnapshot;
pub use startup::{
    StartupAttachmentSnapshot, StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot,
    StartupState,
};
pub use state::AppState;
pub use turn_submission::{CorePromptOrigin, TurnSubmissionRequest};
