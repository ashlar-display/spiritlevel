# spiritlevel

[![CI](https://github.com/ashlar-display/spiritlevel/actions/workflows/ci.yml/badge.svg)](https://github.com/ashlar-display/spiritlevel/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/spiritlevel.svg)](https://crates.io/crates/spiritlevel)
[![docs.rs](https://docs.rs/spiritlevel/badge.svg)](https://docs.rs/spiritlevel)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
[![SLSA Level 2](https://slsa.dev/images/gh-badge-level2.svg)](https://slsa.dev)
[![trusted publishing only](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fcrates.io%2Fapi%2Fv1%2Fcrates%2Fspiritlevel&query=%24.crate.trustpub_only&label=trusted%20publishing%20only)](#verifying-releases)

> **Part of [Ashlar](https://ashlar-display.org)** — a display connection stack in Rust,
> HDMI 2.1 first: from EDID parsing through mode negotiation to FRL link training.
> See [all crates](https://ashlar-display.org/crates.html).

TMDS link bring-up for HDMI sources.

`spiritlevel` decides a TMDS link's bit clock ratio (1/10 or 1/40) and scrambling from the
TMDS character rate, what the sink and the source can do, and the caller's policy, and
states what the sink's `TMDS_Config` must hold. Its procedures then carry that out over the
PHY and the sink's SCDC: bring-up, tear-down, the reconnect check after a hotplug, and the
quick restore. Where implementations of HDMI sources differ, the behaviour is a setting
with a documented default, every reason the link is or is not scrambled is reported, and
every step of a procedure is recorded as a structured event.

## Status

The link parameters are a pure, I/O-free, `const` function. The procedures are written
once as an I/O-free core (`spiritlevel::procedure`) and driven synchronously by `TmdsLink`,
over [hdmi-hal](https://github.com/ashlar-display/hdmi-hal)'s TMDS PHY, clock and delay
traits and the crate's own `TmdsScdc`. Each makes one attempt per call; retries are the
caller's. The async companion `spiritlevel-async` is planned; see the
[roadmap](doc/roadmap.md).

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

Then bring the link up over the parts: the sink's SCDC (`TmdsScdc`, implemented for
culvert's client by the `culvert` feature), a `TmdsPhy`, a `Clock` and a `Delay`:

```rust
use hdmi_hal::phy::{TmdsPhy, TmdsVideo};
use hdmi_hal::time::{Clock, Delay};
use hdmi_hal::{ColorBitDepth, ColorFormat};
use spiritlevel::{
    BringUpConfig, BringUpOutcome, CheckResult, SinkPresence, SinkTmdsFacts,
    SourceTmdsFacts, TearDownConfig, TmdsLink, TmdsLinkParams, TmdsPolicy, TmdsScdc,
};

/// Brings up 4K60 RGB over TMDS, then stops it again.
fn show<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay>(
    scdc: S,
    phy: P,
    clock: K,
    delay: D,
    sink: SinkTmdsFacts,
) {
    let source = SourceTmdsFacts::from(&phy.capabilities());
    let Ok(link) = TmdsLinkParams::choose(594_000, sink, source, TmdsPolicy::DEFAULT) else {
        return; // a rate this sink and source cannot carry
    };
    let video = TmdsVideo::new(594_000, ColorBitDepth::Depth8, ColorFormat::Rgb444);

    let mut tmds = TmdsLink::new(scdc, phy, clock, delay);
    match tmds.bring_up(&link, video, &BringUpConfig::DEFAULT) {
        Ok(BringUpOutcome::Up(up)) => {
            // Whether the sink reports scrambled TMDS, and what went wrong without
            // stopping the bring-up.
            let _seen = up.check.result == CheckResult::Set;
            for _warning in up.warnings.iter() {}
        }
        Ok(_cancelled) => {} // the cancel flag (`with_cancel`); never without one
        Err(_error) => {}    // the step that failed, and the tear-down that followed
    }
    let _ = tmds.tear_down(&link, &TearDownConfig::DEFAULT, SinkPresence::Present);
}
```

Each procedure has a `_with_events` form that calls a closure with every step, timestamped
from the caller's clock; `MAX_BRING_UP_EVENTS` and its siblings bound how many, so a caller
without `alloc` sizes its buffer.

## Features

- **`display-types`** (default) — `From` conversions from display-types' `HdmiForumSinkCap`
  into the sink facts. Without it the caller fills the sink facts itself.
- **`culvert`** — `TmdsScdc` for [culvert](https://github.com/ashlar-display/culvert)'s
  SCDC client (`culvert::Scdc`), with culvert's errors wrapped in `CulvertError`.

## `no_std` builds

`spiritlevel` is `#![no_std]` without `alloc` in every build, and `#![forbid(unsafe_code)]`.

## Stack position

`spiritlevel` is the TMDS counterpart of [plumbob](https://github.com/ashlar-display/plumbob),
which trains FRL links: beside it in the stack, not on top of it. A link orchestrator
chooses between them.

Below it, [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) supplies the TMDS PHY,
clock and delay traits the procedures run over,
[display-types](https://github.com/ashlar-display/display-types) the sink's HDMI Forum
capabilities (the `display-types` feature), and, behind a feature,
[culvert](https://github.com/ashlar-display/culvert) the SCDC client.

## Out of scope

EDID reading and parsing, mode negotiation, FRL, InfoFrames and other data island packets,
HDCP, hotplug detection, PHY electrical settings, and retry schedules. See
[`doc/architecture.md`](doc/architecture.md).

## Documentation

- [`doc/architecture.md`](doc/architecture.md) — role, scope, the rules for choosing the
  link and their sources, the types, design principles and decisions
- [`doc/roadmap.md`](doc/roadmap.md) — what is planned and under consideration
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

0.1.0 was uploaded by the publish workflow with a crates.io API token scoped to publishing
this new crate, because trusted publishing can only be set up for a crate already on
crates.io; the token was revoked afterwards. Later releases are uploaded through crates.io
[trusted publishing](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html):
the publish workflow obtains a short-lived token via OpenID Connect instead of using a
stored API token. crates.io rejects uploads made with an API token for this crate; the
badge above shows that this setting is on. Each version's publishing details, including
the workflow run and commit, are available from the crates.io API:

```sh
curl -A "your-tool (contact)" https://crates.io/api/v1/crates/spiritlevel/X.Y.Z
# see .version.trustpub_data
```

## License

[MPL-2.0](LICENSE)
