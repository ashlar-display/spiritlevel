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
/// The cancel flag, re-exported from hdmi-hal, so one flag stops TMDS bring-up and FRL
/// training alike. Implemented for `AtomicBool` and any reference to a flag.
pub use hdmi_hal::cancel::{Cancel, NoCancel};
pub use link::TmdsLink;
pub use outcome::{
    BringUpError, BringUpOutcome, BringUpWarning, CheckResult, Cleanup, FailedRead, LinkUp,
    OutputState, ReconnectCheck, RestoreError, Restored, ScramblingCheck, SinkClear, TearDownError,
    TornDown, Warnings,
};
pub use params::{ScramblingDecision, ScramblingOff, TmdsLinkParams};
pub use scdc::TmdsScdc;

#[cfg(feature = "culvert")]
pub use crate::culvert::CulvertError;
pub use types::{
    ClockRatio, ScramblingStatus, ScramblingThreshold, SinkTmdsConfig, SinkTmdsFacts,
    SourceTmdsFacts, TmdsConfigGate, TmdsConfigReading, TmdsPolicy,
};

/// The hdmi-hal types the procedures' signatures use, re-exported so that callers and
/// implementations name the same versions `spiritlevel` does without depending on hdmi-hal
/// themselves: the PHY trait and its configuration, the clock, delay and timestamp, the
/// SCDC error classification, and the colour depth and format a `TmdsVideo` carries.
pub use hdmi_hal::ddc::{DdcError, DdcErrorKind, NoAcknowledgeSource};
pub use hdmi_hal::phy::{TmdsPhy, TmdsPhyCapabilities, TmdsPhyConfig, TmdsVideo};
pub use hdmi_hal::time::{Clock, Delay, Timestamp};
pub use hdmi_hal::{ColorBitDepth, ColorFormat};

/// The sink capabilities the sink facts convert from, re-exported from display-types.
#[cfg(feature = "display-types")]
pub use display_types::cea861::HdmiForumSinkCap;

/// culvert's client, its errors and the transport it runs over, re-exported for the
/// adapter's users.
#[cfg(feature = "culvert")]
pub use ::culvert::{ProtocolError, Scdc, ScdcError, ScdcTransport};
