# Architecture

> **Skeleton.** Each section states what it will settle; none is filled in yet. Claims about
> HDMI behaviour will cite at least two independent public implementations when written.

## Role

*To settle:* what `spiritlevel` is (TMDS bring-up for an HDMI source: configure the sink
over SCDC, start TMDS output, verify scrambling, tear down), how it relates to plumbob
(TMDS beside FRL; plumbob scopes TMDS out), and that it is usable on its own (a driver or
firmware can take it without the link orchestrator above it).

---

## Scope

*To settle:* what is in (bring-up above 340 Mcsc; optional scrambling at or below 340 Mcsc;
tear-down; checking `TMDS_Config` against the link and a quick restore) and what is out
(EDID, negotiation, InfoFrames, FRL, HDCP, retry policy — one attempt and its check per
call; how often to retry belongs to the caller).

---

## Dependencies

*To settle:* per release. 0.1: display-types at most, no hdmi-hal. 0.2: hdmi-hal (and
hdmi-hal-async for `spiritlevel-async`) for `TmdsPhy`, the power trait, clock and delay;
culvert behind a `culvert` feature. Why dependencies only point down the stack.

---

## Bring-up Procedure

*To settle:* the steps, in order, with the timing each one has and where it comes from.

### Choosing the link parameters

*To settle:* from the TMDS character rate to the clock ratio (1/10 or 1/40) and the
scrambling decision; the 340 Mcsc boundary; scrambling at or below it when the source can
and the sink allows it, configurable, on by default. Pure functions — part of 0.1.

### Above 340 Mcsc

*To settle:* write `TMDS_Config`, then start scrambled TMDS output within the deadline
(100 ms), then check `Scrambling_Status`; what counts as success or failure of the check;
the order relative to the PHY (sink configured before scrambled TMDS starts).

### At or below 340 Mcsc

*To settle:* the plain path and the optional scrambled path; what differs from the path
above.

### Tear-down

*To settle:* clearing the clock ratio and scrambling on the sink and in the source; the
order with stopping TMDS output; what happens if the sink is already gone.

### Reconnect check and quick restore

*To settle:* reading `TMDS_Config` back against the link the source runs (the sink may lose
it, e.g. after a long HPD low); the quick restore (a rewrite and a status check) and when a
full bring-up is needed instead.

---

## Key Types

### Owned protocol types

*To settle:* the I/O-free core's inputs and outputs (link parameters, the scrambling choice,
the `TMDS_Config` contents, timing rules); `#[non_exhaustive]` where consumers see them.

### The SCDC trait

*To settle:* the small SCDC trait `spiritlevel` defines for itself (write `TMDS_Config`,
read `TMDS_Config`, read `Scrambling_Status`); its error bound; why it is the crate's own
and not culvert's client.

### Procedure types

*To settle:* outcomes, errors and configuration of the 0.2 procedures.

---

## Diagnostics

*To settle:* the procedure's events (each step, its timestamp from the clock, the
`Scrambling_Status` read), how the caller receives them, and how they fit the stack's
structured diagnostics (no logging).

---

## Interface Boundaries

### Below: the SCDC trait and the culvert adapter

*To settle:* the adapter over culvert behind the `culvert` feature, living in this crate
so a change to `spiritlevel`'s trait never forces a culvert release; the additive
`read_tmds_config` it needs from culvert.

### Below: the PHY, power, clock and delay

*To settle:* see "What spiritlevel needs from hdmi-hal".

### Above: the link orchestrator (planned)

*To settle:* what the orchestrator calls and when (bring-up, tear-down, the reconnect check
after a hotplug), and that it decides retries.

---

## What spiritlevel needs from hdmi-hal

*To settle:* this section defines the traits for hdmi-hal's next breaking release, shaped
by this crate as their main caller.

### `TmdsPhy`

*To settle:* configure TMDS (character rate, clock ratio), start and stop TMDS output, the
source's TMDS scrambler (`set_scrambling`, moved from today's `HdmiPhy`), the capabilities
report; its errors; the cancellation clause for the async futures.

### The power trait

*To settle:* name; power off and on, used on unplug and before bring-up; on a PHY that
implements both `TmdsPhy` and `FrlPhy`, entering one mode leaves the other.

### Clock and delay

*To settle:* a non-`async` `now()` returning the stack's microsecond timestamp; the delay
as a trait of the stack's own rather than a bound on embedded-hal's `DelayNs` (so async
futures can be `Send`); the async delay and its `Send` twin in hdmi-hal-async; how an
implementation wraps a concrete `DelayNs`.

---

## `no_std`, `alloc`, and `async`

*To settle:* `no_std` without `alloc` throughout; the I/O-free `async fn` core with a sync
driver in this crate and `spiritlevel-async` driving the same core over async I/O, as
plumbob does; the compile-time `Send` test.

---

## Testing

*To settle:* unit tests of the core; the procedures against a register-level simulated sink
(planned crate) with a simulated clock; conformance checks implementors of the new
hdmi-hal traits can run.

---

## Releases

*To settle:* 0.1 — the I/O-free core, no hdmi-hal dependency; 0.2 — the procedures over
`TmdsPhy` after hdmi-hal's breaking release, with `spiritlevel-async` and the `culvert`
feature; what each release's docs promise.

---

## Design Principles

*To settle:* how the stack's principles apply here (hardware via traits, diagnostics first,
no policy in the hardware layers — the scrambling default is configurable, `no unsafe`,
stable output structs).

---

## Decisions

*To settle:* numbered decisions as they are made, each dated, with rejected alternatives.
