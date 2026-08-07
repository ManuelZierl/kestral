//! The five kernel services — the entire trusted computing base.
//!
//! Registry & Identity, Permission Broker & Trusted Chrome, Run Ledger,
//! Surface Manager, Message Router & Lease Manager. Nothing else is kernel.
//! (`artifacts` is the ledger's durable-object substrate, not a sixth
//! service.)

pub mod artifacts;
pub mod broker;
pub mod chrome;
pub mod ledger;
pub mod registry;
pub mod router;
pub mod surfaces;
