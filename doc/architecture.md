# Architecture

This document describes `spiritlevel` as released: what it decides and does, the rules it
follows and where they come from, its types, and why it works the way it does. Claims about HDMI behaviour cite
at least two independent public implementations of HDMI sources (see "Sources").

## Role

`spiritlevel` brings up and tears down the TMDS link of an HDMI source. First it decides,
without any I/O, what the link will be: from the TMDS character rate, what the sink and the
source can do, and the caller's policy, the link's bit clock ratio (1/10 or 1/40), whether
it is scrambled and why, and what the sink's `TMDS_Config` register must hold. Then its
procedures carry that out over the PHY and the sink's SCDC: bring-up, which configures the
sink, starts TMDS output in the right order and checks that the sink follows; tear-down;
and, after a hotplug while the link is up, the reconnect check and the quick restore.

The decision is a pure `const fn`, usable on its own by a caller that drives its hardware
itself. The procedures define the interfaces they need instead of depending on particular
implementations: a small SCDC trait of the crate's own (`TmdsScdc`) and
[hdmi-hal](https://github.com/ashlar-display/hdmi-hal)'s TMDS PHY, clock and delay traits.
A driver, a firmware image or a vendor SDK supplies those and uses the crate without a link
orchestrator or a negotiation layer above it.

TMDS and FRL are separate link modes with separate procedures. FRL link training is
[plumbob](https://github.com/ashlar-display/plumbob)'s; `spiritlevel` is its counterpart for
TMDS, beside it in the stack rather than on top of it.

It implements the procedure, not policy: where implementations of HDMI sources differ, the
behaviour is a setting with a documented default, and how often to retry is the caller's.

---

## Scope

`spiritlevel` covers:

- **Choosing the link parameters:** the clock ratio, scrambling and the sink's
  `TMDS_Config` contents, from the character rate, the sink's and source's facts, and the
  policy.
- **Explaining the choice:** every reason a link is not scrambled, and every missing fact
  when a rate is refused.
- **Bring-up:** clearing the sink's FRL settings when the link may have been in FRL,
  `Source_Version` on an SCDC sink when the settings say, writing `TMDS_Config`, starting
  TMDS output in an order that never sends scrambled TMDS before the sink is told, and
  checking `Scrambling_Status`.
- **Tear-down:** stopping TMDS output and, when the caller asks, clearing the ratio and
  scrambling on the sink and in the source.
- **After a hotplug while the link is up:** the reconnect check, which reads `TMDS_Config`
  back against the running link, and the quick restore, which writes it again without a
  new bring-up.
- **One attempt per call:** each call makes one attempt and its check, and reports the
  outcome and every step as an event.

It does not cover:

- **Computing the character rate:** it depends on the negotiated colour format, depth and
  pixel repetition, which is the negotiation layer's knowledge. The caller passes it in.
- **Checking the rate against the TMDS maximum** or the sink's, source's and cable's
  ceilings: the negotiation layer checks those before a link is chosen.
- **EDID reading and parsing:** the sink facts come from the parsed HDMI Forum capabilities.
- **The hardware:** the PHY's registers and electrical settings (swing, pre-emphasis, lane
  mapping) and the SCDC transfers are the implementations of the traits.
- **FRL:** plumbob.
- **InfoFrames and other data island packets, AVMUTE, HDCP.**
- **Hotplug detection and when to bring the link up or down:** the caller decides; the
  reconnect check is a tool it calls.
- **Retry and polling schedules:** the caller's, with `spiritlevel` making one attempt per
  call.

---

## Dependencies and features

The features and the dependencies, with their versions in `Cargo.toml`:

```toml
[features]
default = ["display-types"]
culvert = ["dep:culvert"]

[dependencies]
display-types = { default-features = false, optional = true }
hdmi-hal = { default-features = false }
culvert = { default-features = false, optional = true }
```

Dependencies point only down the stack: nothing below `spiritlevel` depends on it.

- **hdmi-hal** (always): the TMDS PHY trait (`TmdsPhy`, with its capabilities report and
  `TmdsClockRatio`, which `spiritlevel` re-exports as `ClockRatio`), the clock and delay
  traits, and the cancel flag (`Cancel`, re-exported). It is a public dependency. hdmi-hal itself depends on display-types, so
  display-types is always in the graph.
- **`display-types`** (default): `From<&HdmiForumSinkCap>` and
  `From<Option<&HdmiForumSinkCap>>` for `SinkTmdsFacts`. Without it the caller states the
  sink facts itself. display-types is then a public dependency (its type appears in those
  impls), so a breaking display-types release means a breaking `spiritlevel` release.
- **`culvert`**: `TmdsScdc` for [culvert](https://github.com/ashlar-display/culvert)'s
  SCDC client (`culvert::Scdc`, over any `ScdcTransport`), with culvert's errors wrapped in
  `CulvertError`. The adapter lives here, so a change to `TmdsScdc` never forces a culvert
  release.

---

## Choosing the link

`TmdsLinkParams::choose(rate_khz, sink, source, policy)` returns the link or a
`ParamsError`.

### Inputs

| Input               | Type                                                                         | Where the caller gets it                                                                                                                                                                                                                                                                         |
|---------------------|------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| TMDS character rate | `u32`, kHz                                                                   | the negotiated configuration (pixel clock × deep-colour factor, halved for YCbCr 4:2:0, with any pixel repetition); **rounded up**, never truncated (12 bpc on a 1 kHz clock gives half kHz, and truncating 340 000.5 would pick 1/10)                                                           |
| Sink facts          | `SinkTmdsFacts { scdc_present, scrambling_supported, lte_340mcsc_scramble, frl_capable }` | the sink's HDMI Forum capabilities (`From<&HdmiForumSinkCap>`; no HF-SCDB means `SinkTmdsFacts::NONE`); `scrambling_supported` is SCDC present and a non-zero `Max_TMDS_Character_Rate`, as Linux derives `scrambling.supported` [11]; `frl_capable` a non-zero `Max_FRL_Rate`, used by bring-up's FRL clear, not by the rules here; the caller may clear a fact for a sink known to misbehave |
| Source facts        | `SourceTmdsFacts { scrambling, low_rate_scrambling, max_character_rate_khz }` | the PHY's capabilities report (`From<&TmdsPhyCapabilities>`): whether it has a scrambler, whether it can scramble at or below 340 Mcsc, and its highest TMDS character rate                                                                                                                       |
| Policy              | `TmdsPolicy { low_rate_scrambling, threshold, tmds_config_gate }`            | the caller; `TmdsPolicy::DEFAULT` follows the majority of the sources where there is one (rules 3 and 4), and a chosen default where there is none (rule 6)                                                                                                                                       |

### Rules

0. **A rate above the source's own maximum** (`max_character_rate_khz`) is refused first,
   with `ParamsError::AboveSourceMaximum`, so a sink never gets 1/40 for a link the PHY
   cannot then drive. A capability of this source, not a behaviour to cite.
1. **Above the threshold**, the clock ratio is 1/40 and the link is scrambled; the sink's
   `TMDS_Config` gets both `TMDS_Bit_Clock_Ratio` and `Scrambling_Enable`. Every source
   examined does this: the Linux DRM helper and vc4 (one lineage), i915, dw-hdmi, amdgpu,
   nouveau, Tegra, MediaTek v2, Xilinx and Amlogic [1], [3]–[10]. It needs a sink that can
   take it and a source with a scrambler (rule 5).
2. **At or below the threshold**, the clock ratio is 1/10, and a sink with SCDC has the
   ratio cleared in `TMDS_Config` [1], [3]–[10].
3. **Scrambling at or below the threshold** is on when all three allow it: the source can
   (`scrambling` and `low_rate_scrambling`), the sink supports scrambling (`scdc_present` and
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
     naming every missing fact. The sources gate the sink write and carry on unscrambled;
     `spiritlevel` refuses instead, because negotiation never picks such a rate, so
     reaching `choose` with it means the inputs disagree, and an unscrambled link would
     hide that.
   - **At exactly 340 000 kHz with `AtOrAbove340`**, such a sink gets the plain 1/10 link,
     which is a valid link at that rate under the default threshold, and `ScramblingOff`
     names the missing fact, so the departure from the requested threshold is visible.
     Tegra enables its scrambler and the sink's ratio only with SCDC [7]; MediaTek v2 sets
     its own scrambler regardless and skips only the sink write [8].

   - **A source without a scrambler** (`scrambling` not set) is treated the same way,
     since its sink would otherwise be told to expect scrambling the source cannot send:
     refused above 340 000 kHz, with `ScramblingUnsupported::source_cannot_scramble` set
     together with any sink flags that also apply; the plain 1/10 link at exactly
     340 000 kHz with `AtOrAbove340` (`ScramblingOff::source_cannot`); and no low-rate
     scrambling (rule 3). `SourceTmdsFacts::new` takes the source to have one.

   A source scrambling with an unconfigured sink is never an outcome of `choose`. One
   source, Amlogic, writes `TMDS_Config` even to a sink that declares no SCDC [10]; every
   other source gates the write on SCDC, and so does `spiritlevel`.
6. **Which SCDC sinks get a `TMDS_Config` write on a plain link** is a setting,
   `TmdsPolicy::tmds_config_gate`, with **any sink with SCDC** (`AnyScdcSink`) as chosen
   default. amdgpu, MediaTek v2 and Xilinx write to any SCDC sink; i915, dw-hdmi and
   nouveau only to one that supports scrambling; Amlogic to every sink [25]: no majority.
   The choice matters because most sources write `TMDS_Config` at every bring-up, a plain
   one included [20], while the helper lineage and Tegra clear it only at their own
   tear-down [24], so a sink left at 1/40 by something else would stay there. The groups
   differ only for a sink with SCDC that declares no TMDS rate above 340 Mcsc. Under
   `ScramblingSink`, such a sink's plain link has no `sink_config()`. The gate is a policy
   setting, not an edit of the sink facts: the facts say what the sink declares, the
   policy what the source does about it, so `Source_Version` and the FRL clear still
   happen on a sink the gate leaves unwritten. Amlogic's "every sink" is not offered
   (rule 5): a caller whose sink has SCDC despite its EDID sets `scdc_present`.

### Output

`TmdsLinkParams`: the rate, the clock ratio, whether the link is scrambled and why
(`ScramblingDecision`), the sink's `TMDS_Config` contents (`SinkTmdsConfig`, `None` exactly
when no write is due: a sink without SCDC, or a plain link rule 6 leaves unwritten), and the
sink facts it was chosen from (`sink()`, which bring-up's FRL clear uses). It also keeps the
source facts and policy, so `fits(sink)` can tell whether choosing again for other sink
facts gives the same link, as the reconnect check asks. The procedures program the PHY with the ratio and the scrambling flag and write the
sink's value through `TmdsScdc`; a caller without them does the same itself. `spiritlevel` states *what* the
register holds, not its bit layout: the encoding belongs to the SCDC implementation
([culvert](https://github.com/ashlar-display/culvert)'s `TmdsConfig`, for one).

---

## The procedures

Each procedure takes the running link's `TmdsLinkParams`, makes one attempt (and, for
bring-up and the quick restore, one check) and returns what happened; whether and when to
try again is the caller's. The procedures are written once as an I/O-free core
(`spiritlevel::procedure`, over the `TmdsIo` trait) and driven synchronously by `TmdsLink`,
over a `TmdsScdc`, a `TmdsPhy`, a `Clock`, a `Delay` and an optional `Cancel` flag. Every step
is also recorded as a `TmdsEvent` ("Diagnostics").

### Before both paths: the FRL clear and `Source_Version`

On a sink with SCDC, bring-up first does two things, in this order; a sink without SCDC
gets neither.

1. **The FRL clear.** A source that may have used FRL clears the sink's FRL settings
   before TMDS: `Config_1` (the FRL rate), then `FLT_update` [36]. amdgpu does it on every
   TMDS bring-up, Amlogic only on a sink that declares FRL [22][23]. Two sources, no
   majority, so a setting, `ClearFrl`: **`IfSinkDeclaresFrl`** (chosen default; `Config_1`
   holds FRL settings only on such a sink, `SinkTmdsFacts::frl_capable`) or `EveryScdcSink` (amdgpu's,
   for a driver whose EDID may not describe the sink behind it). `SinkTmdsFacts::new` sets
   `frl_capable` for a sink with SCDC, so a caller that never states it still gets the
   clear, which on a sink without FRL writes a register that sink does not use.
2. **`Source_Version` = 1.** No two sources agree on when to write it [39]: amdgpu on every
   bring-up when `Sink_Version` is 1, the helper lineage and dw-hdmi only on a scrambled link
   (the helper only to a sink reporting a version, dw-hdmi its version, 0 included), Xilinx
   unconditionally when it detects the sink, Amlogic only on its FRL path. So a setting,
   `SourceVersionWrite`: **`EveryBringUp`** (chosen default, closest to Xilinx's
   unconditional write), `IfSinkVersion1` (amdgpu's), `WhenScrambling` (the helper lineage's:
   on a scrambled link, to a sink reporting a version) or `Never` (for a sink that
   misbehaves).

Both come before the `TMDS_Config` write, as in every source that does them [22]; the order
between them is amdgpu's, the one source that does both. A failed step here is a warning
and the bring-up carries on, whatever the error's kind: neither source can see the result of
its FRL clear [27], and no source treats `Source_Version` as fatal. A bus that is gone
(`Detached`) fails the `TMDS_Config` write that follows, which stops the bring-up there; a
request the SCDC implementation cannot do (`Unsupported`) does not block the link.

### Bring-up

`bring_up(link, video, config)` brings up the link `choose` decided, carrying `video` (the
TMDS character rate's colour depth and format, which the PHY needs). The steps for a
scrambled link above 340 Mcsc, with `BringUpConfig::DEFAULT`:

1. **Output off.** TMDS output is stopped before anything is written, and stays stopped
   until step 5. i915, amdgpu, the dw-hdmi family, MediaTek v2, Xilinx and Amlogic write the
   sink with their output off [12]–[17]; vc4 and Tegra write it with unscrambled output
   running and turn their scrambler on afterwards [18]. A majority with dissent, so a
   setting, `OutputDuringSinkWrite`: **`Stopped`** (default) or `Running` (below).
2. **The sink's `TMDS_Config`, both bits in one write**, from `sink_config()`. amdgpu,
   nouveau and Amlogic write both bits in one transaction [14][6][17]; the other six
   lineages (i915, the helper lineage, the dw-hdmi family, Tegra, MediaTek v2, Xilinx) write
   them one at a time, in either order [37]. None depends on an order, and the register ends
   the same, so this is a platform difference rather than a dissent: `spiritlevel` writes
   both at once, which leaves no moment in which the sink holds one bit without the other. How it is written is a setting,
   `TmdsConfigWrite`: **`ReadModifyWrite`** (default; the bits set, every other bit kept as
   read), as seven lineages do (the five that write through Linux's SCDC helper, nouveau and
   Xilinx) [37]; or `WholeByte`, as amdgpu and Amlogic do [14][17] (one transaction, no
   read).
3. **The PHY:** the character rate, the clock ratio and the source's scrambler, with the
   output still off, as Xilinx and MediaTek v2 set their scrambler before output starts
   [16][15].
4. **The wait after the ratio write**, `ratio_wait`, **default 1 ms**: the five lineages
   that write the ratio through Linux's SCDC helper sleep 1–2 ms after every ratio write
   (i915, the helper lineage, the dw-hdmi family, Tegra, MediaTek v2) [37], the dw-hdmi
   family 100 ms more in its own code [12]; amdgpu, nouveau, Xilinx and Amlogic start output
   at once [14][6][16][17]. It is measured from the write, so the PHY's own time counts
   towards it, and spent in slices of at most `check_interval` (for cancellation).
5. **Output on.** If more than `scrambling_deadline` (**default 100 ms**) has passed since
   the write, the outcome carries a `DeadlineMissed` warning saying by how much. The 100 ms is
   stated in the comments of three lineages: i915's (the sink may disable scrambling if it
   sees no scrambled clock within 100 ms of the write) [19], and Linux's SCDC helper's and
   dw-hdmi's (1 to 100 ms from the ratio write until transmission resumes) [37][12]. dw-hdmi's
   own code waits longer than that [12], so it warns rather than stops: the check shows
   whether the sink followed.
6. **The check** of `Scrambling_Status` ("The scrambling check").

**No scrambled TMDS ever reaches the sink before its write**, in every source and in both
orders. With `Running`, the PHY is configured with its scrambler off and the output started
first (steps 3 and 5), then `TMDS_Config` is written and the wait spent, and only then the
source's scrambler turned on; the deadline runs from the write to the scrambler, and the
check from the scrambler. This is vc4's order: the PHY runs at the link's clock ratio from
the start, so on a 1/40 link the sink sees the 1/40 clock before its write [18]. Tegra,
which also writes with its output running, switches its clock mode only together with its
scrambler, after the write [18]; `TmdsPhy` sets the ratio only with the output stopped, so
that order cannot be expressed over it.

**At or below 340 Mcsc** the same steps run, with these differences:

- **`TMDS_Config` is written at every bring-up** where a write is due (rule 6), with the
  ratio cleared and `Scrambling_Enable` set only for a scrambled low-rate link, as i915,
  amdgpu, the dw-hdmi family, nouveau, MediaTek v2, Xilinx and Amlogic do [20], so a sink
  left at 1/40 by something else (another source, a firmware that ran first) is corrected.
  The wait follows every `TMDS_Config` write, one that clears the ratio included [37].
- **A plain link has no deadline and no check**: `Scrambling_Status` stays 0 there, so
  there is nothing to wait for (`CheckResult::NotApplicable`). A scrambled low-rate link
  has both, as i915 polls a low-rate scrambled link as it does a high-rate one [21].
- **A failed write on a plain link is followed by one `TMDS_Config` read**, whose result
  (the two bits and the byte, or the read's error) is part of the write's warning: with no
  check, a sink that kept 1/40 after a failed clear would otherwise go unnoticed.

**A failed `TMDS_Config` write** is a setting, `OnTmdsConfigFailure`: **`CarryOn`**
(default) keeps a warning and continues to the PHY and output, and the check shows whether
the sink followed. Most sources carry on (i915 logs it and enables the port, the others do
not look at the result) [26]; the DRM helper clears both sink bits and leaves its scrambler
off, and nouveau returns when its read fails [38]. `Stop` ends the bring-up with an error.
With read-modify-write, the warning (an `UpdateError`) says which half failed, and an error
or event names it by its step, `ReadTmdsConfig` or `WriteTmdsConfig`: a failed read means
nothing was written. Carrying on is also what a DRM driver needs, since its enable hooks cannot fail.

### The scrambling check

`Scrambling_Status` is read every `check_interval` (**default 1 ms**) until the sink reports
scrambling or `check_window` (**default 200 ms**) ends, counted from output start (from the
scrambler with `Running`): i915's values [19]. The helper lineage and Tegra re-check only
after 1 s and 5 s [1][7]; that is the caller's repeat, not this check.

- **Failed reads do not end it.** Polling goes on until the status is set or the window
  ends, so the result tells a sink that missed one read and then reported scrambling from
  one that never answered. It stops early only on an error that will not pass: the bus is
  gone (`DdcErrorKind::Detached`) or the read cannot be done (`Unsupported`). Any other kind,
  including one added to `DdcErrorKind` later, counts as transient: polling on costs a window
  at most, stopping early would misreport a link that came up.
- **Bounded by count as well as by time:** at most `ceil(window / interval) + 1` reads, so a
  clock that does not advance (a timer not started) ends the check instead of hanging the
  bring-up; reaching the count is recorded (`read_cap_reached`). The count bounds reads, not
  time: on such a clock the check takes as long as its reads take. No read starts after the
  window: the check ends when another interval would reach the window's end, and again when
  a delay ran past it; a slow read that started inside the window may end after it. The
  interval is spent by the `Delay`, not measured by the clock.
- **At least one read**, whatever the window: a window of 0 reads once, and an interval of
  0 reads again at once. `BringUpConfig::validate()` reports the settings that defeat their
  purpose: a deadline not longer than the wait (every bring-up would miss it), an interval
  not shorter than the window (one read) and an interval of 0 (reads back to back); the
  procedures run with any values.

The check ends `Set` (with the time it took), `NotSet` (the window ended) or `Unreadable`
(every read failed, or one failed with a non-transient error), with the number of reads and
failed reads and the first error. The link is left running in every case: no source treats
an unset `Scrambling_Status` as fatal, so the result is part of the outcome, not a condition
of it, and repeating the bring-up later is the caller's. `NotSet` and `Unreadable` are also a
warning, `ScramblingNotConfirmed`, since a link the sink does not follow shows nothing: a
caller that only looks at the warnings still sees it.

### What a bring-up returns

- **`BringUpOutcome::Up(LinkUp)`** when steps 1–5 completed: the link, when the sink was
  written and the output started, the check, and the **warnings**, each kind at most once:
  a failed FRL clear (`Config_1` or `FLT_update`), `Sink_Version` read or `Source_Version`
  write, a failed `TMDS_Config` write under `CarryOn`, a missed deadline, and a check that did
  not see scrambling.
- **`BringUpError`** when a step failed in a way that stops the bring-up: a PHY call (output
  off, configure, scrambler, output on), or the sink's `TMDS_Config` write with a
  non-transient error or under `Stop`. It names the step, the error, whether the output may be
  running, and the warnings gathered.
- **After an error, a tear-down**, with the caller's `TearDownConfig`, and its result in the
  error (`Cleanup`); with the sink taken to be gone after `Detached`. `tear_down_on_error:
  false` leaves both ends as the error left them, for a tool that wants to read the state.
- **`BringUpOutcome::Cancelled`** when the caller's cancel flag stopped it ("Cancellation").

### Cancellation

A caller stops a running procedure through its `Cancel` flag (hdmi-hal's, re-exported;
`AtomicBool` and any reference to a flag implement it),
never by dropping the future: a dropped call leaves the PHY and the sink in an unknown state.
Bring-up reads the flag at entry, before each SCDC step, before each slice of the ratio wait
and before each read of the check, so a cancel is seen within one `check_interval` whatever
the configured wait (with an interval of 0 the wait is one slice, read before it). Once it is set, the bring-up ends `BringUpOutcome::Cancelled`, never an
error, with where it was seen (`CancelPoint`):

- at entry, before any I/O, with nothing torn down (`Cleanup::Skipped`);
- anywhere else, after a tear-down with the caller's `TearDownConfig`, whatever
  `tear_down_on_error` says.

Tear-down is the recovery and is not cancelled; the reconnect check is one read; the quick
restore reads the flag in its check alone. No procedure clears the flag: the caller resets it
before its next call, or that call ends `Cancelled` at entry.

### Tear-down

`tear_down(link, config, presence)` stops the link the bring-up started; `presence` says
whether the sink is still there, which only the caller knows. Powering the PHY down is not
part of it.

**Whether the sink is cleared** is a setting, `TearDownConfig::clear_sink`, **default off**:
the dw-hdmi family, nouveau, MediaTek v2, Xilinx and Amlogic leave `TMDS_Config` at tear-down
and rely on the next bring-up writing it [28]; i915, amdgpu, the helper lineage and Tegra
clear both bits [29]. Off is safe when the next link is a TMDS bring-up by this crate, which
writes `TMDS_Config` every time. **Before FRL it is not**: FRL training leaves `TMDS_Config`
alone, so a 1/40 scrambled link followed by FRL would leave both bits set for the whole FRL
session; amdgpu, the one source with separate TMDS and FRL paths, clears on every disable
[29]. A caller taking the link from TMDS to FRL, or handing the sink to something that may not
write it, sets `clear_sink`. Amlogic clears on suspend for sinks on a quirk list [31], which
this setting also covers.

With clearing on, the steps are:

1. **The sink's `TMDS_Config` to 0, in one write.** When is a setting, `SinkClearTiming`:
   **`BeforeOutputStops`** (default), as i915, amdgpu, the helper lineage and Tegra do [29],
   or `AfterOutputStops`, Amlogic's quirk path [31]. Only what the bring-up set is cleared: a
   plain link needs no write, as the helper lineage and Tegra write nothing when they did not
   scramble [29].
2. **The source's scrambler off.** Its order against the sink is a setting, `ScramblerOff`,
   a majority with dissent: **`AfterSink`** (default; i915, amdgpu, the DRM helper and Tegra
   stop their source side after the sink's clear) or `BeforeSink` (vc4, which splits the
   helper lineage) [29].
3. **Output off.**

With clearing off, only the output stops; the next bring-up sets the scrambler again.

- **Whether the sink is there** is the caller's argument, `SinkPresence`, on every call:
  `Present` writes, as i915, vc4 and Tegra do regardless of the connection; `Gone` writes
  nothing, as the helper and amdgpu do for a sink that is not connected [30].
- **Failures never keep the output running.** A failed sink write is a warning, as in every
  source that clears [29]. A failed PHY call is an error, but every later step is still
  attempted, so a failed scrambler call does not leave the output on; the result records
  each step's first error (`TearDownError`).
- **No waits:** the sources' waits during tear-down belong to AVMUTE and video, not to the
  sink's bits.

### The reconnect check

A sink may lose its `TMDS_Config` while the source keeps the link up: the helper lineage
says the sink's SCDC status "is reset and must be explicitly restored" on a reconnect with the
display active [32]. So after a hotplug while the link is up, the helper lineage checks on
every hotplug with the connector connected, dw-hdmi brings the link up again when the sink is
back, and Xilinx rewrites on a short HPD pulse [32][33][34]. Hotplug is the caller's;
`check_reconnect(link, current_sink)` is the check it calls, also usable at start-up on a link
found running (Amlogic writes `TMDS_Config` again then [35]).

- **The same sink first.** `current_sink` is the sink's facts from the EDID read after the
  hotplug, with any quirk the caller applied before applied again. When the link no longer
  fits them (`TmdsLinkParams::fits`: choosing again, with the source facts and policy the
  link was chosen with, would refuse the rate or give another ratio, scrambling or
  `TMDS_Config`), the result is `SinkChanged`, with nothing read. A fact the link does not
  depend on may change: a TV that toggles FRL in its EDID keeps its TMDS link. Facts that
  fit do not prove the same sink; telling sinks apart by their EDID is the caller's, before
  it calls the check.
- **One read, both bits compared** with `sink_config()`: i915 compares both, the helper and
  vc4 only `Scrambling_Enable`, since they always set the two together [32]. Every link is
  checked, a plain one too (its expected value is both bits 0, which a reset sink matches),
  unlike the helper lineage, which checks only a link that set something.
- **It writes nothing.** The result is `Matches`, `Lost` (with the bits and byte read),
  `Unreadable` (the read's error), `NotApplicable` (no write is due on this link) or
  `SinkChanged`. A failed read is reported, not acted on, as i915 and vc4 log it and the
  helper returns it [32].

**After `Lost`**, the way back is the caller's. The helper lineage does a full mode set and
dw-hdmi its whole setup; Xilinx rewrites with the stream running [32][33][34]. A majority does
the full bring-up, which in this crate is `tear_down` followed by `bring_up`; the quick restore
is the option.

### The quick restore

`quick_restore(link, config)` is Xilinx's way back [34], with the check added:

1. **`TMDS_Config` in one write**, with the output left running. There is no wait, because
   the output never stops: the sink switches under a clock that keeps running. After `Lost`
   on a 1/40 link the sink's ratio bit was reset, so this write switches the sink's clock
   ratio with TMDS running, as Xilinx and Amlogic do; the DRM helper, i915 and dw-hdmi
   avoid exactly that with a full mode set, their comments say [32][12], which is why the
   full bring-up is the default. On a 1/10 link, or a sink that kept its bits (a check that
   ended `NotSet`), nothing switches. `tmds_config_write` and `on_tmds_config_failure` apply
   as in bring-up; a write under `CarryOn` that fails is kept in `Restored::write_failed`.
2. **The source's scrambler set again** to the link's, as Xilinx does with the sink write
   [34] (Amlogic writes only the sink [35]): one PHY call that covers a source that lost its
   state with the sink. A plain link gets it too, set off.
3. **The check**, as in bring-up, from the scrambler, on a scrambled link.

It does not touch the PHY's rate or clock ratio, stops nothing and tears nothing down on an
error (`RestoreError`): the link was running, and what to do next is the caller's. If the
source side may have changed too, the full bring-up is the way back. It reads the cancel flag
only in its check: the write and the scrambler are one call each, and a restore stopped
between them would leave the sink and the source disagreeing. A cancelled check ends
`CheckResult::Cancelled`, with the link left running.

---

## Key Types

### Choosing the link

```rust
/// The TMDS bit clock ratio: the TMDS clock runs at the bit rate / 10 or / 40.
/// Exhaustive: `TMDS_Bit_Clock_Ratio` is one bit. hdmi-hal's type, so the link's ratio
/// passes straight to the PHY.
pub use hdmi_hal::phy::TmdsClockRatio as ClockRatio;   // { Div10, Div40 }

/// Which character rate first requires scrambling and 1/40 (rule 4).
#[non_exhaustive]
pub enum ScramblingThreshold {
    Above340,       // default: > 340 000 kHz
    AtOrAbove340,   // ≥ 340 000 kHz
}

/// What the sink declares (`new`, which sets `frl_capable` to `scdc_present`; `NONE` for a sink without HDMI
/// Forum capabilities, e.g. a DVI or HDMI 1.x sink; `with_frl_capable`). With the
/// `display-types` feature, `From<&HdmiForumSinkCap>` and `From<Option<&HdmiForumSinkCap>>`.
#[non_exhaustive]
pub struct SinkTmdsFacts {
    pub scdc_present: bool,
    pub scrambling_supported: bool,   // SCDC and a non-zero Max_TMDS_Character_Rate
    pub lte_340mcsc_scramble: bool,   // counts only with scrambling_supported
    pub frl_capable: bool,            // a non-zero Max_FRL_Rate: bring-up's FRL clear
}

/// What the source's PHY can do: `From<&TmdsPhyCapabilities>`, or `new(low_rate_scrambling)`
/// with a scrambler and no maximum (`u32::MAX`), and `with_scrambling`,
/// `with_max_character_rate_khz`.
#[non_exhaustive]
pub struct SourceTmdsFacts {
    pub scrambling: bool,
    pub low_rate_scrambling: bool,
    pub max_character_rate_khz: u32,
}

/// The caller's choices (rules 3, 4 and 6). `Default`/`DEFAULT`: low-rate scrambling on,
/// `Above340`, `AnyScdcSink`; a `with_…` setter for each.
#[non_exhaustive]
pub struct TmdsPolicy {
    pub low_rate_scrambling: bool,
    pub threshold: ScramblingThreshold,
    pub tmds_config_gate: TmdsConfigGate,
}

/// Which SCDC sinks get a `TMDS_Config` write on a plain link (rule 6).
#[non_exhaustive]
pub enum TmdsConfigGate { AnyScdcSink /* default */, ScramblingSink }

/// The two fields of `TMDS_Config` (0x20), what is written and what a read decodes to
/// (`new`). Its bit layout belongs to the SCDC implementation.
#[non_exhaustive]
pub struct SinkTmdsConfig { pub scrambling_enable: bool, pub high_tmds_clock_ratio: bool }

/// A read of `TMDS_Config`: the two fields and the byte as read (`new`), so reserved bits a
/// sink sets stay visible.
#[non_exhaustive]
pub struct TmdsConfigReading { pub config: SinkTmdsConfig, pub raw: u8 }

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
/// `sink_config() -> Option<SinkTmdsConfig>` (`None` exactly when no write is due),
/// `sink() -> SinkTmdsFacts`, `fits(SinkTmdsFacts) -> bool`. Equality compares the source
/// facts and policy it was chosen with too.
pub struct TmdsLinkParams { /* private */ }

impl TmdsLinkParams {
    pub const fn choose(
        rate_khz: u32,
        sink: SinkTmdsFacts,
        source: SourceTmdsFacts,
        policy: TmdsPolicy,
    ) -> Result<Self, ParamsError>;
}

/// `Display` and `core::error::Error`. Both variants are `#[non_exhaustive]` too.
#[non_exhaustive]
pub enum ParamsError {
    /// A rate above 340 000 kHz on a sink that cannot take scrambling or a source without
    /// a scrambler (rule 5); every missing fact is set.
    ScramblingUnsupported {
        rate_khz: u32,
        no_scdc: bool,
        no_scrambling_support: bool,
        source_cannot_scramble: bool,
    },
    /// A rate above the source's own maximum (rule 0).
    AboveSourceMaximum { rate_khz: u32, max_khz: u32 },
}
```

All are `Debug, Clone, Copy, PartialEq, Eq, Hash`; all constructors are `const fn`, so a
firmware can fix its policy and facts at compile time. `ScramblingStatus`, what the sink
reports in `Scrambling_Status` (`set`, with the byte read in `raw()`), is the procedures'
and is described with them.

- **Why private fields on `TmdsLinkParams`:** the PHY and the sink must agree. With public
  fields a caller could change the ratio and leave the sink's value stale; with getters the
  only way to a `TmdsLinkParams` is through the rules.
- **Why `ScramblingDecision`:** an unscrambled low-rate link is not a failure, but a
  caller expecting scrambling should see which of source, sink or policy declined it.
- **Why the sink facts are a struct of their own,** not `HdmiForumSinkCap` directly: the
  decision needs three flags, a caller can override a misbehaving sink without editing
  parsed EDID data, and the API is tied to display-types only through the `From` impls.

### The procedures' settings

```rust
/// Bring-up's and the quick restore's settings. `Default`/`DEFAULT`; a `with_…` setter
/// for each; the quick restore reads `tmds_config_write`, `on_tmds_config_failure` and
/// the check's two timings.
#[non_exhaustive]
pub struct BringUpConfig {
    pub ratio_wait: Duration,                          // 1 ms
    pub scrambling_deadline: Duration,                 // 100 ms
    pub check_window: Duration,                        // 200 ms
    pub check_interval: Duration,                      // 1 ms
    pub clear_frl: ClearFrl,                           // IfSinkDeclaresFrl
    pub source_version: SourceVersionWrite,            // EveryBringUp
    pub output_during_sink_write: OutputDuringSinkWrite, // Stopped
    pub tmds_config_write: TmdsConfigWrite,            // ReadModifyWrite
    pub on_tmds_config_failure: OnTmdsConfigFailure,   // CarryOn
    pub tear_down_on_error: bool,                      // true
    pub tear_down: TearDownConfig,                     // used after an error or a cancel
}
impl BringUpConfig {
    /// A deadline not longer than the wait, an interval not shorter than the window, or an
    /// interval of 0.
    pub const fn validate(&self) -> Result<(), ConfigError>;
}

#[non_exhaustive]
pub struct TearDownConfig {
    pub clear_sink: bool,                   // false
    pub sink_clear_timing: SinkClearTiming, // BeforeOutputStops
    pub scrambler_off: ScramblerOff,        // AfterSink
}

#[non_exhaustive] pub enum OutputDuringSinkWrite { Stopped, Running }
#[non_exhaustive] pub enum TmdsConfigWrite { ReadModifyWrite, WholeByte }
#[non_exhaustive] pub enum OnTmdsConfigFailure { CarryOn, Stop }
#[non_exhaustive] pub enum ClearFrl { IfSinkDeclaresFrl, EveryScdcSink }
#[non_exhaustive] pub enum SourceVersionWrite { EveryBringUp, IfSinkVersion1, WhenScrambling, Never }
#[non_exhaustive] pub enum SinkClearTiming { BeforeOutputStops, AfterOutputStops }
#[non_exhaustive] pub enum ScramblerOff { AfterSink, BeforeSink }
/// Tear-down's argument: whether the sink is still there. Exhaustive.
pub enum SinkPresence { Present, Gone }

/// Every problem `validate` found; `Display` and `core::error::Error`.
#[non_exhaustive]
pub struct ConfigError {
    pub deadline_always_missed: bool,
    pub check_reads_once: bool,
    pub check_interval_zero: bool,
}
```

All are `Copy`, every setter is a `const fn`, so firmware fixes its settings at compile time.

### The procedures' results

`S` is the SCDC error (`TmdsScdc::Error`), `P` the PHY error (`TmdsPhy::Error`).

```rust
#[non_exhaustive]
pub enum BringUpOutcome<S, P> {
    Up(LinkUp<S>),
    #[non_exhaustive]
    Cancelled { at: CancelPoint, cleanup: Cleanup<S, P>, warnings: Warnings<S> },
}

#[non_exhaustive]
pub struct LinkUp<S> {
    pub link: TmdsLinkParams,
    pub sink_written_at: Option<Timestamp>,   // None: no write was due
    pub output_started_at: Timestamp,
    pub check: ScramblingCheck<S>,            // never CheckResult::Cancelled
    pub warnings: Warnings<S>,
}

/// Each kind at most once per bring-up.
#[non_exhaustive]
pub enum BringUpWarning<S> {
    FrlRateClearFailed(S),
    FltUpdateClearFailed(UpdateError<S>),
    SinkVersionReadFailed(S),
    SourceVersionWriteFailed(S),
    TmdsConfigWriteFailed {
        error: UpdateError<S>,                               // Read: nothing was written
        read_back: Option<Result<TmdsConfigReading, S>>,     // a plain link's read-back
    },
    DeadlineMissed { by: Duration },
    ScramblingNotConfirmed { result: CheckResult },          // NotSet or Unreadable
}

/// Fixed storage for the warnings, in the order they came: `iter_warnings`, `len`, `is_empty`.
pub struct Warnings<S> { /* private */ }

#[non_exhaustive]
pub enum BringUpError<S, P> {
    Scdc { step: TmdsStep, error: S, output: OutputState, cleanup: Cleanup<S, P>, warnings: Warnings<S> },
    Phy { step: TmdsStep, error: P, output: OutputState, cleanup: Cleanup<S, P>, warnings: Warnings<S> },
}

#[non_exhaustive]
pub enum Cleanup<S, P> {
    TornDown(Result<TornDown<S>, TearDownError<S, P>>),
    Skipped,   // tear_down_on_error off, or a cancel seen at entry
}

pub enum OutputState { Stopped, MayBeRunning }   // exhaustive

#[non_exhaustive]
pub struct ScramblingCheck<S> {
    pub result: CheckResult,
    pub reads: u32,
    pub failed_reads: u32,
    pub first_error: Option<FailedRead<S>>,   // { error, after }
    pub set_after: Option<Duration>,          // from the check's start
    pub read_cap_reached: bool,
}
#[non_exhaustive]
pub enum CheckResult { NotApplicable, Set, NotSet, Unreadable, Cancelled }

#[non_exhaustive] pub struct TornDown<S> { pub sink: SinkClear<S> }
/// At least one PHY step failed; each field holds that step's error.
#[non_exhaustive]
pub struct TearDownError<S, P> { pub sink: SinkClear<S>, pub scrambler: Option<P>, pub output: Option<P> }
#[non_exhaustive]
pub enum SinkClear<S> { NotRequested, NothingToClear, SkippedSinkGone, Cleared, Failed(S) }

#[non_exhaustive]
pub enum ReconnectCheck<S> {
    Matches, Lost { read: TmdsConfigReading }, Unreadable(S), NotApplicable, SinkChanged,
}

#[non_exhaustive]
pub struct Restored<S> {
    pub sink_written_at: Option<Timestamp>,
    pub check: ScramblingCheck<S>,
    pub write_failed: Option<UpdateError<S>>,
}
#[non_exhaustive]
pub enum RestoreError<S, P> {
    Scdc(UpdateError<S>),
    Phy { error: P, sink_written_at: Option<Timestamp>, write_failed: Option<UpdateError<S>> },
}
```

All are `Debug, Clone, Copy, PartialEq, Eq` where `S` and `P` are. `BringUpError`,
`TearDownError` and `RestoreError` implement `Display` and `core::error::Error`, with the
SCDC error as the source (a PHY error, which is only `Debug`, has none). The warnings live in fixed
storage, one slot per kind, so an outcome needs no `alloc`.

- **Why `Cancelled` is an outcome, not an error:** the caller asked for it, and the link
  was torn down as asked.
- **Why the check is part of `LinkUp`, not a condition of it:** the link is up and running
  whatever the sink reports; the result tells the caller whether to repeat, and how soon.
- **Why the errors carry the warnings and the cleanup:** a caller that only sees the error
  still learns everything that happened before it.

### The SCDC trait

```rust
pub trait TmdsScdc {
    type Error: hdmi_hal::ddc::DdcError;
    fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), Self::Error>;
    fn update_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), UpdateError<Self::Error>>;
    fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, Self::Error>;
    fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, Self::Error>;
    fn read_sink_version(&mut self) -> Result<u8, Self::Error>;
    fn write_source_version(&mut self, version: u8) -> Result<(), Self::Error>;
    fn clear_frl_rate(&mut self) -> Result<(), Self::Error>;
    fn clear_flt_update(&mut self) -> Result<bool, UpdateError<Self::Error>>;   // whether it was set
}

/// An operation of a read and a write, and which failed: exhaustive, as it has two halves.
/// `Display`, `core::error::Error`, and `DdcError` with the failed half's kind.
pub enum UpdateError<E> { Read(E), Write(E) }

/// What the sink reports in `Scrambling_Status` (0x21), with the byte read (`raw()`).
#[non_exhaustive]
pub struct ScramblingStatus { pub set: bool /* , raw: private */ }
```

The sink-side operations the procedures make, and nothing more:

- **`write_tmds_config`** writes the whole byte, the two bits as given and the others 0;
  **`update_tmds_config`** reads `TMDS_Config`, sets the two bits and writes the byte back
  with the others as read, writing nothing after a failed read. The bit layout is the
  implementation's, so the read-modify-write is too.
- **`clear_frl_rate`** writes `Config_1` (0x31) as 0, `FRL_Rate` and `FFE_Levels` both, as
  Amlogic does; amdgpu reads it first and writes only when `FRL_Rate` is not 0 [36]. The
  setting has already decided the clear is wanted, so the read would save nothing.
- **`clear_flt_update`** reads `Update_0` (0x10) and, when `FLT_update` is set, writes it
  back as 1, which clears it, as both sources do [36]. It writes that flag alone, as amdgpu
  does; Amlogic writes back every flag it read. Chosen, because the other flags belong to
  whoever reads them next, such as a link health monitor. Two methods, so the bring-up can say which
  register's clear failed.
- **The error is a `DdcError`**, whose kind tells a failure that will not pass (`Detached`,
  `Unsupported`) from one that may; the procedures poll through the second and stop on the
  first. An implementation over an `ScdcTransport` passes its errors through.

It is the crate's own trait, not culvert's client, so a change to it touches only this crate,
and a firmware with its own SCDC code, a kernel driver whose platform already has SCDC
helpers, or a test double implements eight methods rather than a whole register map.

**The culvert adapter** (`culvert` feature): `TmdsScdc` for `culvert::Scdc<T>` over any
`ScdcTransport`. Its error, `CulvertError<E>`, wraps culvert's `ScdcError<E>`: its kind is
the transport's own for a transport error, and `Unsupported` for a request culvert refuses,
which would be refused again. `update_tmds_config` uses culvert's own read-modify-write, and
the bytes read reach `TmdsConfigReading::raw` and `ScramblingStatus::raw`.

---

## Diagnostics

Each procedure has a plain form and a `_with_events` form that calls `FnMut(TmdsEvent)` with
every step (`bring_up` and `bring_up_with_events`, and so on). A callback needs no buffer, so
nothing overflows and the API is the same in every build; a caller that wants a list pushes
each event into its own. There is no logging.

```rust
#[non_exhaustive]
pub struct TmdsEvent {
    pub at: Timestamp,          // from the procedure's own clock
    pub kind: TmdsEventKind,    // #[non_exhaustive], as is each variant with fields
}
```

- **Stamped by the procedure,** from the clock it already holds: the timing is the point
  (the write, the wait, output start, the check).
- **One kind per step and per decision**, with the values written or read: the start, with
  the link and the settings in force; each step done (the FRL clear, `Sink_Version` read,
  `Source_Version` written, output stopped, `TMDS_Config` written or read, the PHY
  configured, the scrambler set, the wait spent, output started); each step skipped, with
  the rule that skipped it (`FrlClearSkip`, `SourceVersionSkip`, `TmdsConfigSkip`,
  `SinkClearSkip`); each step failed, with its `TmdsStep`, whether it ends the procedure, and
  for SCDC the error's `DdcErrorKind`; a missed deadline; the check, **one event per window**
  with what `ScramblingCheck` records; a cancellation, with its `CancelPoint`; and the end.
- **No error values,** only their kinds: the values stay in the outcome or the error, which
  own them, so neither error type has to be `Clone`, and `TmdsEvent` is `Copy` with no type
  parameters, so an embedded caller can send it through a channel.
- **Events explain outcomes:** every warning and every error is also an event, with its
  step, so a tool that only listens can explain every result. A failed read-modify-write of
  `TMDS_Config` names its half by its step (`ReadTmdsConfig` or `WriteTmdsConfig`); which
  half of the `FLT_update` clear failed is only in the warning's value, as are the error
  values themselves.

**A fixed maximum per procedure,** since every step records at most one event per call and
the check one per window: `MAX_BRING_UP_EVENTS` (which includes the tear-down after an error
or a cancel), `MAX_TEAR_DOWN_EVENTS`, `MAX_RECONNECT_CHECK_EVENTS` and
`MAX_QUICK_RESTORE_EVENTS`. A caller without `alloc` sizes its channel or buffer from them and
never drops an event.

---

## Interface Boundaries

### The core and the sync driver

```rust
// spiritlevel::procedure: the core, written once as async fns
#[allow(async_fn_in_trait)]
pub trait TmdsIo {
    type ScdcError: hdmi_hal::ddc::DdcError;
    type PhyError: core::fmt::Debug;
    // `TmdsScdc`'s eight methods and `TmdsPhy`'s four I/O methods, as `async fn`
    fn now(&self) -> Timestamp;
    async fn delay(&mut self, duration: Duration);
    fn cancelled(&self) -> bool;
}
pub async fn bring_up<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io, link: &TmdsLinkParams, video: TmdsVideo, config: &BringUpConfig, record: &mut F,
) -> Result<BringUpOutcome<Io::ScdcError, Io::PhyError>, BringUpError<Io::ScdcError, Io::PhyError>>;
// tear_down(io, link, config, presence, record), check_reconnect(io, link, current_sink, record),
// quick_restore(io, link, config, record)

// spiritlevel: the sync driver
pub struct TmdsLink<S, P, K, D, X = NoCancel> { /* the five parts */ }
impl<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay> TmdsLink<S, P, K, D, NoCancel> {
    pub fn new(scdc: S, phy: P, clock: K, delay: D) -> Self;
}
impl<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay, X: Cancel> TmdsLink<S, P, K, D, X> {
    pub fn with_cancel<X2: Cancel>(self, cancel: X2) -> TmdsLink<S, P, K, D, X2>;
    pub fn bring_up(&mut self, link: &TmdsLinkParams, video: TmdsVideo, config: &BringUpConfig) -> …;
    // bring_up_with_events, tear_down(_with_events), check_reconnect(_with_events),
    // quick_restore(_with_events)
    pub fn into_parts(self) -> (S, P, K, D, X);
    // scdc, scdc_mut, phy, phy_mut, clock: the parts between procedures
}

pub use hdmi_hal::cancel::{Cancel, NoCancel};   // for AtomicBool, &T and NoCancel
```

- **`TmdsIo` flattens the I/O** into one trait, so the core sees one bound and a test drives
  it through one double; a driver implements it over its parts. `TmdsLink` is that driver for
  sync parts: their calls complete at once, so it polls the core's future to completion
  without a runtime, and the `Delay` blocks inside it. An async driver awaits the same
  futures.
- **`Cancel` is a trait,** not a bare `&AtomicBool`, so the driver needs no lifetime unless
  it is given a flag, and a platform with its own primitive implements one method.
  `NoCancel`, the default, never cancels. It is hdmi-hal's, so one flag type serves every
  procedure in the stack, FRL training included.
- **One value can be both clock and delay:** implement `Clock` and `Delay` for a reference
  to it (`&Time`, keeping the time in a `Cell`) and pass the reference twice.

### Below: hdmi-hal and the SCDC implementation

From [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) the procedures use:

| Trait | What for |
|---|---|
| `TmdsPhy` | `stop_output` (bring-up step 1, tear-down), `configure` (rate, ratio, video) and `set_scrambling` (step 3, tear-down, the quick restore), `start_output` (step 5); `capabilities`, through `SourceTmdsFacts::from`, for `choose` |
| `Clock` | `now()`: the write time, the deadline, the check's window, the events |
| `Delay` | the ratio wait and the check's interval, "at least" the duration given |

Start and stop are separate calls, so the output stays off while the sink is written, and
`configure` sets rate, ratio and video without starting output. Powering the PHY on and off
is hdmi-hal's `PhyPower`, which the caller calls, not the procedures: on unplug, after a
tear-down with `SinkPresence::Gone`.

The SCDC side is `TmdsScdc` ("The SCDC trait"), implemented by the caller or by culvert's
adapter.

### Above: the caller

The caller owns hotplug, the EDID and negotiation, and retries. How a caller maps its events
onto the procedures:

| When | Calls |
|---|---|
| A TMDS mode is to be shown | `choose` (sink facts from the EDID, source facts from `TmdsPhy::capabilities`, the policy), then `bring_up` |
| Before a mode change | `tear_down` with the sink present |
| Before FRL training, from a TMDS link | `tear_down` with `clear_sink` ("Tear-down") |
| On unplug | `tear_down` with `SinkPresence::Gone`, then power the PHY off |
| A hotplug while the link is up | read the EDID again; for the same sink, `check_reconnect` with its current facts; on `Lost`, `tear_down` and `bring_up`, or `quick_restore` |
| The check ended `NotSet` | `quick_restore` again later |

A DRM driver maps them onto its hooks the same way: atomic enable to `bring_up`, disable to
`tear_down`, its hotplug handler to `check_reconnect`; `tear_down_on_error` keeps a failed
enable, which the hook cannot report, from leaving a half-started output.

---

## `no_std`, `alloc`, and `async`

`#![no_std]` without `alloc` in every build, and `#![forbid(unsafe_code)]`. `choose` is a
pure `const fn`. The procedures are `async fn`s over `TmdsIo`, and `TmdsLink` polls them to
completion with a no-op waker, without a runtime; their outcomes use fixed storage and their
events a callback, so nothing needs `alloc`. CI builds the crate for a bare target
(`thumbv7em-none-eabi`) with and without default features and with `culvert`, and checks the
MSRV (Rust 1.85).

An async driver over hdmi-hal-async's traits, with futures that can be `Send`, is planned
as a companion crate; it drives the same core. Whatever drives it, a procedure is stopped
through its cancel flag, not by dropping its future (with `select`, a timeout or an
aborted task): a dropped call leaves the PHY and the sink in an unknown state, and the
caller loses the tear-down that `Cancelled` carries.

---

## Testing

**Choosing the link.** The input space is small, so the tests cover it whole:

- **Every combination:** both thresholds × the sink facts × the source facts × the policy,
  at rates either side of and at 340 000 kHz, from 0 to `u32::MAX`; each case checked
  against the rules, including the `ScramblingDecision` and `ParamsError`.
- **The boundary:** 340 000 kHz gives 1/10 with `Above340`; with `AtOrAbove340` it gives
  1/40 when both ends can scramble and the plain 1/10 link, with the reason, when one
  cannot; 340 001 kHz gives 1/40, or `ScramblingUnsupported`.
- **The gate on `LTE_340Mcsc_scramble`:** a sink that declares it without scrambling
  support gets no low-rate scrambling.
- **Every reason:** each `ScramblingOff` flag and each `ScramblingUnsupported` flag is set
  exactly when its own fact blocks scrambling, whatever the others are.
- **Invariants:** `sink_config()` is `None` exactly when no write is due (no SCDC, or a
  plain link the gate leaves unwritten), and otherwise matches `clock_ratio()` and
  `scrambling()`; `scrambling()` is true exactly for `Required` and `LowRate`;
  `rate_khz()` returns the input.
- **`const` use:** tests evaluate `choose` and the constructors in `const` items, so a
  change that breaks `const fn` fails the build.
- **`From` impls:** every combination of the HF-SCDB fields against a literal table of
  expected facts, and `None` maps to `SinkTmdsFacts::NONE`; the PHY's capabilities map to
  the source facts.

**The procedures** run against a double of `TmdsIo` that records every call, answers from a
few registers, fails what a test tells it to, and advances its clock on every read and
delay:

- **Each step and its order,** for both bring-up orders, plain and scrambled links, both
  write forms, and every setting's options.
- **Every failure:** each SCDC step failing with a transient and a non-transient error, each
  PHY call failing, and what each leaves (the warning, the error, the output's state, the
  cleanup).
- **The check:** a sink that reports at once, late or never; failed reads; a gone bus; a
  clock that does not advance, ended by the read count; a zero window.
- **Cancellation:** a sweep that sets the flag after every operation of a full bring-up and
  checks where it is seen and that the tear-down follows.
- **The event bounds:** every combination of link, sink, order, write form and failure,
  checked against `MAX_BRING_UP_EVENTS`, and each other procedure against its own.
- **The sync driver** over simple parts, and the culvert adapter over a register-level
  `ScdcTransport`.
- **The README's examples** run as doctests.

CI measures line coverage and fails if it drops more than 0.1% below the recorded
baseline.

---

## Design Principles

- **Procedure, not policy.** Where HDMI sources differ, the behaviour is a setting with the
  majority as default (rules 3 and 4); the defaults are documented and changeable.
- **Every reason, not the first.** A decision or an error carries every condition that
  applies, each judged on its own fact, so a diagnostic tool's user sees all of them at
  once instead of fixing one and meeting the next. They fit in a `Copy` value without
  `alloc`.
- **Cited behaviour.** Each rule cites the public implementations it follows; where
  `spiritlevel` departs from them, the rule says so and why.
- **The output stays consistent.** The PHY's settings and the sink's register come from one
  value that only the rules can build.
- **Stable output types.** Output structs and enums are `#[non_exhaustive]`, so fields and
  reasons can be added without breaking callers; `ClockRatio`, a single register bit, is
  exhaustive.
- **Interfaces, not implementations.** The procedures run over traits: hdmi-hal's for the
  PHY, clock and delay, and the crate's own `TmdsScdc`; culvert's adapter and
  display-types' conversions are optional conveniences.

---

## What comes next

An async companion crate, testing against a simulated sink, and what is under
consideration are on the [roadmap](roadmap.md).

---

## Sources

Driver source read at these commits; line numbers refer to them.

- Linux, github.com/torvalds/linux `e9954f3fbddd` (2026-10-31)
- Xilinx embeddedsw, github.com/Xilinx/embeddedsw `97f2baf7f69e` (2026-06-24)
- Amlogic, github.com/CoreELEC/common_drivers `8ca9b6c3b715` (2026-10-31)

|      | Source                              | Location                                                                                                                                                                                 |
|------|-------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| [1]  | Linux DRM helper, vc4 (one lineage) | `drivers/gpu/drm/display/drm_hdmi_helper.c:460` (threshold), `476–482`; `drivers/gpu/drm/display/drm_hdmi_state_helper.c:600` (threshold); `drivers/gpu/drm/vc4/vc4_hdmi.c:134–141`, `815–816` |
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
| [12] | dw-hdmi family (dw-hdmi, Meson and Rockchip glue) | `drivers/gpu/drm/bridge/synopsys/dw-hdmi.c:1406–1413` (comment: 1 to 100 ms from the ratio write until transmission resumes), `1646–1675` (PHY off, ratio write, 100 ms above 340 Mcsc, PHY on); `meson/meson_dw_hdmi.c:399–403`; `rockchip/dw_hdmi-rockchip.c:352–354` |
| [13] | i915, enable path                   | `drivers/gpu/drm/i915/display/intel_ddi.c:3440–3445` (sink write), `3528` (port buffers enabled)                                                                                          |
| [14] | amdgpu, TMDS enable                 | `drivers/gpu/drm/amd/display/dc/link/link_dpms.c:1908–1912` (sink write), `1931–1940` (output enabled), `1942–1943`; `link/protocols/link_ddc.c:602–613` (one byte, `3` above 340 MHz)    |
| [15] | MediaTek v2, enable path            | `drivers/gpu/drm/mediatek/mtk_hdmi_v2.c:87–101`, `746` (scrambler, then sink), `1039` → `1045` (`phy_power_on` after)                                                                    |
| [16] | Xilinx, stream start                | `v_hdmitxss1/src/xv_hdmitxss1.c:2213–2217`; `v_hdmitx1/src/xv_hdmitx1.c:633–638`; pass-through example `v_hdmitxss1/examples/xhdmi_example/PassThrough/xhdmi_exdes_sm_tx.c:601`, `2118–2121` (TMDS clock off), `2531–2545` (stream start, then clock on) |
| [17] | Amlogic, mode set                   | `drivers/media/vout/hdmitx_common/hdmitx_mode_set.c:457–460` (PHY off); `hdmitx21/hw/hdmi_tx_hw.c:1475` → `3628–3631` → `2889–2904` (one byte, then source), `1578` (PHY on)               |
| [18] | vc4, Tegra (output running)         | `drivers/gpu/drm/vc4/vc4_hdmi.c:1690–1692`, `815–820`; `drivers/gpu/drm/tegra/sor.c:2606`, `2631`, `2658`, `2177–2180`                                                                  |
| [19] | i915, scrambling check              | `drivers/gpu/drm/i915/display/intel_hdmi.c:2749–2754` (1 ms polls for 200 ms, logged only), `2767–2770` (comment: 100 ms)                                                                |
| [20] | Plain link: `TMDS_Config` written at every bring-up | i915 `intel_hdmi.c:2794–2795`; amdgpu `link_ddc.c:602–613`; dw-hdmi `dw-hdmi.c:1428–1430`, `2131–2134`, `2303–2306`; nouveau `disp.c:800–812`; MediaTek v2 `mtk_hdmi_v2.c:87–101`, `746`; Xilinx `xv_hdmitxss1.c:2213–2217`, `xv_hdmitx1.c:663–668`, `737–744`; Amlogic `hdmi_tx_hw.c:2889–2904` |
| [21] | Low-rate scrambling check           | i915 `intel_hdmi.c:2433–2435`, `2745–2754`; amdgpu `link_ddc.c:629–643`                                                                                                                 |
| [22] | amdgpu, before `TMDS_Config`        | `drivers/gpu/drm/amd/display/dc/link/protocols/link_ddc.c:585–588` (SCDC gate), `589–590` (FRL cleared), `592–600` (`Source_Version` when `Sink_Version` is 1), `602` (`TMDS_Config`)    |
| [23] | Amlogic, clearing FRL               | `drivers/media/vout/hdmitx21/hdmi_tx_main.c:466–488` (only with FRL and SCDC declared)                                                                                                    |
| [24] | Helper lineage, Tegra: tear-down only | `drivers/gpu/drm/vc4/vc4_hdmi.c:807–810`, `857–858`, `922`; `drivers/gpu/drm/tegra/sor.c:2149–2155`, `2202`, `2218`                                                                     |
| [25] | Which sinks get the write           | any SCDC sink: amdgpu `link_ddc.c:585–588`, MediaTek v2 `mtk_hdmi_v2.c:96–99`, Xilinx `xv_hdmitx1.c:789–806`; scrambling support: i915 `intel_hdmi.c:2785–2786`, dw-hdmi `dw-hdmi.c:1380–1404`, nouveau `disp.c:796`; every sink: Amlogic `hdmi_tx_hw.c:2893–2901` |
| [26] | A failed `TMDS_Config` write        | logged, then continued: i915 `intel_ddi.c:3440–3445`, dw-hdmi-qp `dw-hdmi-qp.c:558–560`, nouveau `disp.c:812–815`; result unused: amdgpu `link_ddc.c:611`, Xilinx `xv_hdmitxss1.c:2214–2217`, vc4 `vc4_hdmi.c:815–816`, Tegra `sor.c:2177–2178`, MediaTek v2 `mtk_hdmi_v2.c:98–99`, dw-hdmi `dw-hdmi.c:1428–1430`; no result: Amlogic `hdmi_tx_hw.c:2780–2798` |
| [27] | A failed FRL clear                  | amdgpu `drivers/gpu/drm/amd/display/dc/link/protocols/link_hdmi_frl.h:30`, `32` (`void`), called at `link_ddc.c:589–590`; Amlogic `drivers/media/vout/hdmitx21/hdmi_tx_main.c:484–487` (results unused) |
| [28] | Tear-down without clearing          | dw-hdmi `dw-hdmi.c:2408–2416`, `2979–2990`; nouveau `disp.c:800–812` (enable only); MediaTek v2 `mtk_hdmi_v2.c:1071–1104`; Xilinx `xhdmi_exdes_sm_tx.c:2020–2035`, `2620–2636`; Amlogic `hdmitx_common/hdmitx_mode_set.c:785–792`, `hdmitx21/hdmi_tx_main.c:4199–4222` |
| [29] | Tear-down clearing the sink, and its order | i915 `intel_ddi.c:3614–3626` (in `disable`, logged), `3230` (buffers off in `post_disable`), `3281` (the scrambling transcoder off, after the sink), `5345–5347`; amdgpu `link_dpms.c:2442`, `2460–2465`, `2487–2491`; helper `drm_hdmi_helper.c:602–630`; vc4 `vc4_hdmi.c:831–860`, `908–922`; dw-hdmi-qp `dw-hdmi-qp.c:599–601`; Tegra `sor.c:2149–2162`, `2218–2220` |
| [30] | Tear-down with the sink gone        | helper `drm_hdmi_helper.c:622–624` (connected only); amdgpu `link_ddc.c:582–583` (only with a sink); vc4 `vc4_hdmi.c:831–860`, Tegra `sor.c:2149–2162` (regardless)                       |
| [31] | Amlogic, clearing on suspend (quirk) | `hdmitx21/hdmi_tx_main.c:331`, `353–361`; `hdmitx_common/hdmitx_compliance.c:21–25`, `63–76`                                                                                       |
| [32] | Helper lineage, reconnect check     | i915 `intel_ddi.c:4824`, `4847–4869` (conditions, read, both bits), `4858–4862` (failed read logged), `4871–4880` (full mode set), `4908–4915` (on hotplug); helper `drm_hdmi_helper.c:632–731` (`drm_connector_hdmi_sync_scdc`; comment 640–641, check 677–709, reset 712–726); vc4 `vc4_hdmi.c:320–354`, `376–392` |
| [33] | dw-hdmi, reconnect                  | `drivers/gpu/drm/bridge/synopsys/dw-hdmi.c:3084–3109` (HPD and RX sense), `2397–2438` (PHY off, then the whole setup)                                                                  |
| [34] | Xilinx, HPD toggle                  | pass-through example `xhdmi_exdes_sm_tx.c:1394–1403` (`StreamStart` on a toggle); `xv_hdmitxss1.c:2213–2217`; toggle timing `xv_hdmitx1.c:205–224`                                     |
| [35] | Amlogic, restores without reading   | `drivers/media/vout/hdmitx21/hdmi_tx_main.c:5053–5062` (start-up with HPD high), `4744–4796` (stalled clock); `hw/hdmi_tx_hw.c:2972–2985` (the sink write only)                                                                             |
| [36] | The FRL clear's registers          | amdgpu `drivers/gpu/drm/amd/display/dc/link/protocols/link_hdmi_frl.c:178–200` (`FLT_update`), `394–415` (`Config_1`); Amlogic `drivers/media/vout/hdmitx21/hdmi_tx_main.c:484–487` |
| [37] | Linux's SCDC helper                 | `drivers/gpu/drm/display/drm_scdc_helper.c:221–245`, `277–307` (read-modify-write; nothing written after a failed read), `300–305` (1–2 ms after every ratio write; the comment gives 1 to 100 ms); its callers `intel_hdmi.c:2794–2795`, `vc4_hdmi.c:815–816`, `drm_hdmi_helper.c:477–481`, `dw-hdmi.c:1428–1430`, `sor.c:2177–2178`, `mtk_hdmi_v2.c:98–99`; nouveau `disp.c:800–812` and Xilinx `xv_hdmitx1.c:643–680`, 715–757 (read-modify-write in their own code) |
| [38] | A failed `TMDS_Config` write, dissent | DRM helper `drivers/gpu/drm/display/drm_hdmi_helper.c:576–586` (no source scrambler, both sink bits cleared); nouveau `drivers/gpu/drm/nouveau/dispnv50/disp.c:800–804` (returns on a failed read) |
| [39] | When `Source_Version` is written    | amdgpu `link_ddc.c:592–600` (when `Sink_Version` is 1); helper `drm_hdmi_helper.c:544`, `564`, `drm_scdc_helper.c:322–350` (scrambled links, only to a sink reporting a version); dw-hdmi `dw-hdmi.c:2112–2115` (scrambled links, its version, 0 included); Xilinx `xv_hdmitx1.c:789–806` (unconditionally, at detection); Amlogic `hdmitx21/hdmi_tx_main.c:3859–3867` (FRL path only) |
