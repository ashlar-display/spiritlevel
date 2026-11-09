//! TMDS link bring-up for HDMI sources.
//!
//! `spiritlevel` decides a TMDS link's bit clock ratio and scrambling from the TMDS
//! character rate, the sink's and source's facts and the caller's policy, and states what
//! the sink's `TMDS_Config` must hold. It currently provides this decision alone: a pure,
//! I/O-free function. The bring-up, tear-down and reconnect-check procedures over the
//! PHY and SCDC are planned.
//!
//! # Features
//!
//! - **`display-types`** (default) — `From` conversions from display-types'
//!   `HdmiForumSinkCap` into the sink facts. Without it the crate has no dependencies.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod params;
mod types;

// Compiles and runs the README's examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub use error::{ParamsError, UpdateError};
pub use params::{ScramblingDecision, ScramblingOff, TmdsLinkParams};
pub use types::{
    ClockRatio, ScramblingStatus, ScramblingThreshold, SinkTmdsConfig, SinkTmdsFacts,
    SourceTmdsFacts, TmdsPolicy,
};
