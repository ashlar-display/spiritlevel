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
//!   `HdmiForumSinkCap` into the sink facts. Without it the caller fills the sink facts
//!   itself.
//! - **`culvert`** — [`TmdsScdc`] for culvert's SCDC client, with its errors as
//!   `CulvertError`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod config;
#[cfg(feature = "culvert")]
mod culvert;
mod error;
mod event;
mod link;
mod outcome;
mod params;
pub mod procedure;
mod scdc;
mod types;

// Compiles and runs the README's examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub use config::{
    BringUpConfig, ClearFrl, OnTmdsConfigFailure, OutputDuringSinkWrite, ScramblerOff,
    SinkClearTiming, SinkPresence, SourceVersionWrite, TearDownConfig, TmdsConfigWrite,
};
pub use error::{ParamsError, UpdateError};
pub use event::{
    MAX_RECONNECT_CHECK_EVENTS, MAX_TEAR_DOWN_EVENTS, SinkClearSkip, TmdsEvent, TmdsEventKind,
    TmdsStep,
};
pub use link::{Cancel, NoCancel, TmdsLink};
pub use outcome::{
    BringUpError, BringUpOutcome, BringUpWarning, CheckResult, Cleanup, FailedRead, LinkUp,
    OutputState, ReconnectCheck, ScramblingCheck, SinkClear, TearDownError, TornDown, Warnings,
    WriteStage,
};
pub use params::{ScramblingDecision, ScramblingOff, TmdsLinkParams};
pub use scdc::TmdsScdc;

#[cfg(feature = "culvert")]
pub use crate::culvert::CulvertError;
pub use types::{
    ClockRatio, ScramblingStatus, ScramblingThreshold, SinkTmdsConfig, SinkTmdsFacts,
    SourceTmdsFacts, TmdsPolicy,
};
