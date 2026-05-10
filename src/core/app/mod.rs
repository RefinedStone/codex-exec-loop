/*
 * The app module owns core-facing contracts. Inbound adapters send AppCommand
 * through CoreInput, then read AppEvent/AppSnapshot without depending on TUI
 * state or terminal framework types.
 */
pub mod command;
pub mod controller;
pub mod event;
pub mod snapshot;
pub mod startup;
pub mod state;

pub use command::AppCommand;
pub use controller::{CoreController, CoreDispatchOutcome};
pub use event::{AppEvent, CoreEffectCompletion, CoreInput};
pub use snapshot::AppSnapshot;
pub use startup::{StartupReadySnapshot, StartupSnapshot, StartupState};
pub use state::AppState;
