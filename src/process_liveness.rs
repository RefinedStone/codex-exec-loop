//! Compatibility path for the native process-identity boundary.
pub(crate) use akra_native::process_liveness::{
    process_is_alive, process_start_identity, required_process_start_identity,
};
