# spiritlevel

[![CI](https://github.com/ashlar-display/spiritlevel/actions/workflows/ci.yml/badge.svg)](https://github.com/ashlar-display/spiritlevel/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/spiritlevel.svg)](https://crates.io/crates/spiritlevel)
[![docs.rs](https://docs.rs/spiritlevel/badge.svg)](https://docs.rs/spiritlevel)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
[![SLSA Level 2](https://slsa.dev/images/gh-badge-level2.svg)](https://slsa.dev)

> **Part of [Ashlar](https://ashlar-display.org)** — a display connection stack in Rust,
> HDMI 2.1 first: from EDID parsing through mode negotiation to FRL link training.
> See [all crates](https://ashlar-display.org/crates.html).

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

```sh
cargo add spiritlevel
```

State what the sink and the source can do, and choose the link for the TMDS character
rate (in kHz, rounded up):

```rust
use spiritlevel::{
    ClockRatio, ParamsError, ScramblingDecision, SinkTmdsFacts, SourceTmdsFacts,
    TmdsLinkParams, TmdsPolicy,
};

fn main() -> Result<(), ParamsError> {
    // From the sink's HF-SCDB (feature `display-types`): `SinkTmdsFacts::from(hf_scdb)`,
    // where `None` is a sink without one. Or state the facts directly:
    let sink = SinkTmdsFacts::new(
        true,  // SCDC present
        true,  // scrambling supported (non-zero Max_TMDS_Character_Rate)
        false, // LTE_340Mcsc_scramble
    );
    let source = SourceTmdsFacts::new(true); // the PHY can scramble at low rates

    let link = TmdsLinkParams::choose(297_000, sink, source, TmdsPolicy::DEFAULT)?;

    assert_eq!(link.clock_ratio(), ClockRatio::Div10);
    // Not scrambled, and every reason is reported: here only the sink's.
    assert!(matches!(
        link.scrambling_decision(),
        ScramblingDecision::Off(off)
            if off.sink_does_not_declare && !off.source_cannot && !off.policy_off
    ));
    // What the sink's TMDS_Config must hold; `None` for a sink without SCDC.
    let config = link.sink_config().expect("this sink has SCDC");
    assert!(!config.scrambling_enable && !config.high_tmds_clock_ratio);
    Ok(())
}
```

Where HDMI sources differ, `TmdsPolicy` holds the setting: whether to scramble at or
below 340 Mcsc when both ends can (default on), and whether exactly 340 000 kHz counts as
above the threshold (default not). A rate above 340 000 kHz on a sink that cannot take
scrambling is refused with `ParamsError::ScramblingUnsupported`, naming every missing fact.

## Features

- **`display-types`** (default) — `From` conversions from display-types' `HdmiForumSinkCap`
  into the sink facts. Without it the crate has no dependencies.

## `no_std` builds

`spiritlevel` is `#![no_std]` without `alloc` in every build, and `#![forbid(unsafe_code)]`.

## Stack position

`spiritlevel` is the TMDS counterpart of [plumbob](https://github.com/ashlar-display/plumbob),
which trains FRL links: beside it in the stack, not on top of it. A link orchestrator
chooses between them.

Below it, [display-types](https://github.com/ashlar-display/display-types) supplies the
sink's HDMI Forum capabilities (the `display-types` feature). From 0.2 the procedures also
use [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) for the PHY and, behind a
feature, [culvert](https://github.com/ashlar-display/culvert) for SCDC.

## Out of scope

EDID reading and parsing, mode negotiation, FRL, InfoFrames and other data island packets,
HDCP, hotplug detection, PHY electrical settings, and retry schedules. See
[`doc/architecture.md`](doc/architecture.md).

## Documentation

- [`doc/architecture.md`](doc/architecture.md) — role, scope, the rules for choosing the
  link and their sources, the types, design principles and decisions
- [`doc/setup.md`](doc/setup.md) — build, test, MSRV and coverage commands
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — issues, pull requests and coding standards

## Verifying releases

Releases are built on GitHub Actions and attested with
[SLSA Build Level 2](https://slsa.dev) provenance. To verify a release `.crate` against
its signed provenance, install the [GitHub CLI](https://cli.github.com/) and run:

```sh
gh attestation verify spiritlevel-X.Y.Z.crate --repo ashlar-display/spiritlevel
```

The attested `.crate` is attached to each
[GitHub release](https://github.com/ashlar-display/spiritlevel/releases).

0.1.0 is uploaded by the publish workflow with a crates.io API token scoped to publishing
this new crate, because trusted publishing can only be set up for a crate already on
crates.io; the token is revoked afterwards. Later releases are uploaded through crates.io
[trusted publishing](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html):
the publish workflow obtains a short-lived token via OpenID Connect instead of using a
stored API token. Each version's publishing details, including the workflow run and
commit, are available from the crates.io API:

```sh
curl -A "your-tool (contact)" https://crates.io/api/v1/crates/spiritlevel/X.Y.Z
# see .version.trustpub_data
```

## License

[MPL-2.0](LICENSE)
