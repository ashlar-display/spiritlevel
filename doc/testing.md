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
| `src/config.rs` | The settings: the defaults are the documented ones, each setter changes its field alone, `validate` reports every problem at once, and `SinkPresence` is exhaustive |
| `src/procedure/bring_up.rs` | Bring-up over the test double: the default order for a scrambled link, `OutputDuringSinkWrite::Running` running unscrambled output through the write, and a plain link clearing the sink with no deadline; whole-byte writes without reading, and no write where none is due; an FRL sink cleared and given the source version by default, each following its setting, and a sink without SCDC given nothing; a video that does not give the link's rate refused before any I/O; a failure in the work before both paths (the FRL clear and `Source_Version`) a warning, transient or not, with the work going on, a kind added later counted as transient, and those warnings kept when the write later stops; the wait counted from the write; a failed write carrying on or, under `Stop`, tearing down, with no writes to a gone sink in the cleanup; a read-modify-write that fails at the write saying so; a check that sees no scrambling, and a late start, as warnings; a PHY failure saying where and whether output may run; `tear_down_on_error` off leaving both ends; the cancel flag read before each step of that work and before a plain link's read-back, ending the procedure before any I/O when set at entry and tearing down at the next point otherwise; a zero interval spending the wait in one delay; no combination of settings recording more events than the bound |
| `src/procedure/check.rs` | The scrambling check: a sink that reports at once is set after one read, one that reports late when it does, one that never does is not set after the window; failed reads do not end it, every read failing is `Unreadable`, and a detached bus or `Unsupported` ends it at once; a frozen clock ends it on its read count, a delay that runs late ends it, and no read starts after the window; a zero window reads once; the read cap follows window and interval; the cancel flag is read before every read |
| `src/procedure/reconnect.rs` | The reconnect check: a sink that kept its configuration matches; a reset sink lost it, with the bits read kept; both bits are compared, and a plain link expects both clear; a failed read is reported, not acted on; changed facts or a sink without SCDC read nothing |
| `src/procedure/restore.rs` | The quick restore: a scrambled link written, set and checked with the output running; a plain link written and not checked; no write where none is due; a failed write carrying on or stopping; a failed scrambler call an error, with the write; only the check reading the cancel flag |
| `src/procedure/tear_down.rs` | Tear-down: by default only the output stops; clearing follows the order the settings give; nothing is written to a plain link, a sink without SCDC or a gone sink; a failed sink write is a warning, and a failed PHY step does not keep the output running; a low-rate scrambled link is cleared too; events are stamped from the I/O's clock; no combination of settings records more events than the bound |
| `src/procedure/test_io.rs` | The test double the procedure tests run against: writes reach its registers and calls cost simulated time; it records each call and answers as set |
| `src/scdc.rs` | `TmdsScdc`: a register-level implementation does what each method says, a borrowed implementation forwards every call, and `UpdateError` tells the read's failure from the write's |
| `src/culvert.rs` | With `culvert`, the adapter for culvert's client: every method goes through culvert, errors carry the transport's kind and say which transfer failed, a protocol error is `Unsupported`, the transport reads blocks as a sink would, and `CulvertError` prints, compares and chains as culvert's error |
| `src/link.rs` | `TmdsLink`: bring-up, tear-down, the reconnect check and the quick restore run over its parts, which are reachable between procedures; the plain form and the one with a cancel flag; its synchronous I/O passes every call through |
| `src/outcome.rs` | The result types: errors print and chain, warnings keep their order and never overflow, a plain link's check read nothing, and the kinds keep the result and drop the value |
| `src/event.rs` | Events are `Copy` and carry no error values |

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
