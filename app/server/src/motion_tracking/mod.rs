//! Linked ShellX Motion tracking/stabilization connector.
//!
//! Contract parsing, CLI execution, and Cut project orchestration stay split so
//! the central Motion bridge and dispatch router remain composition-only.

mod command;
#[cfg(test)]
mod command_tests;
mod contract;
mod handlers;
mod inventory;
mod link;

pub(crate) use handlers::{apply, detach, inspect, inventory, request, verify};
