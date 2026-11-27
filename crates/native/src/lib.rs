//! Host-owned OS primitives. Product configuration, provider policy and database transactions stay with consumers.
#![deny(dead_code)]

pub mod private_directory;
#[cfg(windows)]
pub mod private_fs;
pub mod process_liveness;
#[cfg(test)]
mod test_utils;
#[cfg(any(windows, test))]
mod windows_ace;

pub mod private_path;
pub mod private_temp;
pub mod stable_file;
pub mod subprocess;
pub mod trusted_executable;
