# Roadmap

What each release contains is in the [changelog](../CHANGELOG.md); this page is about
what comes next. Today `spiritlevel` is the decision a TMDS bring-up starts from
(`TmdsLinkParams::choose`) and the procedures that carry it out: bring-up, tear-down, the
reconnect check and the quick restore, over hdmi-hal's TMDS PHY, clock and delay traits.

## Planned

### Conformance checks for `TmdsScdc`

Checks that an implementation of `TmdsScdc` runs in its own tests against
[maquette](https://github.com/ashlar-display/maquette)'s register-level simulated sink:
each method's reads and writes, `clear_flt_update` leaving the other `Update_0` flags set
([architecture](architecture.md), "The SCDC trait"), and errors passing their kind through. The procedures themselves are already tested
against that sink.

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

`choose` refuses a rate above the source's own maximum, from its PHY's capabilities, but
not one above the TMDS maximum; the negotiation layer checks the sink's, source's and
cable's ceilings before a link is chosen. A check here would become a rule once the
maximum is cited from public implementations.

### Writing `TMDS_Config` to a sink without SCDC

One source, Amlogic, writes `TMDS_Config` even to a sink that declares no SCDC; every other
source examined gates the write on SCDC, and `spiritlevel` follows them. A setting for the
other behaviour would be added only if a real sink needs it.
