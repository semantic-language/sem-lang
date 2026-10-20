//! Provider-neutral adopted semantic execution contracts.
//!
//! Physical process loading belongs to the capability owner.  This crate only
//! validates exact semantic inputs and outputs at that boundary.
pub mod semantic;
pub use semantic::*;
