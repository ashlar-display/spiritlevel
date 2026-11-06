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

The first step decides, without any I/O, what the link will be: the TMDS bit clock ratio,
whether the link is scrambled, and what (if anything) is written to the sink's
`TMDS_Config`. It is a pure function and the whole of the 0.1 release.

**Inputs**

| Input               | Type (proposed)                                        | Where the caller gets it                                                                                                                            |
|---------------------|--------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------|
| TMDS character rate | `u32`, kHz                                             | the negotiated configuration (pixel clock × deep-colour factor, halved for YCbCr 4:2:0) — the same unit the stack's negotiation layer uses          |
| Sink facts          | `SinkTmdsFacts { scdc_present, lte_340mcsc_scramble }` | the sink's HDMI Forum capabilities (`From<&HdmiForumSinkCap>`; no HF-SCDB means neither); the caller may clear a fact for a sink known to misbehave |
| Source fact         | `SourceTmdsFacts { low_rate_scrambling }`              | the PHY's capabilities (from 0.2 the `TmdsPhy` capabilities report; in 0.1 the caller states it)                                                    |
| Policy              | `TmdsPolicy { low_rate_scrambling, threshold }`        | the caller; defaults below                                                                                                                          |

The rate is an input, not computed here: the character rate depends on the negotiated
colour format, depth and pixel repetition, which is negotiation's knowledge, not bring-up's.

**Rules**

1. **Above the threshold**, the clock ratio is 1/40 and the link is scrambled; the sink's
   `TMDS_Config` gets both `TMDS_Bit_Clock_Ratio` and `Scrambling_Enable`. Settled: every
   source examined does this — the Linux DRM helper and vc4 (one lineage), i915, dw-hdmi,
   amdgpu, nouveau, Tegra, MediaTek v2, Xilinx and Amlogic [1], [3]–[10].
2. **At or below the threshold**, the clock ratio is 1/10, and a source with an SCDC sink
   clears the ratio in `TMDS_Config` [1], [3]–[10].
3. **Scrambling at or below the threshold** is on when all three allow it: the source can
   (`low_rate_scrambling` fact), the sink declares `LTE_340Mcsc_scramble`, and the policy
   permits it. Settled for sources that can scramble at low rates: i915, amdgpu, nouveau
   and dw-hdmi enable it whenever the sink declares it [3][4][5][6]; Xilinx leaves it to
   the application, off by default [9]. The helper lineage, Tegra, MediaTek v2 and Amlogic
   never scramble at low rates [1][2][7][8][10]; sources whose hardware cannot scramble at low
   rates are a capability, not dissent. Hence configurable, **default on**.
4. **The threshold** is "above 340 Mcsc" by default; "at or above 340 Mcsc" is the
   alternative. The sources differ: above in the helper lineage, i915, dw-hdmi, Xilinx and
   Amlogic [1][3][4][9][10]; at or above in Tegra, MediaTek v2 and amdgpu's source
   scrambler [7][8][5]. A majority with dissent, so configurable with the majority as
   default. The two differ only at exactly 340 000 kHz.
5. **Without SCDC** the sink cannot be configured: the result has no `TMDS_Config` write,
   ratio 1/10 and no scrambling. A rate that needs scrambling (rule 1) on such a sink is
   refused with a typed error rather than run unscrambled.

**Output**

`TmdsLinkParams`: the clock ratio, whether the link is scrambled and why, and the
`TMDS_Config` value for the sink (`None` for a sink without SCDC). The PHY takes the ratio
and the scrambling flag (0.2, `TmdsPhy`); the SCDC trait takes the sink's value. The core
states *what* the register holds, not its bit layout: the encoding belongs to the SCDC
implementation (culvert's `TmdsConfig` already has it). Error: `ScramblingRequiredWithoutScdc`
(rule 5). Types under "Key Types".

**Open points for this step**

- Whether the core also refuses a rate above the TMDS maximum, or leaves the ceiling to
  negotiation alone (the stack's negotiation layer already checks sink, source and cable
  ceilings). Needs the maximum cited before it can be a rule here.
- Amlogic writes `TMDS_Config` even when the sink declares no SCDC [10]; every other source
  gates the write on SCDC. One dissenting source against a majority; rule 5 follows the
  majority, and whether a setting is worth having is to settle.
- Pixel repetition in the character rate: who computes it, given the core takes the rate
  as input (a display-types helper, or the negotiation layer).

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

These are the I/O-free core's vocabulary: the inputs and output of "Choosing the link
parameters" and the contents of `TMDS_Config`. They are the whole public API of 0.1. Types
for the procedures' timing come with the procedure sections.

```rust
/// The TMDS bit clock ratio: the TMDS clock runs at the bit rate / 10 or / 40.
/// Exhaustive: `TMDS_Bit_Clock_Ratio` is one bit.
pub enum ClockRatio { Div10, Div40 }

/// Which character rate first requires scrambling and 1/40 (rule 4).
#[non_exhaustive]
pub enum ScramblingThreshold {
    Above340,       // default: > 340 000 kHz
    AtOrAbove340,   // ≥ 340 000 kHz
}

/// What the sink declares (`new`; `NONE` for a sink without HDMI Forum capabilities,
/// e.g. a DVI or HDMI 1.x sink). `From<&HdmiForumSinkCap>` and
/// `From<Option<&HdmiForumSinkCap>>`. `lte_340mcsc_scramble` counts only with
/// `scdc_present` (rule 5).
#[non_exhaustive]
pub struct SinkTmdsFacts { pub scdc_present: bool, pub lte_340mcsc_scramble: bool }

/// What the source's PHY can do (`new`). From 0.2 built from the `TmdsPhy` capabilities.
#[non_exhaustive]
pub struct SourceTmdsFacts { pub low_rate_scrambling: bool }

/// The caller's choices (rules 3 and 4). `Default`/`DEFAULT`: low-rate scrambling on,
/// `Above340`; `with_low_rate_scrambling`, `with_threshold`.
#[non_exhaustive]
pub struct TmdsPolicy { pub low_rate_scrambling: bool, pub threshold: ScramblingThreshold }

/// The two fields of `TMDS_Config` (0x20), what is written and what a read returns
/// (`new`). Its bit layout belongs to the SCDC implementation.
#[non_exhaustive]
pub struct SinkTmdsConfig { pub scrambling_enable: bool, pub high_clock_ratio: bool }

/// Why the link is or is not scrambled.
#[non_exhaustive]
pub enum ScramblingDecision {
    Required,             // above the threshold (rule 1)
    LowRate,              // at or below; source, sink and policy all allow it (rule 3)
    Off(ScramblingOff),   // at or below, not scrambled
}

#[non_exhaustive]
pub enum ScramblingOff { PolicyOff, SourceCannot, SinkDoesNotDeclare, NoScdc }

/// The chosen link. Fields are private, so the sink's value always matches the link:
/// `clock_ratio()`, `scrambling()` (true for `Required` and `LowRate`),
/// `scrambling_decision()`, `sink_config() -> Option<SinkTmdsConfig>` (`None` without
/// SCDC). Built only by `choose`.
#[non_exhaustive]
pub struct TmdsLinkParams { /* private */ }

impl TmdsLinkParams {
    pub const fn choose(
        rate_khz: u32,
        sink: SinkTmdsFacts,
        source: SourceTmdsFacts,
        policy: TmdsPolicy,
    ) -> Result<Self, ParamsError>;
}

/// `Display` and `core::error::Error`.
#[non_exhaustive]
pub enum ParamsError {
    ScramblingRequiredWithoutScdc { rate_khz: u32 },   // rule 5
}
```

All are `Debug, Clone, Copy, PartialEq, Eq` (and `Hash` where it costs nothing); all
constructors are `const fn`, so a firmware can fix its policy and facts at compile time.

- **Why private fields on `TmdsLinkParams`:** the PHY and the sink must agree. With public
  fields a caller could change the ratio and leave `sink_config` stale; with getters the
  only way to a `TmdsLinkParams` is through the rules.
- **Why `ScramblingDecision`:** an unscrambled low-rate link is not a failure, but the
  stack's diagnostics say why a choice was made; a caller expecting scrambling sees which
  of source, sink or policy declined it.
- **Why the sink facts are a struct of their own,** not `HdmiForumSinkCap` directly: the
  core needs two flags, a caller can override a misbehaving sink without editing parsed
  EDID data, and 0.1 does not tie its API to display-types' struct beyond the `From`
  impls.

**Open point:** optional `serde` derives behind a feature, as display-types and
concordance have, once `serde` without `std` is settled for the stack.

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

---

## Sources

Driver source read at these commits; line numbers refer to them.

- Linux, github.com/torvalds/linux `e9954f3fbddd` (2026-10-31)
- Xilinx embeddedsw, github.com/Xilinx/embeddedsw `97f2baf7f69e` (2026-06-24)
- Amlogic, github.com/CoreELEC/common_drivers `8ca9b6c3b715` (2026-10-31)

|      | Source                              | Location                                                                                                                                    |
|------|-------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------|
| [1]  | Linux DRM helper, vc4 (one lineage) | `drivers/gpu/drm/display/drm_hdmi_helper.c:476–482`; `drivers/gpu/drm/vc4/vc4_hdmi.c:134–141`, `815–816`                                    |
| [2]  | Linux DRM state helper              | `drivers/gpu/drm/display/drm_hdmi_state_helper.c:599` (low-rate scrambling a TODO)                                                          |
| [3]  | i915                                | `drivers/gpu/drm/i915/display/intel_hdmi.c:2433–2435`, `2793–2795`                                                                          |
| [4]  | dw-hdmi                             | `drivers/gpu/drm/bridge/synopsys/dw-hdmi.c:1400–1402`, `1424–1431`, `2101–2102`, `2118`                                                     |
| [5]  | amdgpu                              | `drivers/gpu/drm/amd/display/dc/link/protocols/link_ddc.c:571–613`; `dc/core/dc_stream.c:158`; `dc/dce/dce_stream_encoder.c:621`, `629–640` |
| [6]  | nouveau                             | `drivers/gpu/drm/nouveau/dispnv50/disp.c:800–815`                                                                                           |
| [7]  | Tegra                               | `drivers/gpu/drm/tegra/sor.c:2175–2181`, `2198`                                                                                             |
| [8]  | MediaTek v2                         | `drivers/gpu/drm/mediatek/mtk_hdmi_v2.c:86–100`, `746`                                                                                      |
| [9]  | Xilinx                              | `XilinxProcessorIPLib/drivers/v_hdmitx1/src/xv_hdmitx1.c:630–635`, `742–744`; `v_hdmitxss1/src/xv_hdmitxss1.c:2938–2957`                    |
| [10] | Amlogic                             | `drivers/media/vout/hdmitx21/hw/hdmi_tx_hw.c:668–676`, `2780–2799`, `2889–2906`                                                             |
