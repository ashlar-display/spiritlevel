//! The procedures — bring-up, tear-down, the reconnect check and the quick restore — through
//! culvert's SCDC client, against maquette's simulated sink, PHY and time.
#![cfg(feature = "culvert")]

use core::time::Duration;
use hdmi_hal::scdc::ScdcOverDdc;
use maquette::hdmi_hal::ddc::SCDC;

use maquette::{
    Bench, Ddc, DdcErrorKind, Effect, Env, Event, Fault, Filter, HookWrite, HpdReset, Line,
    NoAcknowledgeSource, OpKind, Phy, PhyCall, PhyCallKind, PhyFault, Refusal, RefusalKind,
    RegisterGroup, ScdcHook, ScdcRegisters, SinkConfig, Time, TmdsClockRatio, Trigger,
};
use spiritlevel::{
    BringUpConfig, BringUpError, BringUpOutcome, BringUpWarning, CheckResult, ColorBitDepth,
    ColorFormat, LinkUp, OutputDuringSinkWrite, ReconnectCheck, SinkClear, SinkPresence,
    SinkTmdsFacts, SourceTmdsFacts, TearDownConfig, TmdsLink, TmdsLinkParams, TmdsPhy, TmdsPolicy,
    TmdsVideo,
};

/// The link spiritlevel drives: culvert's client over hdmi-hal's `ScdcOverDdc` over the bench's
/// bus, the bench's PHY, and its time handle as clock and delay.
type Link<'b> = TmdsLink<culvert::Scdc<ScdcOverDdc<Ddc<'b>>>, Phy<'b>, Time<'b>, Time<'b>>;

fn link<'b>(bench: &'b Bench<'_>) -> Link<'b> {
    let scdc =
        culvert::Scdc::new(ScdcOverDdc::new(bench.ddc()).expect("the bench's bus serves SCDC"));
    TmdsLink::new(scdc, bench.phy(), bench.time(), bench.time())
}

/// The sink facts these tests use: SCDC, scrambling, and low-rate scrambling or not.
fn sink_facts(low_rate_scrambling: bool) -> SinkTmdsFacts {
    SinkTmdsFacts::new(true, true, low_rate_scrambling)
}

/// The video at `pixel_clock_khz`, RGB 8-bit, and the link spiritlevel chooses for it on
/// `sink`, from the bench PHY's capabilities.
fn video(
    bench: &Bench<'_>,
    pixel_clock_khz: u32,
    sink: SinkTmdsFacts,
) -> (TmdsVideo, TmdsLinkParams) {
    let video = TmdsVideo::new(pixel_clock_khz, ColorBitDepth::Depth8, ColorFormat::Rgb444);
    let rate = video.character_rate_khz().expect("TMDS carries it");
    let source = SourceTmdsFacts::from(&bench.phy().capabilities());
    let link =
        TmdsLinkParams::choose(rate, sink, source, TmdsPolicy::DEFAULT).expect("a link for it");
    (video, link)
}

/// 4K60: a scrambled 1/40 link.
fn uhd(bench: &Bench<'_>) -> (TmdsVideo, TmdsLinkParams) {
    video(bench, 594_000, sink_facts(false))
}

fn bench(config: SinkConfig) -> Bench<'static> {
    Bench::new(config, &[]).unwrap()
}

/// Brings `params` up, expecting it up (not cancelled); returns what came up.
fn bring_up<'b>(
    tmds: &mut Link<'b>,
    video: TmdsVideo,
    params: &TmdsLinkParams,
    config: &BringUpConfig,
) -> LinkUp<spiritlevel::CulvertError<maquette::BusError>> {
    match tmds.bring_up(params, video, config) {
        Ok(BringUpOutcome::Up(up)) => up,
        Ok(_) => panic!("cancelled without a cancel flag"),
        Err(error) => panic!("the bring-up failed: {error}"),
    }
}

fn not_confirmed(up: &LinkUp<spiritlevel::CulvertError<maquette::BusError>>) -> bool {
    up.warnings
        .iter_warnings()
        .any(|warning| matches!(warning, BringUpWarning::ScramblingNotConfirmed { .. }))
}

#[test]
fn a_bring_up_at_594_mcsc_locks_the_sink_and_a_tear_down_clears_it() {
    let bench = Bench::new(SinkConfig::new(), &[]).unwrap();
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    assert_eq!(params.clock_ratio(), TmdsClockRatio::Div40);

    let outcome = tmds
        .bring_up(&params, video, &BringUpConfig::DEFAULT)
        .expect("the bring-up succeeds");
    let BringUpOutcome::Up(up) = outcome else {
        panic!("not cancelled: no cancel flag");
    };
    assert_eq!(up.check.result, CheckResult::Set);
    assert_eq!(up.warnings.iter_warnings().count(), 0);
    assert_eq!(bench.sink().tmds_config(), Some(0b11));
    assert_eq!(bench.sink().scrambling_status(), Some(1));
    assert_eq!(
        bench.line(),
        Line::Tmds {
            character_rate_khz: 594_000,
            ratio: TmdsClockRatio::Div40,
            scrambled: true
        }
    );
    // The sink was written before scrambled output started.
    let records: Vec<_> = bench.transcript().collect();
    let written = records.iter().position(|record| {
        matches!(
            record.event,
            Event::Register {
                addr: SCDC,
                reg: 0x20,
                to: 0b11,
                ..
            }
        )
    });
    let started = records.iter().position(|record| {
        matches!(
            record.event,
            Event::Phy {
                call: PhyCall::StartOutput,
                result: Ok(()),
                ..
            }
        )
    });
    assert!(written.unwrap() < started.unwrap());

    let torn = tmds
        .tear_down(
            &params,
            &TearDownConfig::DEFAULT.with_clear_sink(true),
            SinkPresence::Present,
        )
        .expect("the tear-down succeeds");
    assert!(matches!(torn.sink, SinkClear::Cleared));
    assert_eq!(bench.line(), Line::Off);
    assert_eq!(bench.sink().tmds_config(), Some(0));
    assert_eq!(bench.sink().scrambling_status(), Some(0));
}

#[test]
fn a_plain_1_10_link_writes_tmds_config_0_and_checks_nothing() {
    let bench = bench(SinkConfig::new());
    let mut tmds = link(&bench);
    let (video, params) = video(&bench, 148_500, sink_facts(false));
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::NotApplicable);
    assert_eq!(bench.sink().tmds_config(), Some(0));
    assert_eq!(bench.sink().scrambling_status(), Some(0));
    assert_eq!(
        bench.line(),
        Line::Tmds {
            character_rate_khz: 148_500,
            ratio: TmdsClockRatio::Div10,
            scrambled: false
        }
    );
}

#[test]
fn low_rate_scrambling_at_1_10_locks() {
    let bench = bench(SinkConfig::new());
    let mut tmds = link(&bench);
    let (video, params) = video(&bench, 297_000, sink_facts(true));
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::Set);
    assert_eq!(bench.sink().tmds_config(), Some(0b01));
}

#[test]
fn a_slow_sink_locks_within_the_check_window_and_a_slower_one_does_not() {
    let config = SinkConfig::new().with_scrambling_lock(Trigger::After(Duration::from_millis(150)));
    let bench = bench(config);
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::Set);

    let config = SinkConfig::new().with_scrambling_lock(Trigger::After(Duration::from_millis(250)));
    let bench = self::bench(config);
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::NotSet);
    assert!(not_confirmed(&up));
    // The link is left running, and the sink locks after the window.
    bench.advance(Duration::from_millis(100));
    assert_eq!(bench.sink().scrambling_status(), Some(1));
}

#[test]
fn a_sink_that_withholds_the_status_once_locks_on_the_quick_restore() {
    let config =
        SinkConfig::new().with_scrambling_refusals(Refusal::new(RefusalKind::WithholdStatus, 1));
    let bench = bench(config);
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::NotSet);
    assert!(not_confirmed(&up));
    let restored = tmds
        .quick_restore(&params, &BringUpConfig::DEFAULT)
        .expect("the restore succeeds");
    assert_eq!(restored.check.result, CheckResult::Set);
}

/// A sink that re-syncs when `TMDS_Config` is rewritten with the value it holds, as the
/// sinks amdgpu's `skip_scdc_overwrite` quirk exists for.
struct ResyncsOnRewrite;

impl ScdcHook for ResyncsOnRewrite {
    fn write(
        &mut self,
        reg: u8,
        value: u8,
        regs: &ScdcRegisters<'_>,
        _: &mut Env<'_>,
    ) -> HookWrite {
        if reg == 0x20 && regs.get(0x20) == value {
            HookWrite::StoreRelocking(value)
        } else {
            HookWrite::Store(value)
        }
    }
}

#[test]
fn a_quick_restore_waits_out_a_sink_that_resyncs_on_the_rewrite() {
    let mut hook = ResyncsOnRewrite;
    let bench = Bench::builder(&[]).scdc_hook(&mut hook).build().unwrap();
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::Set);
    let restored = tmds
        .quick_restore(&params, &BringUpConfig::DEFAULT)
        .expect("the restore succeeds");
    // The rewrite cleared the status; the check window saw it set again.
    assert_eq!(restored.check.result, CheckResult::Set);
    let status: Vec<_> = bench
        .transcript()
        .filter_map(|record| match record.event {
            Event::Register {
                reg: 0x21,
                from,
                to,
                ..
            } => Some((from, to)),
            _ => None,
        })
        .collect();
    assert_eq!(status, [(0, 1), (1, 0), (0, 1)]);
}

#[test]
fn a_sink_that_drops_scrambling_enable_is_found_lost_and_restored() {
    let config =
        SinkConfig::new().with_scrambling_refusals(Refusal::new(RefusalKind::DropEnable, 1));
    let bench = bench(config);
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::NotSet);
    assert_eq!(bench.sink().tmds_config(), Some(0b10));
    assert!(matches!(
        tmds.check_reconnect(&params, sink_facts(false)),
        ReconnectCheck::Lost { .. }
    ));
    let restored = tmds
        .quick_restore(&params, &BringUpConfig::DEFAULT)
        .expect("the restore succeeds");
    assert_eq!(restored.check.result, CheckResult::Set);
    assert_eq!(bench.sink().tmds_config(), Some(0b11));
}

#[test]
fn by_default_any_hpd_pulse_loses_the_configuration() {
    let bench = bench(SinkConfig::new());
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    bench.hpd_pulse(Duration::from_millis(1));
    assert!(matches!(
        tmds.check_reconnect(&params, sink_facts(false)),
        ReconnectCheck::Lost { .. }
    ));
    let restored = tmds
        .quick_restore(&params, &BringUpConfig::DEFAULT)
        .expect("the restore succeeds");
    assert_eq!(restored.check.result, CheckResult::Set);
}

#[test]
fn a_short_hpd_pulse_keeps_the_configuration_and_a_long_one_loses_it() {
    // A sink that resets after 100 ms of HPD low.
    let config = SinkConfig::new()
        .with_hpd_reset(
            RegisterGroup::TmdsConfig,
            HpdReset::After(Duration::from_millis(100)),
        )
        .with_hpd_reset(
            RegisterGroup::Status,
            HpdReset::After(Duration::from_millis(100)),
        );
    let bench = bench(config);
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    bench.hpd_pulse(Duration::from_millis(50));
    assert!(matches!(
        tmds.check_reconnect(&params, sink_facts(false)),
        ReconnectCheck::Matches
    ));
    bench.hpd_pulse(Duration::from_millis(200));
    assert_eq!(bench.sink().scrambling_status(), Some(0));
    assert!(matches!(
        tmds.check_reconnect(&params, sink_facts(false)),
        ReconnectCheck::Lost { .. }
    ));
    let restored = tmds
        .quick_restore(&params, &BringUpConfig::DEFAULT)
        .expect("the restore succeeds");
    assert_eq!(restored.check.result, CheckResult::Set);
    // A sink whose facts changed is not read.
    assert!(matches!(
        tmds.check_reconnect(&params, SinkTmdsFacts::new(false, false, false)),
        ReconnectCheck::SinkChanged
    ));
}

#[test]
fn a_strict_sink_locks_when_the_scrambler_follows_the_write_but_not_on_a_running_restore() {
    let config = SinkConfig::new().with_strict_lock(true);
    let bench = bench(config);
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    // Unscrambled output through the write, the scrambler on after it: strict enough.
    let running =
        BringUpConfig::DEFAULT.with_output_during_sink_write(OutputDuringSinkWrite::Running);
    let up = bring_up(&mut tmds, video, &params, &running);
    assert_eq!(up.check.result, CheckResult::Set);
    // After a long HPD low, the quick restore writes the sink with scrambled output already
    // running: the strict sink does not lock.
    bench.hpd_pulse(Duration::from_millis(200));
    let restored = tmds
        .quick_restore(&params, &BringUpConfig::DEFAULT)
        .expect("the restore succeeds");
    assert_eq!(restored.check.result, CheckResult::NotSet);
    // A tear-down and a bring-up recover.
    tmds.tear_down(&params, &TearDownConfig::DEFAULT, SinkPresence::Present)
        .expect("the tear-down succeeds");
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert_eq!(up.check.result, CheckResult::Set);
}

#[test]
fn a_nacked_tmds_config_write_carries_on_with_a_warning() {
    let bench = bench(SinkConfig::new());
    let write = Filter::ANY
        .with_addr(maquette::hdmi_hal::ddc::SCDC)
        .with_op(OpKind::Write)
        .with_first(0x20);
    let nack = DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Data);
    bench
        .schedule(Fault::new(Effect::Fail(nack)).on(write))
        .unwrap();
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let up = bring_up(&mut tmds, video, &params, &BringUpConfig::DEFAULT);
    assert!(
        up.warnings
            .iter_warnings()
            .any(|warning| matches!(warning, BringUpWarning::TmdsConfigWriteFailed { .. }))
    );
    assert_eq!(up.check.result, CheckResult::NotSet);
    assert_eq!(bench.sink().tmds_config(), Some(0));
}

#[test]
fn a_detached_tmds_config_write_ends_the_bring_up_with_a_tear_down() {
    let bench = bench(SinkConfig::new());
    let write = Filter::ANY
        .with_addr(maquette::hdmi_hal::ddc::SCDC)
        .with_op(OpKind::Write)
        .with_first(0x20);
    bench
        .schedule(Fault::new(Effect::Fail(DdcErrorKind::Detached)).on(write))
        .unwrap();
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let error = tmds
        .bring_up(&params, video, &BringUpConfig::DEFAULT)
        .unwrap_err();
    assert!(matches!(error, BringUpError::Scdc { .. }), "{error}");
    assert_eq!(bench.line(), Line::Off);
}

#[test]
fn a_failed_start_output_ends_the_bring_up_with_a_phy_error() {
    let bench = bench(SinkConfig::new());
    bench
        .schedule_phy(PhyFault::new().on(PhyCallKind::StartOutput))
        .unwrap();
    let mut tmds = link(&bench);
    let (video, params) = uhd(&bench);
    let error = tmds
        .bring_up(&params, video, &BringUpConfig::DEFAULT)
        .unwrap_err();
    assert!(matches!(error, BringUpError::Phy { .. }), "{error}");
    assert_eq!(bench.line(), Line::Off);
}
