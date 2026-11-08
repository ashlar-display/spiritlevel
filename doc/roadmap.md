# Roadmap

What each release contains is in the [changelog](../CHANGELOG.md); this page is about
what comes next. Today `spiritlevel` is the decision a TMDS bring-up starts from
(`TmdsLinkParams::choose`); next come the procedures that carry it out.

## Planned

### The procedures

The next step adds the I/O: the procedures that take a `TmdsLinkParams` to a running link, each
making one attempt and its check per call, with retries left to the caller.

- **Bring-up.** Above 340 Mcsc: write the sink's `TMDS_Config`, start scrambled TMDS output
  within the sink's deadline, and check `Scrambling_Status`. At or below 340 Mcsc: the plain
  link, or the scrambled one when the decision allows it. The sink's FRL settings are
  cleared first when the link may have been in FRL, and `Source_Version` is written on an
  SCDC sink when the policy says so.
- **Tear-down.** Clearing the clock ratio and scrambling on the sink and in the source, and
  stopping TMDS output, in the order that keeps the two in step.
- **The reconnect check.** After a hotplug, reading `TMDS_Config` back against the link the
  source runs (a sink may lose it, for example after a long HPD low), and a quick restore
  (a rewrite and a status check) when the caller chooses it over a full bring-up.
- **Diagnostics.** Each step reported as a structured event with its timestamp, so a caller
  can see what happened and when, without logging.

The order of the steps, their timing and where each comes from will be cited from public
implementations of HDMI sources, as the rules for choosing the link are.

### The interfaces the procedures need

`spiritlevel` defines what it needs rather than depending on particular implementations,
so a driver, a firmware image or a vendor SDK can supply its own:

- **From [hdmi-hal](https://github.com/ashlar-display/hdmi-hal)'s next breaking release:**
  a TMDS PHY trait (configure the character rate and clock ratio, start and stop TMDS
  output, the source's scrambler, a capabilities report from which `SourceTmdsFacts` is
  built), a PHY power trait, and clock and delay traits. These traits are being shaped with
  `spiritlevel` as their main caller, so the procedures follow that hdmi-hal release.
- **A small SCDC trait of the crate's own:** write and read `TMDS_Config`, read
  `Scrambling_Status`. A `culvert` feature adds its implementation over
  [culvert](https://github.com/ashlar-display/culvert)'s `Scdc`, kept in this crate so a
  change to the trait never forces a culvert release; it uses the `TMDS_Config` read on
  culvert's roadmap.

### `spiritlevel-async`

An async companion crate, as plumbob has plumbob-async: the procedures as an I/O-free core
shared by a sync driver here and an async driver there, over hdmi-hal-async's traits, with
futures that can be `Send`.

### Testing the procedures

The procedures will be tested against a register-level simulated HDMI sink with a simulated
clock, including sinks that refuse, delay or lose their configuration, and the crate will
offer conformance checks that implementors of the new hdmi-hal traits can run.

### Changes to today's types

The current types may change where the procedures need it, for example `SourceTmdsFacts`
built from the PHY's capabilities report. Such a change is a breaking release, and the
changelog will list it.

### The link orchestrator

A link orchestrator planned for the stack will choose between FRL (plumbob) and TMDS
(`spiritlevel`) and call the procedures: bring-up and tear-down, and the reconnect check
after a hotplug, deciding retries itself. It will drive `spiritlevel` through an interface
of its own; nothing in `spiritlevel` changes for it.

## Under consideration

### `serde` derives

Optional `Serialize` and `Deserialize` on the public types behind a feature, as
display-types and concordance have, so a diagnostic tool can record decisions. Waits until
`serde` without `std` is settled for the stack.

### Checking the rate against the TMDS maximum

`choose` does not refuse a character rate above the TMDS maximum; the negotiation layer
checks the sink's, source's and cable's ceilings before a link is chosen. A check here
would become a rule once the maximum is cited from public implementations.

### Writing `TMDS_Config` to a sink without SCDC

One source, Amlogic, writes `TMDS_Config` even to a sink that declares no SCDC; every other
source examined gates the write on SCDC, and `spiritlevel` follows them. A setting for the
other behaviour would be added only if a real sink needs it.
