//! Behavioral suite for the kernel, organized by service and ending with
//! the architecture acceptance criteria. Everything runs against the public
//! `Kernel` API — the same surface the shell and every app use.

mod action_path;
mod broker;
mod data_scopes;
mod durable_state;
mod helpers;
mod leases_and_router;
mod ledger;
mod manifest_and_registry;
mod phased_execution;
mod success_criteria;
mod upgrades;
