//! The parity suite: one test per TS test case, at the path that
//! `manifest.tsv` names for it.
#![allow(
    clippy::module_inception,
    reason = "test modules mirror TS describe blocks, which repeat the file name"
)]

mod completeness;
mod constants;
mod context_parameters_edge_cases;
mod golden;
mod handle_conflict_report;
mod handle_injection;
mod handles;
mod ksuid;
mod posthog_uuidv7;
mod support;
mod tokenestimate;
mod validation;
mod vectors;
