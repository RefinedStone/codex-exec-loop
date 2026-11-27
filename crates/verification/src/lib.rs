//! Task acceptance, evidence, and host verification without repository or process adapters.
//! Execution facts are not authority to deliver or finish a task. Root orchestration retains
//! lease, issue binding, reservation, and continuation serialization checks.
#![forbid(unsafe_code)]

pub mod acceptance;
pub mod continuation;
pub mod delivery_evidence;
pub mod evidence;
pub mod ports;
pub mod service;
pub mod verification;
