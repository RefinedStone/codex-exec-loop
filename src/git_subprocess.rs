//! Shared production Git isolation lives with the native Git verification adapter.
//! The root libtest facade retains its existing executable fixture policy and timers.

#[cfg(not(test))]
pub(crate) use akra_verification_native::git_command::{
    command, command_for_program, command_with_program, is_git_program,
};

#[cfg(test)]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::{command, command_for_program, command_with_program, is_git_program};
