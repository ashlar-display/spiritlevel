//! The procedures' settings: [`BringUpConfig`] for bring-up and the quick restore,
//! [`TearDownConfig`] for tear-down, and the facts a call is given ([`SinkPresence`]).
//!
//! Where HDMI sources differ, the setting offers each surveyed behaviour, with the
//! majority's as the default or, where there is no majority, a chosen one; the architecture
//! document cites the sources behind each.

use core::fmt;
use core::time::Duration;

/// Bring-up and quick-restore settings.
///
/// [`DEFAULT`](Self::DEFAULT) (also [`Default`]) holds the default of every field; the
/// `with_…` setters change one each, as `const fn`, so firmware can fix its settings at
/// compile time.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BringUpConfig {
    /// How long to wait after the sink's `TMDS_Config` write before starting the output, so
    /// the sink can switch its clock ratio. Default 1 ms.
    pub ratio_wait: Duration,
    /// The longest time from the sink's `TMDS_Config` write to scrambled output reaching
    /// the sink: to the output's start, or with [`OutputDuringSinkWrite::Running`] to the
    /// scrambler coming on. The wait counts towards it. Later is a warning
    /// ([`DeadlineMissed`](crate::BringUpWarning::DeadlineMissed)), not a failure. Default
    /// 100 ms.
    pub scrambling_deadline: Duration,
    /// How long the scrambling check reads `Scrambling_Status`, from the output's start (from
    /// the scrambler coming on, with [`OutputDuringSinkWrite::Running`] and in the quick
    /// restore). Default 200 ms.
    pub check_window: Duration,
    /// The time between two reads of the check, and the longest slice of the ratio wait
    /// between two reads of the cancel flag. Default 1 ms.
    pub check_interval: Duration,
    /// Which sinks get the FRL clear before a bring-up.
    pub clear_frl: ClearFrl,
    /// When the source writes `Source_Version`.
    pub source_version: SourceVersionWrite,
    /// Whether the output is stopped while the sink's `TMDS_Config` is written.
    pub output_during_sink_write: OutputDuringSinkWrite,
    /// How `TMDS_Config` is written.
    pub tmds_config_write: TmdsConfigWrite,
    /// What a failed `TMDS_Config` write does to the bring-up.
    pub on_tmds_config_failure: OnTmdsConfigFailure,
    /// Whether an error is followed by a tear-down, so no half-started output is left
    /// behind. Default `true`; off leaves both ends as the error left them.
    pub tear_down_on_error: bool,
    /// The tear-down settings used after an error and when the bring-up is cancelled.
    pub tear_down: TearDownConfig,
}

impl BringUpConfig {
    /// Every field at its default.
    pub const DEFAULT: Self = Self {
        ratio_wait: Duration::from_millis(1),
        scrambling_deadline: Duration::from_millis(100),
        check_window: Duration::from_millis(200),
        check_interval: Duration::from_millis(1),
        clear_frl: ClearFrl::IfSinkDeclaresFrl,
        source_version: SourceVersionWrite::EveryBringUp,
        output_during_sink_write: OutputDuringSinkWrite::Stopped,
        tmds_config_write: TmdsConfigWrite::ReadModifyWrite,
        on_tmds_config_failure: OnTmdsConfigFailure::CarryOn,
        tear_down_on_error: true,
        tear_down: TearDownConfig::DEFAULT,
    };

    /// Checks the timings for combinations that defeat their purpose; the procedures run
    /// with any values.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] with every problem found: a scrambling deadline not longer than the
    /// ratio wait, which counts towards it, so every bring-up misses it; a check interval
    /// not shorter than the check window, so the check reads once; a check interval of 0,
    /// so the check reads back to back, as fast as the bus answers.
    pub const fn validate(&self) -> Result<(), ConfigError> {
        let error = ConfigError {
            deadline_always_missed: self.scrambling_deadline.as_nanos()
                <= self.ratio_wait.as_nanos(),
            check_reads_once: self.check_interval.as_nanos() >= self.check_window.as_nanos(),
            check_interval_zero: self.check_interval.is_zero(),
        };
        if error.deadline_always_missed || error.check_reads_once || error.check_interval_zero {
            Err(error)
        } else {
            Ok(())
        }
    }

    /// Sets [`ratio_wait`](Self::ratio_wait).
    #[must_use]
    pub const fn with_ratio_wait(mut self, ratio_wait: Duration) -> Self {
        self.ratio_wait = ratio_wait;
        self
    }

    /// Sets [`scrambling_deadline`](Self::scrambling_deadline).
    #[must_use]
    pub const fn with_scrambling_deadline(mut self, scrambling_deadline: Duration) -> Self {
        self.scrambling_deadline = scrambling_deadline;
        self
    }

    /// Sets [`check_window`](Self::check_window).
    #[must_use]
    pub const fn with_check_window(mut self, check_window: Duration) -> Self {
        self.check_window = check_window;
        self
    }

    /// Sets [`check_interval`](Self::check_interval).
    #[must_use]
    pub const fn with_check_interval(mut self, check_interval: Duration) -> Self {
        self.check_interval = check_interval;
        self
    }

    /// Sets [`clear_frl`](Self::clear_frl).
    #[must_use]
    pub const fn with_clear_frl(mut self, clear_frl: ClearFrl) -> Self {
        self.clear_frl = clear_frl;
        self
    }

    /// Sets [`source_version`](Self::source_version).
    #[must_use]
    pub const fn with_source_version(mut self, source_version: SourceVersionWrite) -> Self {
        self.source_version = source_version;
        self
    }

    /// Sets [`output_during_sink_write`](Self::output_during_sink_write).
    #[must_use]
    pub const fn with_output_during_sink_write(
        mut self,
        output_during_sink_write: OutputDuringSinkWrite,
    ) -> Self {
        self.output_during_sink_write = output_during_sink_write;
        self
    }

    /// Sets [`tmds_config_write`](Self::tmds_config_write).
    #[must_use]
    pub const fn with_tmds_config_write(mut self, tmds_config_write: TmdsConfigWrite) -> Self {
        self.tmds_config_write = tmds_config_write;
        self
    }

    /// Sets [`on_tmds_config_failure`](Self::on_tmds_config_failure).
    #[must_use]
    pub const fn with_on_tmds_config_failure(
        mut self,
        on_tmds_config_failure: OnTmdsConfigFailure,
    ) -> Self {
        self.on_tmds_config_failure = on_tmds_config_failure;
        self
    }

    /// Sets [`tear_down_on_error`](Self::tear_down_on_error).
    #[must_use]
    pub const fn with_tear_down_on_error(mut self, tear_down_on_error: bool) -> Self {
        self.tear_down_on_error = tear_down_on_error;
        self
    }

    /// Sets [`tear_down`](Self::tear_down).
    #[must_use]
    pub const fn with_tear_down(mut self, tear_down: TearDownConfig) -> Self {
        self.tear_down = tear_down;
        self
    }
}

impl Default for BringUpConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What [`BringUpConfig::validate`] found: every problem, at least one.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfigError {
    /// The scrambling deadline is not longer than the ratio wait, which counts towards it:
    /// every bring-up would miss it.
    pub deadline_always_missed: bool,
    /// The check interval is not shorter than the check window: the check would read once.
    pub check_reads_once: bool,
    /// The check interval is 0: the check would read back to back, as fast as the bus
    /// answers, for the whole window.
    pub check_interval_zero: bool,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let problems = [
            (
                self.deadline_always_missed,
                "the scrambling deadline is not longer than the ratio wait",
            ),
            (
                self.check_reads_once,
                "the check interval is not shorter than the check window",
            ),
            (self.check_interval_zero, "the check interval is 0"),
        ];
        let mut first = true;
        for (found, text) in problems {
            if found {
                if !first {
                    f.write_str("; ")?;
                }
                f.write_str(text)?;
                first = false;
            }
        }
        if first {
            f.write_str("the bring-up settings are inconsistent")?;
        }
        Ok(())
    }
}

impl core::error::Error for ConfigError {}

/// Whether the output is stopped while the sink's `TMDS_Config` is written.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OutputDuringSinkWrite {
    /// Stopped first, then started after the write (the default).
    #[default]
    Stopped,
    /// Left running through the write, with the source's scrambler held off until after
    /// it, so the sink never receives scrambled TMDS before it has been told to expect it.
    /// vc4's order: the PHY runs at the link's clock ratio from the start, so on a 1/40 link
    /// the sink sees the 1/40 clock before its write. Tegra switches its clock mode only with
    /// its scrambler, after the write, which `TmdsPhy` cannot do while transmitting.
    Running,
}

/// How `TMDS_Config` is written.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TmdsConfigWrite {
    /// Read the byte, set the two bits, write it back with the other bits as read
    /// ([`TmdsScdc::update_tmds_config`](crate::TmdsScdc::update_tmds_config); the default).
    #[default]
    ReadModifyWrite,
    /// Write the whole byte, the other bits 0
    /// ([`TmdsScdc::write_tmds_config`](crate::TmdsScdc::write_tmds_config)).
    WholeByte,
}

/// What a failed `TMDS_Config` write does to the bring-up.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OnTmdsConfigFailure {
    /// Carry on and report a warning (the default).
    #[default]
    CarryOn,
    /// Stop the bring-up with an error.
    Stop,
}

/// Which sinks get the FRL clear (`Config_1` and `FLT_update`) before a bring-up.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ClearFrl {
    /// A sink that declares FRL (the default, chosen: the two sources differ).
    #[default]
    IfSinkDeclaresFrl,
    /// Every sink with SCDC.
    EveryScdcSink,
}

/// When the source writes `Source_Version` before a bring-up.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SourceVersionWrite {
    /// On every bring-up to a sink with SCDC (the default, chosen: no two sources agree).
    #[default]
    EveryBringUp,
    /// Only when `Sink_Version` reads 1; a failed read writes nothing.
    IfSinkVersion1,
    /// Only on a scrambled link, as the smaller of the sink's version and 1, and only to a
    /// sink reporting a non-zero version.
    WhenScrambling,
    /// Never: for a sink that misbehaves on the write, and for compliance and diagnostic
    /// tools.
    Never,
}

/// Tear-down settings.
///
/// [`DEFAULT`](Self::DEFAULT) (also [`Default`]): the sink is not cleared; were it cleared,
/// before the output stops; the scrambler turned off after the sink.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TearDownConfig {
    /// Whether the sink's `TMDS_Config` is cleared. Default `false`.
    pub clear_sink: bool,
    /// When the sink is cleared, relative to stopping the output.
    pub sink_clear_timing: SinkClearTiming,
    /// When the source's scrambler is turned off, relative to the sink.
    pub scrambler_off: ScramblerOff,
}

impl TearDownConfig {
    /// Every field at its default.
    pub const DEFAULT: Self = Self {
        clear_sink: false,
        sink_clear_timing: SinkClearTiming::BeforeOutputStops,
        scrambler_off: ScramblerOff::AfterSink,
    };

    /// Sets [`clear_sink`](Self::clear_sink).
    #[must_use]
    pub const fn with_clear_sink(mut self, clear_sink: bool) -> Self {
        self.clear_sink = clear_sink;
        self
    }

    /// Sets [`sink_clear_timing`](Self::sink_clear_timing).
    #[must_use]
    pub const fn with_sink_clear_timing(mut self, sink_clear_timing: SinkClearTiming) -> Self {
        self.sink_clear_timing = sink_clear_timing;
        self
    }

    /// Sets [`scrambler_off`](Self::scrambler_off).
    #[must_use]
    pub const fn with_scrambler_off(mut self, scrambler_off: ScramblerOff) -> Self {
        self.scrambler_off = scrambler_off;
        self
    }
}

impl Default for TearDownConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// When the sink's `TMDS_Config` is cleared in a tear-down, relative to stopping the output.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SinkClearTiming {
    /// Before the output stops (the default).
    #[default]
    BeforeOutputStops,
    /// After the output stops.
    AfterOutputStops,
}

/// When the source's scrambler is turned off in a tear-down, relative to the sink.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ScramblerOff {
    /// After the sink is cleared, or after its place in the order when it is not (the
    /// default, the majority's).
    #[default]
    AfterSink,
    /// Before the sink is cleared.
    BeforeSink,
}

/// Whether the sink is still there, as the caller knows from hotplug.
///
/// A fact about one call, not a setting. Exhaustive: present or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkPresence {
    /// The sink is connected.
    Present,
    /// The sink is gone: a tear-down skips every sink write.
    Gone,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_ones() {
        let c = BringUpConfig::default();
        assert_eq!(c, BringUpConfig::DEFAULT);
        assert_eq!(c.ratio_wait, Duration::from_millis(1));
        assert_eq!(c.scrambling_deadline, Duration::from_millis(100));
        assert_eq!(c.check_window, Duration::from_millis(200));
        assert_eq!(c.check_interval, Duration::from_millis(1));
        assert_eq!(c.clear_frl, ClearFrl::IfSinkDeclaresFrl);
        assert_eq!(c.source_version, SourceVersionWrite::EveryBringUp);
        assert_eq!(c.output_during_sink_write, OutputDuringSinkWrite::Stopped);
        assert_eq!(c.tmds_config_write, TmdsConfigWrite::ReadModifyWrite);
        assert_eq!(c.on_tmds_config_failure, OnTmdsConfigFailure::CarryOn);
        assert!(c.tear_down_on_error);
        assert_eq!(c.tear_down, TearDownConfig::default());

        let t = TearDownConfig::default();
        assert!(!t.clear_sink);
        assert_eq!(t.sink_clear_timing, SinkClearTiming::BeforeOutputStops);
        assert_eq!(t.scrambler_off, ScramblerOff::AfterSink);

        assert_eq!(
            OutputDuringSinkWrite::default(),
            OutputDuringSinkWrite::Stopped
        );
        assert_eq!(TmdsConfigWrite::default(), TmdsConfigWrite::ReadModifyWrite);
        assert_eq!(OnTmdsConfigFailure::default(), OnTmdsConfigFailure::CarryOn);
        assert_eq!(ClearFrl::default(), ClearFrl::IfSinkDeclaresFrl);
        assert_eq!(
            SourceVersionWrite::default(),
            SourceVersionWrite::EveryBringUp
        );
        assert_eq!(
            SinkClearTiming::default(),
            SinkClearTiming::BeforeOutputStops
        );
        assert_eq!(ScramblerOff::default(), ScramblerOff::AfterSink);
    }

    /// Every setter is `const fn`: firmware fixes its settings at compile time.
    const _: BringUpConfig = BringUpConfig::DEFAULT
        .with_ratio_wait(Duration::from_millis(2))
        .with_scrambling_deadline(Duration::from_millis(50))
        .with_check_window(Duration::from_millis(300))
        .with_check_interval(Duration::from_millis(5))
        .with_clear_frl(ClearFrl::EveryScdcSink)
        .with_source_version(SourceVersionWrite::Never)
        .with_output_during_sink_write(OutputDuringSinkWrite::Running)
        .with_tmds_config_write(TmdsConfigWrite::WholeByte)
        .with_on_tmds_config_failure(OnTmdsConfigFailure::Stop)
        .with_tear_down_on_error(false)
        .with_tear_down(
            TearDownConfig::DEFAULT
                .with_clear_sink(true)
                .with_sink_clear_timing(SinkClearTiming::AfterOutputStops)
                .with_scrambler_off(ScramblerOff::BeforeSink),
        );

    #[test]
    fn each_setter_changes_its_field_alone() {
        let tear_down = TearDownConfig::DEFAULT
            .with_clear_sink(true)
            .with_sink_clear_timing(SinkClearTiming::AfterOutputStops)
            .with_scrambler_off(ScramblerOff::BeforeSink);
        let config = BringUpConfig::DEFAULT
            .with_ratio_wait(Duration::from_millis(2))
            .with_scrambling_deadline(Duration::from_millis(50))
            .with_check_window(Duration::from_millis(300))
            .with_check_interval(Duration::from_millis(5))
            .with_clear_frl(ClearFrl::EveryScdcSink)
            .with_source_version(SourceVersionWrite::Never)
            .with_output_during_sink_write(OutputDuringSinkWrite::Running)
            .with_tmds_config_write(TmdsConfigWrite::WholeByte)
            .with_on_tmds_config_failure(OnTmdsConfigFailure::Stop)
            .with_tear_down_on_error(false)
            .with_tear_down(tear_down);

        assert_eq!(config.ratio_wait, Duration::from_millis(2));
        assert_eq!(config.scrambling_deadline, Duration::from_millis(50));
        assert_eq!(config.check_window, Duration::from_millis(300));
        assert_eq!(config.check_interval, Duration::from_millis(5));
        assert_eq!(config.clear_frl, ClearFrl::EveryScdcSink);
        assert_eq!(config.source_version, SourceVersionWrite::Never);
        assert_eq!(
            config.output_during_sink_write,
            OutputDuringSinkWrite::Running
        );
        assert_eq!(config.tmds_config_write, TmdsConfigWrite::WholeByte);
        assert_eq!(config.on_tmds_config_failure, OnTmdsConfigFailure::Stop);
        assert!(!config.tear_down_on_error);
        assert_eq!(config.tear_down, tear_down);
        assert!(tear_down.clear_sink);
        assert_eq!(
            tear_down.sink_clear_timing,
            SinkClearTiming::AfterOutputStops
        );
        assert_eq!(tear_down.scrambler_off, ScramblerOff::BeforeSink);

        // One setter leaves every other field at its default.
        assert_eq!(
            BringUpConfig::DEFAULT.with_tear_down_on_error(false),
            BringUpConfig {
                tear_down_on_error: false,
                ..BringUpConfig::DEFAULT
            }
        );
    }

    #[test]
    fn validate_reports_every_problem() {
        const { assert!(BringUpConfig::DEFAULT.validate().is_ok()) };
        let none = ConfigError {
            deadline_always_missed: false,
            check_reads_once: false,
            check_interval_zero: false,
        };
        // The boundaries: equal is already a problem.
        let wait = Duration::from_millis(50);
        let late = BringUpConfig::DEFAULT
            .with_ratio_wait(wait)
            .with_scrambling_deadline(wait);
        assert_eq!(
            late.validate(),
            Err(ConfigError {
                deadline_always_missed: true,
                ..none
            })
        );
        assert!(
            late.with_scrambling_deadline(wait + Duration::from_nanos(1))
                .validate()
                .is_ok()
        );
        let once = BringUpConfig::DEFAULT.with_check_interval(BringUpConfig::DEFAULT.check_window);
        assert_eq!(
            once.validate(),
            Err(ConfigError {
                check_reads_once: true,
                ..none
            })
        );
        let zero = BringUpConfig::DEFAULT.with_check_interval(Duration::ZERO);
        assert_eq!(
            zero.validate(),
            Err(ConfigError {
                check_interval_zero: true,
                ..none
            })
        );

        extern crate std;
        use std::string::ToString;
        assert_eq!(
            late.validate().unwrap_err().to_string(),
            "the scrambling deadline is not longer than the ratio wait"
        );
        let all = late
            .with_check_window(Duration::ZERO)
            .with_check_interval(Duration::ZERO)
            .validate()
            .unwrap_err();
        assert_eq!(
            all.to_string(),
            "the scrambling deadline is not longer than the ratio wait; the check interval \
             is not shorter than the check window; the check interval is 0"
        );
        assert_eq!(none.to_string(), "the bring-up settings are inconsistent");
    }

    #[test]
    fn sink_presence_is_exhaustive() {
        let gone = |p: SinkPresence| match p {
            SinkPresence::Present => false,
            SinkPresence::Gone => true,
        };
        assert!(gone(SinkPresence::Gone) && !gone(SinkPresence::Present));
    }
}
