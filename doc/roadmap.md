# Roadmap

What each release contains is in the [changelog](../CHANGELOG.md); this page is about
what comes next. Today `spiritlevel` is the decision a TMDS bring-up starts from
(`TmdsLinkParams::choose`) and the procedures that carry it out: bring-up, tear-down, the
reconnect check and the quick restore, over hdmi-hal's TMDS PHY, clock and delay traits.
They are released after the hdmi-hal release that adds those traits.

## Planned

### `spiritlevel-async`

An async companion crate, as plumbob has plumbob-async: an async driver over
hdmi-hal-async's traits for the procedures' I/O-free core (`spiritlevel::procedure`), which
the sync driver `TmdsLink` already runs, with futures that can be `Send`.

### Testing the procedures

Today the procedures are tested against a double of their I/O. Next they will be tested
against a register-level simulated HDMI sink with a simulated clock, including sinks that
refuse, delay or lose their configuration, and the crate will offer conformance checks that
implementors of the new hdmi-hal traits can run.

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
