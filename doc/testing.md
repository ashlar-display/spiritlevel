# Testing strategy

spiritlevel's core is a pure decision, `TmdsLinkParams::choose`: no I/O, no clock, no
allocator. Its tests are unit tests that call it over the inputs that matter and check the
whole result, including every reason it reports.

## Test structure

Unit tests live in a `#[cfg(test)]` module at the end of each file:

| File | What the tests check |
|---|---|
| `src/params.rs` | `choose` over every combination of the sink, source and policy facts, with the result checked for consistency; the 340 MHz threshold at, just above and below it, under both `ScramblingThreshold` settings (`Above340`, `AtOrAbove340`); that `LTE_340Mcsc_scramble` without scrambling support gives no low-rate scrambling; that each reason is set exactly when its fact blocks; the sink's `TMDS_Config` for each case (DVI sink, SCDC sink below the threshold, low-rate scrambling); an error exactly above 340 MHz on a sink that cannot scramble; `choose` usable in `const` contexts |
| `src/types.rs` | The fact and policy types: constructors keep their fields in order and are `const`, `NONE` matches `new(false, false, false)`, `DEFAULT` matches `Default` (low-rate scrambling on, `Above340`), the policy builders change one field each; with `display-types`, the conversion from the HF-SCDB (an absent block gives `NONE`) |
| `src/error.rs` | `ParamsError`'s `Display` names every missing fact, and reads well without one |

## Against a simulated sink (`tests/simulated_sink.rs`)

With the `culvert` feature, the procedures run through culvert's client, over hdmi-hal's
`ScdcOverDdc` on [maquette](https://docs.rs/maquette)'s simulated DDC bus, with its
simulated PHY and its time as clock and delay. The sink keeps its scrambling lock, its
`TMDS_Config` and its HPD resets as the public receivers do; refusals, delays and faults
are set per test.

**Covered:**
- A bring-up at 594 Mcsc locks the sink, and a tear-down clears it; a plain 1/10 link
  writes `TMDS_Config` 0 and checks nothing; low-rate scrambling at 1/10 locks
- A slow sink locks within the check window, and a slower one does not
- The quick restore: a sink that withholds the status once locks on it; one that resyncs
  on the rewrite is waited out; a sink that drops `Scrambling_Enable` is found lost and
  restored
- HPD: by default any pulse loses the configuration; a sink that resets only after 100 ms
  of HPD low keeps it over a short pulse and loses it over a long one
- A strict sink locks when the scrambler follows the write, but not on a quick restore with
  scrambled output already running; a tear-down and a bring-up recover
- Faults: a NACKed `TMDS_Config` write carries on with a warning; a detached one ends the
  bring-up with a tear-down; a failed `start_output` ends it with a PHY error

## Doctests

The README's example runs as a doctest (`ReadmeDoctests` in `src/lib.rs`) and asserts the
link it chooses, so it stays true to the API.

## Builds

CI runs the tests with the default features and with no default features (without the
display-types conversions), on stable and on the MSRV. It builds both for
`thumbv7em-none-eabi`, which has no `std`, on stable, and the no-default build on the MSRV.

## Coverage

Coverage is measured with `cargo-llvm-cov` and kept from falling by a ratchet: CI fails
when line coverage drops more than 0.1 percentage points below `.coverage-baseline`, and
opens a PR raising the baseline when a push improves it. See
[setup.md](setup.md#measuring-coverage) for the commands.

## Philosophy

- **Every reason, not the first.** A decision reports every condition that applies, so
  the tests check each reason against the fact that sets it, not only the outcome.
- **Exhaustive where the input space is small.** The facts are a handful of booleans;
  testing every combination is cheaper than arguing which ones matter.
