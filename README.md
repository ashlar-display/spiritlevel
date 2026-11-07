# spiritlevel

[![CI](https://github.com/ashlar-display/spiritlevel/actions/workflows/ci.yml/badge.svg)](https://github.com/ashlar-display/spiritlevel/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/spiritlevel.svg)](https://crates.io/crates/spiritlevel)
[![docs.rs](https://docs.rs/spiritlevel/badge.svg)](https://docs.rs/spiritlevel)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)

> **Part of [Ashlar](https://ashlar-display.org)** — a display connection stack in Rust,
> HDMI 2.1 first: from EDID parsing through mode negotiation to FRL link training.
>
> [display-types](https://github.com/ashlar-display/display-types) ·
> [piaf](https://github.com/ashlar-display/piaf) ·
> [concordance](https://github.com/ashlar-display/concordance) ·
> [cartouche](https://github.com/ashlar-display/cartouche) ·
> [culvert](https://github.com/ashlar-display/culvert) ·
> [plumbob](https://github.com/ashlar-display/plumbob) ·
> **spiritlevel** ·
> [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) ·
> [hdmi-hal-i2c-dev](https://github.com/ashlar-display/hdmi-hal-i2c-dev)

TMDS link bring-up for HDMI sources.

`spiritlevel` decides a TMDS link's bit clock ratio (1/10 or 1/40) and scrambling from the
TMDS character rate, what the sink and the source can do, and the caller's policy, and
states what the sink's `TMDS_Config` must hold. Where implementations of HDMI sources
differ, the behaviour is a setting with a documented default, and every reason the link is
or is not scrambled is reported.

## Status

Release 0.1 is the link parameters alone: a pure, I/O-free, `const` function with no
hardware dependency. The procedures — bring-up, tear-down and the reconnect check over the
PHY and SCDC — follow in 0.2, together with the async companion `spiritlevel-async`.

## Usage

*To come with the 0.1 implementation.*

## Features

- **`display-types`** (default) — `From` conversions from display-types' `HdmiForumSinkCap`
  into the sink facts. Without it the crate has no dependencies.

## `no_std` builds

`spiritlevel` is `#![no_std]` without `alloc` in every build, and `#![forbid(unsafe_code)]`.

## Stack position

`spiritlevel` is the TMDS counterpart of [plumbob](https://github.com/ashlar-display/plumbob),
which trains FRL links: beside it in the stack, not on top of it. A link orchestrator
chooses between them.

## Out of scope

EDID reading and parsing, mode negotiation, FRL, InfoFrames and other data island packets,
HDCP, hotplug detection, PHY electrical settings, and retry schedules. See
[`doc/architecture.md`](doc/architecture.md).

## Documentation

- [`doc/architecture.md`](doc/architecture.md) — design, rules and their sources.

## License

[MPL-2.0](LICENSE)
