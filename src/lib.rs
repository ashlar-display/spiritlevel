//! TMDS link bring-up for HDMI sources.
//!
//! `spiritlevel` decides a TMDS link's bit clock ratio and scrambling from the TMDS
//! character rate, the sink's and source's facts and the caller's policy, and states what
//! the sink's `TMDS_Config` must hold ([`TmdsLinkParams::choose`], a pure, I/O-free
//! function). Its procedures carry that out over the PHY and the sink's SCDC: bring-up,
//! tear-down, the reconnect check after a hotplug, and the quick restore.
//!
//! The procedures are written once as an I/O-free core over [`procedure::TmdsIo`], and
//! [`TmdsLink`] drives them synchronously over a [`TmdsScdc`], hdmi-hal's `TmdsPhy`, a
//! `Clock` and a `Delay`, with an optional [`Cancel`] flag. Each makes one attempt per
//! call, leaving retries to the caller, and records every step as a [`TmdsEvent`] in its
//! `_with_events` form.
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
    BringUpConfig, ClearFrl, ConfigError, OnTmdsConfigFailure, OutputDuringSinkWrite, ScramblerOff,
    SinkClearTiming, SinkPresence, SourceVersionWrite, TearDownConfig, TmdsConfigWrite,
};
pub use error::{ParamsError, UpdateError};
pub use event::{
    CancelPoint, FrlClearSkip, MAX_BRING_UP_EVENTS, MAX_QUICK_RESTORE_EVENTS,
    MAX_RECONNECT_CHECK_EVENTS, MAX_TEAR_DOWN_EVENTS, SinkClearSkip, SourceVersionSkip,
    TmdsConfigSkip, TmdsEvent, TmdsEventKind, TmdsStep,
};
pub use link::{Cancel, NoCancel, TmdsLink};
pub use outcome::{
    BringUpError, BringUpOutcome, BringUpWarning, CheckResult, Cleanup, FailedRead, LinkUp,
    OutputState, ReconnectCheck, RestoreError, Restored, ScramblingCheck, SinkClear, TearDownError,
    TornDown, Warnings, WriteStage,
};
pub use params::{ScramblingDecision, ScramblingOff, TmdsLinkParams};
pub use scdc::TmdsScdc;

#[cfg(feature = "culvert")]
pub use crate::culvert::CulvertError;
pub use types::{
    ClockRatio, ScramblingStatus, ScramblingThreshold, SinkTmdsConfig, SinkTmdsFacts,
    SourceTmdsFacts, TmdsConfigGate, TmdsConfigReading, TmdsPolicy,
};
