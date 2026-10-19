//! The parity suite: one test per TS test case, at the path that
//! `manifest.tsv` names for it.
#![allow(
    clippy::module_inception,
    reason = "test modules mirror TS describe blocks, which repeat the file name"
)]

mod completeness;
mod constants;
mod golden;
mod ksuid;
mod posthog_uuidv7;
mod support;
