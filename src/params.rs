//! The chosen link: `TmdsLinkParams`, its `choose` function, and why the link is or is
//! not scrambled (`ScramblingDecision`, `ScramblingOff`).

use crate::error::ParamsError;
use crate::types::{
    ClockRatio, ScramblingThreshold, SinkTmdsConfig, SinkTmdsFacts, SourceTmdsFacts, TmdsPolicy,
};

/// The rate, in kHz, that the threshold is measured against: 340 Mcsc.
const THRESHOLD_KHZ: u32 = 340_000;

/// Why the link is or is not scrambled.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScramblingDecision {
    /// The rate is above the threshold, so the link is scrambled at 1/40.
    Required,
    /// The rate is at or below the threshold, and the source, the sink and the policy
    /// all allow scrambling.
    LowRate,
    /// The link is 1/10 and not scrambled: the rate is at or below the threshold, or it
    /// is exactly 340 000 kHz under [`ScramblingThreshold::AtOrAbove340`] on a sink
    /// without SCDC or scrambling support (then `no_scdc` or `no_scrambling_support` is
    /// set).
    Off(ScramblingOff),
}

/// Every condition that keeps a 1/10 link unscrambled.
///
/// At least one flag is set. Each is judged on its own fact, independently of the
/// others: a sink without SCDC that also does not declare `LTE_340Mcsc_scramble` has
/// both flags set.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScramblingOff {
    /// The sink has no SCDC, so it cannot be told to expect scrambling.
    pub no_scdc: bool,
    /// The sink does not support scrambling
    /// ([`SinkTmdsFacts::scrambling_supported`] is clear).
    pub no_scrambling_support: bool,
    /// The sink does not declare `LTE_340Mcsc_scramble`.
    pub sink_does_not_declare: bool,
    /// The source cannot scramble at or below 340 Mcsc.
    pub source_cannot: bool,
    /// The policy does not permit scrambling at or below the threshold.
    pub policy_off: bool,
}

/// The chosen TMDS link: clock ratio, scrambling, and the sink's `TMDS_Config`.
///
/// Built only by [`choose`](Self::choose), so the sink's value always matches the
/// link the source runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TmdsLinkParams {
    rate_khz: u32,
    clock_ratio: ClockRatio,
    decision: ScramblingDecision,
    sink_config: Option<SinkTmdsConfig>,
}

impl TmdsLinkParams {
    /// Chooses the link for a TMDS character rate.
    ///
    /// `rate_khz` is the TMDS character rate in kHz, rounded up: truncating would put a
    /// rate just above 340 000 kHz on the wrong side of the threshold.
    ///
    /// - Above the policy's threshold, the link is 1/40 and scrambled, on a sink with
    ///   SCDC and scrambling support.
    /// - At or below it, the link is 1/10, and scrambled when the source, the sink
    ///   (SCDC, scrambling support and `LTE_340Mcsc_scramble`) and the policy all allow
    ///   it; otherwise every reason is reported.
    /// - At exactly 340 000 kHz with [`ScramblingThreshold::AtOrAbove340`], a sink
    ///   without SCDC or scrambling support gets the plain 1/10 link, and the flag that
    ///   says why.
    /// - A sink without SCDC gets no `TMDS_Config` value.
    ///
    /// # Errors
    ///
    /// [`ParamsError::ScramblingUnsupported`] for a rate above 340 000 kHz on a sink
    /// without SCDC or scrambling support.
    ///
    /// # Example
    ///
    /// ```
    /// use spiritlevel::{
    ///     ClockRatio, ScramblingDecision, SinkTmdsFacts, SourceTmdsFacts, TmdsLinkParams,
    ///     TmdsPolicy,
    /// };
    ///
    /// // 4K at 60 Hz, 8 bpc RGB: 594 MHz.
    /// let sink = SinkTmdsFacts::new(true, true, false);
    /// let source = SourceTmdsFacts::new(false);
    /// let link = TmdsLinkParams::choose(594_000, sink, source, TmdsPolicy::DEFAULT)?;
    ///
    /// assert_eq!(link.clock_ratio(), ClockRatio::Div40);
    /// assert_eq!(link.scrambling_decision(), ScramblingDecision::Required);
    /// let config = link.sink_config().expect("the sink has SCDC");
    /// assert!(config.scrambling_enable && config.high_tmds_clock_ratio);
    /// # Ok::<(), spiritlevel::ParamsError>(())
    /// ```
    pub const fn choose(
        rate_khz: u32,
        sink: SinkTmdsFacts,
        source: SourceTmdsFacts,
        policy: TmdsPolicy,
    ) -> Result<Self, ParamsError> {
        let above_threshold = match policy.threshold {
            ScramblingThreshold::Above340 => rate_khz > THRESHOLD_KHZ,
            ScramblingThreshold::AtOrAbove340 => rate_khz >= THRESHOLD_KHZ,
        };
        let sink_can_scramble = sink.scdc_present && sink.scrambling_supported;

        if above_threshold && sink_can_scramble {
            return Ok(Self {
                rate_khz,
                clock_ratio: ClockRatio::Div40,
                decision: ScramblingDecision::Required,
                sink_config: Some(SinkTmdsConfig::new(true, true)),
            });
        }
        if rate_khz > THRESHOLD_KHZ {
            // Above the threshold under either setting, and the sink cannot take it.
            return Err(ParamsError::ScramblingUnsupported {
                rate_khz,
                no_scdc: !sink.scdc_present,
                no_scrambling_support: !sink.scrambling_supported,
            });
        }

        // At or below the threshold, or exactly 340 000 kHz on a sink without SCDC or
        // scrambling support (one of those flags is then set).
        let off = ScramblingOff {
            no_scdc: !sink.scdc_present,
            no_scrambling_support: !sink.scrambling_supported,
            sink_does_not_declare: !sink.lte_340mcsc_scramble,
            source_cannot: !source.low_rate_scrambling,
            policy_off: !policy.low_rate_scrambling,
        };
        let scrambled = !(off.no_scdc
            || off.no_scrambling_support
            || off.sink_does_not_declare
            || off.source_cannot
            || off.policy_off);
        let decision = if scrambled {
            ScramblingDecision::LowRate
        } else {
            ScramblingDecision::Off(off)
        };
        let sink_config = if sink.scdc_present {
            Some(SinkTmdsConfig::new(scrambled, false))
        } else {
            None
        };
        Ok(Self {
            rate_khz,
            clock_ratio: ClockRatio::Div10,
            decision,
            sink_config,
        })
    }

    /// The TMDS character rate the link was chosen for, in kHz.
    #[must_use]
    pub const fn rate_khz(&self) -> u32 {
        self.rate_khz
    }

    /// The TMDS bit clock ratio.
    #[must_use]
    pub const fn clock_ratio(&self) -> ClockRatio {
        self.clock_ratio
    }

    /// Whether the link is scrambled: true for [`ScramblingDecision::Required`] and
    /// [`ScramblingDecision::LowRate`].
    #[must_use]
    pub const fn scrambling(&self) -> bool {
        matches!(
            self.decision,
            ScramblingDecision::Required | ScramblingDecision::LowRate
        )
    }

    /// Why the link is or is not scrambled.
    #[must_use]
    pub const fn scrambling_decision(&self) -> ScramblingDecision {
        self.decision
    }

    /// What the sink's `TMDS_Config` must hold; `None` for a sink without SCDC, whose
    /// register is not written.
    #[must_use]
    pub const fn sink_config(&self) -> Option<SinkTmdsConfig> {
        self.sink_config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATES: [u32; 9] = [
        0,
        25_175,
        165_000,
        339_999,
        340_000,
        340_001,
        594_000,
        600_000,
        u32::MAX,
    ];
    const THRESHOLDS: [ScramblingThreshold; 2] = [
        ScramblingThreshold::Above340,
        ScramblingThreshold::AtOrAbove340,
    ];
    const BOOLS: [bool; 2] = [false, true];

    /// Calls `f` with every combination of inputs over [`RATES`].
    fn for_all(mut f: impl FnMut(u32, SinkTmdsFacts, SourceTmdsFacts, TmdsPolicy)) {
        for rate in RATES {
            for scdc in BOOLS {
                for supported in BOOLS {
                    for lte in BOOLS {
                        for src in BOOLS {
                            for pol in BOOLS {
                                for threshold in THRESHOLDS {
                                    f(
                                        rate,
                                        SinkTmdsFacts::new(scdc, supported, lte),
                                        SourceTmdsFacts::new(src),
                                        TmdsPolicy::DEFAULT
                                            .with_low_rate_scrambling(pol)
                                            .with_threshold(threshold),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn is_above(rate: u32, threshold: ScramblingThreshold) -> bool {
        match threshold {
            ScramblingThreshold::Above340 => rate > 340_000,
            ScramblingThreshold::AtOrAbove340 => rate >= 340_000,
        }
    }

    const ALL: SinkTmdsFacts = SinkTmdsFacts::new(true, true, true);
    const CAN: SourceTmdsFacts = SourceTmdsFacts::new(true);
    const AT_OR_ABOVE: TmdsPolicy =
        TmdsPolicy::DEFAULT.with_threshold(ScramblingThreshold::AtOrAbove340);

    // ---- Invariants over every combination ----

    #[test]
    fn errors_exactly_above_340_on_an_incapable_sink() {
        for_all(|rate, sink, source, policy| {
            let incapable = !(sink.scdc_present && sink.scrambling_supported);
            let result = TmdsLinkParams::choose(rate, sink, source, policy);
            if rate > 340_000 && incapable {
                assert_eq!(
                    result,
                    Err(ParamsError::ScramblingUnsupported {
                        rate_khz: rate,
                        no_scdc: !sink.scdc_present,
                        no_scrambling_support: !sink.scrambling_supported,
                    })
                );
            } else {
                assert!(result.is_ok(), "{rate} {sink:?} {source:?} {policy:?}");
            }
        });
    }

    #[test]
    fn link_is_consistent() {
        for_all(|rate, sink, source, policy| {
            let Ok(link) = TmdsLinkParams::choose(rate, sink, source, policy) else {
                return;
            };
            assert_eq!(link.rate_khz(), rate);

            let required = link.scrambling_decision() == ScramblingDecision::Required;
            assert_eq!(link.clock_ratio() == ClockRatio::Div40, required);
            assert_eq!(
                required,
                is_above(rate, policy.threshold) && sink.scdc_present && sink.scrambling_supported
            );
            assert_eq!(
                link.scrambling(),
                !matches!(link.scrambling_decision(), ScramblingDecision::Off(_))
            );

            match link.sink_config() {
                None => assert!(!sink.scdc_present),
                Some(config) => {
                    assert!(sink.scdc_present);
                    assert_eq!(config.scrambling_enable, link.scrambling());
                    assert_eq!(
                        config.high_tmds_clock_ratio,
                        link.clock_ratio() == ClockRatio::Div40
                    );
                }
            }
        });
    }

    #[test]
    fn every_reason_is_set_exactly_when_its_fact_blocks() {
        for_all(|rate, sink, source, policy| {
            let Ok(link) = TmdsLinkParams::choose(rate, sink, source, policy) else {
                return;
            };
            if link.scrambling_decision() == ScramblingDecision::Required {
                return;
            }
            let all_allow = sink.scdc_present
                && sink.scrambling_supported
                && sink.lte_340mcsc_scramble
                && source.low_rate_scrambling
                && policy.low_rate_scrambling;
            match link.scrambling_decision() {
                ScramblingDecision::LowRate => assert!(all_allow),
                ScramblingDecision::Off(off) => {
                    assert!(!all_allow);
                    assert_eq!(off.no_scdc, !sink.scdc_present);
                    assert_eq!(off.no_scrambling_support, !sink.scrambling_supported);
                    assert_eq!(off.sink_does_not_declare, !sink.lte_340mcsc_scramble);
                    assert_eq!(off.source_cannot, !source.low_rate_scrambling);
                    assert_eq!(off.policy_off, !policy.low_rate_scrambling);
                }
                ScramblingDecision::Required => unreachable!(),
            }
        });
    }

    // ---- The boundary ----

    #[test]
    fn exactly_340_000_follows_the_threshold() {
        let link = TmdsLinkParams::choose(340_000, ALL, CAN, TmdsPolicy::DEFAULT).unwrap();
        assert_eq!(link.clock_ratio(), ClockRatio::Div10);
        assert_eq!(link.scrambling_decision(), ScramblingDecision::LowRate);

        let link = TmdsLinkParams::choose(340_000, ALL, CAN, AT_OR_ABOVE).unwrap();
        assert_eq!(link.clock_ratio(), ClockRatio::Div40);
        assert_eq!(link.scrambling_decision(), ScramblingDecision::Required);
    }

    #[test]
    fn rate_340_001_is_above_under_both_thresholds() {
        for policy in [TmdsPolicy::DEFAULT, AT_OR_ABOVE] {
            let link = TmdsLinkParams::choose(340_001, ALL, CAN, policy).unwrap();
            assert_eq!(link.clock_ratio(), ClockRatio::Div40);
            assert_eq!(link.sink_config(), Some(SinkTmdsConfig::new(true, true)));
        }
    }

    #[test]
    fn exactly_340_000_at_or_above_on_an_incapable_sink_falls_back() {
        // No SCDC: the plain 1/10 link, nothing written, and the reason.
        let link = TmdsLinkParams::choose(340_000, SinkTmdsFacts::NONE, CAN, AT_OR_ABOVE).unwrap();
        assert_eq!(link.clock_ratio(), ClockRatio::Div10);
        assert_eq!(link.sink_config(), None);
        assert!(matches!(
            link.scrambling_decision(),
            ScramblingDecision::Off(off) if off.no_scdc
        ));

        // SCDC but no scrambling support, even with LTE_340Mcsc_scramble declared: the
        // plain 1/10 link, the ratio cleared, and the reason.
        let sink = SinkTmdsFacts::new(true, false, true);
        let link = TmdsLinkParams::choose(340_000, sink, CAN, AT_OR_ABOVE).unwrap();
        assert_eq!(link.clock_ratio(), ClockRatio::Div10);
        assert_eq!(link.sink_config(), Some(SinkTmdsConfig::new(false, false)));
        assert_eq!(
            link.scrambling_decision(),
            ScramblingDecision::Off(ScramblingOff {
                no_scdc: false,
                no_scrambling_support: true,
                sink_does_not_declare: false,
                source_cannot: false,
                policy_off: false,
            })
        );
    }

    #[test]
    fn low_rate_scrambling_needs_scrambling_support() {
        // LTE_340Mcsc_scramble without a non-zero Max_TMDS_Character_Rate does not count:
        // the sources take the low-rate fact from Linux's gated `scrambling.low_rates`.
        let sink = SinkTmdsFacts::new(true, false, true);
        let link = TmdsLinkParams::choose(297_000, sink, CAN, TmdsPolicy::DEFAULT).unwrap();
        assert!(!link.scrambling());
        assert_eq!(link.sink_config(), Some(SinkTmdsConfig::new(false, false)));
    }

    // ---- Named cases ----

    #[test]
    fn dvi_sink_gets_the_plain_link_with_every_reason() {
        let link = TmdsLinkParams::choose(
            148_500,
            SinkTmdsFacts::NONE,
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT.with_low_rate_scrambling(false),
        )
        .unwrap();
        assert_eq!(link.clock_ratio(), ClockRatio::Div10);
        assert_eq!(link.sink_config(), None);
        assert_eq!(
            link.scrambling_decision(),
            ScramblingDecision::Off(ScramblingOff {
                no_scdc: true,
                no_scrambling_support: true,
                sink_does_not_declare: true,
                source_cannot: true,
                policy_off: true,
            })
        );
    }

    #[test]
    fn scdc_sink_below_threshold_gets_the_ratio_cleared() {
        let sink = SinkTmdsFacts::new(true, true, false);
        let link = TmdsLinkParams::choose(297_000, sink, CAN, TmdsPolicy::DEFAULT).unwrap();
        assert_eq!(link.sink_config(), Some(SinkTmdsConfig::new(false, false)));
    }

    #[test]
    fn low_rate_scrambling_writes_scrambling_enable_only() {
        let link = TmdsLinkParams::choose(297_000, ALL, CAN, TmdsPolicy::DEFAULT).unwrap();
        assert_eq!(link.scrambling_decision(), ScramblingDecision::LowRate);
        assert_eq!(link.sink_config(), Some(SinkTmdsConfig::new(true, false)));
    }

    #[test]
    fn choose_is_const() {
        const LINK: Result<TmdsLinkParams, ParamsError> =
            TmdsLinkParams::choose(594_000, ALL, CAN, TmdsPolicy::DEFAULT);
        const RATIO_IS_DIV40: bool = match LINK {
            Ok(link) => matches!(link.clock_ratio(), ClockRatio::Div40),
            Err(_) => false,
        };
        const { assert!(RATIO_IS_DIV40) };
    }
}
