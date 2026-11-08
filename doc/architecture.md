# Architecture

This document describes `spiritlevel` 0.1: what it decides, the rules it follows and where
they come from, its types, and the decisions behind them. Claims about HDMI behaviour cite
at least two independent public implementations of HDMI sources (see "Sources").

## Role

`spiritlevel` is the TMDS link bring-up crate of an HDMI source. Release 0.1 is its first
step: deciding, without any I/O, what a TMDS link will be. From the TMDS character rate,
what the sink and the source can do, and the caller's policy, it decides the link's bit
clock ratio (1/10 or 1/40), whether the link is scrambled and why, and what the sink's
`TMDS_Config` register must hold.

The decision is a pure `const fn`. A driver, a firmware image or a vendor SDK can use it on
its own: the caller programs its PHY with the ratio and the scrambling flag and writes the
sink's `TMDS_Config` itself. The procedures that do this over a PHY and SCDC are planned
for 0.2 (see "What comes next").

TMDS and FRL are separate link modes with separate procedures. FRL link training is
[plumbob](https://github.com/ashlar-display/plumbob)'s; `spiritlevel` is its counterpart for
TMDS, beside it in the stack rather than on top of it.

It implements the procedure, not policy: where implementations of HDMI sources differ, the
behaviour is a setting with a documented default.

---

## Scope

`spiritlevel` 0.1 covers:

- **Choosing the link parameters:** the clock ratio, scrambling and the sink's
  `TMDS_Config` contents, from the character rate, the sink's and source's facts, and the
  policy.
- **Explaining the choice:** every reason a link is not scrambled, and every missing fact
  when a rate is refused.

It does not cover:

- **Computing the character rate:** it depends on the negotiated colour format, depth and
  pixel repetition, which is the negotiation layer's knowledge. The caller passes it in.
- **Checking the rate against the TMDS maximum** or the sink's, source's and cable's
  ceilings: the negotiation layer checks those before a link is chosen.
- **EDID reading and parsing:** the sink facts come from the parsed HDMI Forum capabilities.
- **The I/O:** programming the PHY and writing the sink's SCDC registers (planned for 0.2).
- **FRL:** plumbob.
- **InfoFrames and other data island packets, AVMUTE, HDCP, hotplug detection, PHY
  electrical settings, retry schedules.**

---

## Dependencies and features

```toml
[features]
default = ["display-types"]

[dependencies]
display-types = { version = "0.5", default-features = false, optional = true }
```

- **`display-types`** (default): `From<&HdmiForumSinkCap>` and
  `From<Option<&HdmiForumSinkCap>>` for `SinkTmdsFacts`. display-types is then a public
  dependency (its type appears in those impls), so a breaking display-types release means a
  breaking `spiritlevel` release.
- **Without default features**, the crate has no dependencies: a firmware states the sink
  facts itself.

---

## Choosing the link

`TmdsLinkParams::choose(rate_khz, sink, source, policy)` returns the link or a
`ParamsError`.

### Inputs

| Input               | Type                                                                         | Where the caller gets it                                                                                                                                                                                                                                                                         |
|---------------------|------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| TMDS character rate | `u32`, kHz                                                                   | the negotiated configuration (pixel clock × deep-colour factor, halved for YCbCr 4:2:0, with any pixel repetition); **rounded up**, never truncated (12 bpc on a 1 kHz clock gives half kHz, and truncating 340 000.5 would pick 1/10)                                                           |
| Sink facts          | `SinkTmdsFacts { scdc_present, scrambling_supported, lte_340mcsc_scramble }` | the sink's HDMI Forum capabilities (`From<&HdmiForumSinkCap>`; no HF-SCDB means `SinkTmdsFacts::NONE`); `scrambling_supported` is SCDC present and a non-zero `Max_TMDS_Character_Rate`, as Linux derives `scrambling.supported` [11]; the caller may clear a fact for a sink known to misbehave |
| Source fact         | `SourceTmdsFacts { low_rate_scrambling }`                                    | whether the source's PHY can scramble at or below 340 Mcsc                                                                                                                                                                                                                                       |
| Policy              | `TmdsPolicy { low_rate_scrambling, threshold }`                              | the caller; `TmdsPolicy::DEFAULT` follows the majority of the sources (rules 3 and 4)                                                                                                                                                                                                            |

### Rules

1. **Above the threshold**, the clock ratio is 1/40 and the link is scrambled; the sink's
   `TMDS_Config` gets both `TMDS_Bit_Clock_Ratio` and `Scrambling_Enable`. Every source
   examined does this: the Linux DRM helper and vc4 (one lineage), i915, dw-hdmi, amdgpu,
   nouveau, Tegra, MediaTek v2, Xilinx and Amlogic [1], [3]–[10]. It needs a sink that can
   take it (rule 5).
2. **At or below the threshold**, the clock ratio is 1/10, and a sink with SCDC has the
   ratio cleared in `TMDS_Config` [1], [3]–[10].
3. **Scrambling at or below the threshold** is on when all three allow it: the source can
   (`low_rate_scrambling` fact), the sink supports scrambling (`scdc_present` and
   `scrambling_supported`) and declares `LTE_340Mcsc_scramble`, and the policy permits
   it. The sources that scramble at low rates, i915, amdgpu, nouveau and dw-hdmi, enable
   it whenever the sink declares it [3][4][5][6], and all four take the declaration from
   Linux's `scrambling.low_rates`, which the EDID parser sets only for a sink with SCDC and
   a non-zero `Max_TMDS_Character_Rate` [11]; i915, nouveau and dw-hdmi also check
   `scrambling.supported` themselves [3][4][6]. Xilinx leaves it to the application, off by
   default [9]. The helper lineage, Tegra, MediaTek v2 and Amlogic do not scramble at low
   rates [1][7][8][10] (in the helper lineage it is a TODO [2]). Hence a setting,
   **default on**.
4. **The threshold** is "above 340 Mcsc" by default; "at or above 340 Mcsc" is the
   alternative, and the two differ only at exactly 340 000 kHz. Above: the helper lineage,
   i915, dw-hdmi, nouveau, Xilinx and Amlogic [1][3][4][6][9][10]; at or above: Tegra and
   MediaTek v2 [7][8]. amdgpu is split and counted on neither side: its sink write uses
   "above" (`link_ddc.c:575`), its own scrambler "at or above"
   (`dce_stream_encoder.c:621`), so at exactly 340 000 kHz it scrambles without configuring
   the sink [5]. nouveau, Tegra and MediaTek v2 compare the mode's pixel clock, not the
   character rate [6][7][8]. Hence a setting, with the majority as default.
5. **A sink that cannot take scrambling** lacks `scdc_present` or `scrambling_supported`.
   Without SCDC the sink cannot be configured at all: no `TMDS_Config` value, ratio 1/10,
   no scrambling.
   - **Above 340 000 kHz**, `choose` refuses with `ParamsError::ScramblingUnsupported`,
     naming every missing fact (decision 2).
   - **At exactly 340 000 kHz with `AtOrAbove340`**, such a sink gets the plain 1/10 link,
     which is a valid link at that rate under the default threshold, and `ScramblingOff`
     names the missing fact (decision 3). Tegra enables its scrambler and the sink's ratio
     only with SCDC [7]; MediaTek v2 sets its own scrambler regardless and skips only the
     sink write [8].

   A source scrambling with an unconfigured sink is never an outcome of `choose`. One
   source, Amlogic, writes `TMDS_Config` even to a sink that declares no SCDC [10]; every
   other source gates the write on SCDC, and so does `spiritlevel`.

### Output

`TmdsLinkParams`: the rate, the clock ratio, whether the link is scrambled and why
(`ScramblingDecision`), and the sink's `TMDS_Config` contents (`SinkTmdsConfig`, `None` for
a sink without SCDC). The caller programs its PHY with the ratio and the scrambling flag
and writes the sink's value through its SCDC implementation. `spiritlevel` states *what* the
register holds, not its bit layout: the encoding belongs to the SCDC implementation
([culvert](https://github.com/ashlar-display/culvert)'s `TmdsConfig`, for one).

---

## Key Types

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
/// e.g. a DVI or HDMI 1.x sink). With the `display-types` feature,
/// `From<&HdmiForumSinkCap>` and `From<Option<&HdmiForumSinkCap>>`.
#[non_exhaustive]
pub struct SinkTmdsFacts {
    pub scdc_present: bool,
    pub scrambling_supported: bool,   // SCDC and a non-zero Max_TMDS_Character_Rate
    pub lte_340mcsc_scramble: bool,   // counts only with scrambling_supported
}

/// What the source's PHY can do (`new`).
#[non_exhaustive]
pub struct SourceTmdsFacts { pub low_rate_scrambling: bool }

/// The caller's choices (rules 3 and 4). `Default`/`DEFAULT`: low-rate scrambling on,
/// `Above340`; `with_low_rate_scrambling`, `with_threshold`.
#[non_exhaustive]
pub struct TmdsPolicy { pub low_rate_scrambling: bool, pub threshold: ScramblingThreshold }

/// The two fields of `TMDS_Config` (0x20), what is written and what a read returns
/// (`new`). Its bit layout belongs to the SCDC implementation.
#[non_exhaustive]
pub struct SinkTmdsConfig { pub scrambling_enable: bool, pub high_tmds_clock_ratio: bool }

/// Why the link is or is not scrambled.
#[non_exhaustive]
pub enum ScramblingDecision {
    Required,             // above the threshold (rule 1)
    LowRate,              // at or below; source, sink and policy all allow it (rule 3)
    Off(ScramblingOff),   // 1/10, not scrambled (at or below, or the rule 5 fallback)
}

/// Every condition that keeps a 1/10 link unscrambled; at least one is set. Each flag
/// is judged on its own fact, independently of the others.
#[non_exhaustive]
pub struct ScramblingOff {
    pub no_scdc: bool,
    pub no_scrambling_support: bool,   // scrambling_supported not set
    pub sink_does_not_declare: bool,   // LTE_340Mcsc_scramble not set
    pub source_cannot: bool,
    pub policy_off: bool,
}

/// The chosen link, built only by `choose`: `rate_khz()`, `clock_ratio()`,
/// `scrambling()` (true for `Required` and `LowRate`), `scrambling_decision()`,
/// `sink_config() -> Option<SinkTmdsConfig>` (`None` without SCDC).
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
    /// A rate above 340 000 kHz on a sink that cannot take scrambling (rule 5); every
    /// missing fact is set. The variant is `#[non_exhaustive]` too.
    ScramblingUnsupported { rate_khz: u32, no_scdc: bool, no_scrambling_support: bool },
}
```

All are `Debug, Clone, Copy, PartialEq, Eq, Hash`; all constructors are `const fn`, so a
firmware can fix its policy and facts at compile time.

- **Why private fields on `TmdsLinkParams`:** the PHY and the sink must agree. With public
  fields a caller could change the ratio and leave the sink's value stale; with getters the
  only way to a `TmdsLinkParams` is through the rules.
- **Why `ScramblingDecision`:** an unscrambled low-rate link is not a failure, but a
  caller expecting scrambling should see which of source, sink or policy declined it.
- **Why the sink facts are a struct of their own,** not `HdmiForumSinkCap` directly: the
  decision needs three flags, a caller can override a misbehaving sink without editing
  parsed EDID data, and the API is tied to display-types only through the `From` impls.

---

## `no_std`, `alloc`, and `async`

`#![no_std]` without `alloc` in every build, and `#![forbid(unsafe_code)]`. `choose` is
synchronous and pure, a `const fn`, so there is nothing async in 0.1. CI builds the crate
for a bare target (`thumbv7em-none-eabi`) with and without default features, and checks the
MSRV (Rust 1.85).

---

## Testing

The input space is small, so the tests cover it whole:

- **Every combination:** both thresholds × the three sink facts × the source fact × the
  policy flag, at rates either side of and at 340 000 kHz, from 0 to `u32::MAX`; each case
  checked against the rules, including the `ScramblingDecision` and `ParamsError`.
- **The boundary:** 340 000 kHz gives 1/10 with `Above340`; with `AtOrAbove340` it gives
  1/40 on a sink that can take scrambling and the plain 1/10 link, with the reason, on one
  that cannot; 340 001 kHz gives 1/40 with both, or `ScramblingUnsupported`.
- **The gate on `LTE_340Mcsc_scramble`:** a sink that declares it without scrambling
  support gets no low-rate scrambling.
- **Every reason:** each `ScramblingOff` flag and each `ScramblingUnsupported` flag is set
  exactly when its own fact blocks scrambling, whatever the others are.
- **Invariants:** `sink_config()` is `None` exactly when the sink has no SCDC, and
  otherwise matches `clock_ratio()` and `scrambling()`; `scrambling()` is true exactly for
  `Required` and `LowRate`; `rate_khz()` returns the input.
- **`const` use:** tests evaluate `choose` and the constructors in `const` items, so a
  change that breaks `const fn` fails the build.
- **`From` impls:** every combination of the three HF-SCDB fields against a literal table
  of expected facts, and `None` maps to `SinkTmdsFacts::NONE`.
- **The README's example** runs as a doctest.

CI measures line coverage and fails if it drops more than 0.1% below the recorded
baseline.

---

## Design Principles

- **Procedure, not policy.** Where HDMI sources differ, the behaviour is a setting with the
  majority as default (rules 3 and 4); the defaults are documented and changeable.
- **Every reason, not the first.** A decision or an error carries every condition that
  applies, so a diagnostic tool's user sees all of them at once (decision 4).
- **Cited behaviour.** Each rule cites the public implementations it follows; where
  `spiritlevel` departs from them, the decision says so and why.
- **The output stays consistent.** The PHY's settings and the sink's register come from one
  value that only the rules can build.
- **Stable output types.** Output structs and enums are `#[non_exhaustive]`, so fields and
  reasons can be added without breaking callers; `ClockRatio`, a single register bit, is
  exhaustive.
- **No dependencies by necessity.** The core needs nothing; display-types is an optional
  convenience.

---

## Decisions

1. **0.1 is the I/O-free decision alone** (2026-11-06). The procedures need PHY, power,
   clock and delay traits that do not exist in hdmi-hal yet; the decision does not, and is
   useful on its own. Rejected: waiting to release the crate with the procedures.
2. **A rate above 340 000 kHz on a sink that cannot take scrambling is refused**
   (2026-11-07). The sources gate the sink write and carry on unscrambled; `spiritlevel`
   returns `ParamsError::ScramblingUnsupported` instead. Negotiation should never pick
   such a rate, so reaching `choose` with it means the inputs disagree, and an unscrambled
   link would hide that. Rejected: following the sources.
3. **At exactly 340 000 kHz with `AtOrAbove340`, a sink that cannot take scrambling gets
   the plain 1/10 link** (2026-11-08). It is a valid link at that rate under the default
   threshold, so refusing would be stricter than needed; `ScramblingOff` names the missing
   fact, so the departure from the requested threshold is visible. Rejected: refusing like
   decision 2; applying low-rate scrambling there, which needs scrambling support the sink
   lacks.
4. **Every reason, not the first** (2026-11-07). `ScramblingOff` and
   `ScramblingUnsupported` set a flag for each condition that applies, each judged on its
   own fact. They fit in a `Copy` value without `alloc`, so there is no capacity reason to
   stop at one. Rejected: a single reason, which would make a user fix one condition and
   then meet the next.
5. **Low-rate scrambling needs scrambling support** (2026-11-08). `LTE_340Mcsc_scramble`
   counts only with SCDC and a non-zero `Max_TMDS_Character_Rate`, as in every source that
   scrambles at low rates (rule 3), with its own reason flag, `no_scrambling_support`.
   Rejected: taking the declaration alone; gating it inside the `From` impl, which would
   report "does not declare" for a sink that does.

---

## What comes next

Release 0.2 is planned to add the procedures that carry out the decision: bring-up,
tear-down and a reconnect check, over hdmi-hal's PHY, power, clock and delay traits and a
small SCDC trait of the crate's own, with an adapter for culvert behind a feature, and an
async companion crate, `spiritlevel-async`. 0.1's types may change where the procedures
need it; before 1.0 that is a breaking minor release, listed in the changelog.

---

## Sources

Driver source read at these commits; line numbers refer to them.

- Linux, github.com/torvalds/linux `e9954f3fbddd` (2026-10-31)
- Xilinx embeddedsw, github.com/Xilinx/embeddedsw `97f2baf7f69e` (2026-06-24)
- Amlogic, github.com/CoreELEC/common_drivers `8ca9b6c3b715` (2026-10-31)

|      | Source                              | Location                                                                                                                                                                                 |
|------|-------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| [1]  | Linux DRM helper, vc4 (one lineage) | `drivers/gpu/drm/display/drm_hdmi_helper.c:476–482`; `drivers/gpu/drm/vc4/vc4_hdmi.c:134–141`, `815–816`                                                                                 |
| [2]  | Linux DRM state helper              | `drivers/gpu/drm/display/drm_hdmi_state_helper.c:599` (low-rate scrambling a TODO)                                                                                                       |
| [3]  | i915                                | `drivers/gpu/drm/i915/display/intel_hdmi.c:2433–2435`, `2793–2795`                                                                                                                       |
| [4]  | dw-hdmi                             | `drivers/gpu/drm/bridge/synopsys/dw-hdmi.c:1392–1394`, `1400–1402`, `1424–1431`, `2101–2102`, `2118`                                                                                     |
| [5]  | amdgpu                              | `drivers/gpu/drm/amd/display/dc/link/protocols/link_ddc.c:571–613`; `dc/core/dc_stream.c:158`; `amdgpu_dm/amdgpu_dm_connector.c:1520–1521`; `dc/dce/dce_stream_encoder.c:621`, `629–640` |
| [6]  | nouveau                             | `drivers/gpu/drm/nouveau/dispnv50/disp.c:797`, `800–815`                                                                                                                                 |
| [7]  | Tegra                               | `drivers/gpu/drm/tegra/sor.c:2165–2181`, `2202`                                                                                                                                          |
| [8]  | MediaTek v2                         | `drivers/gpu/drm/mediatek/mtk_hdmi_v2.c:86–100`, `746`                                                                                                                                   |
| [9]  | Xilinx                              | `XilinxProcessorIPLib/drivers/v_hdmitx1/src/xv_hdmitx1.c:630–635`, `742–744`; `v_hdmitxss1/src/xv_hdmitxss1.c:2938–2957`                                                                 |
| [10] | Amlogic                             | `drivers/media/vout/hdmitx21/hw/hdmi_tx_hw.c:668–676`, `2780–2799`, `2889–2906`                                                                                                          |
| [11] | Linux EDID parser                   | `drivers/gpu/drm/drm_edid.c:6340–6356` (`scrambling.supported`: SCDC present and a non-zero `Max_TMDS_Character_Rate`; `scrambling.low_rates` only within that)                          |
