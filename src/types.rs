//! The inputs and register contents: `ClockRatio`, `ScramblingThreshold`, `SinkTmdsFacts`,
//! `SourceTmdsFacts`, `TmdsPolicy` and `SinkTmdsConfig`.

/// The TMDS bit clock ratio, hdmi-hal's `TmdsClockRatio`: the TMDS clock runs at the bit
/// rate divided by 10 or by 40. One type, so the link parameters pass straight to
/// `TmdsPhy::configure`.
pub use hdmi_hal::phy::TmdsClockRatio as ClockRatio;

/// Which TMDS character rate first requires scrambling and the 1/40 clock ratio.
///
/// HDMI sources differ here; the two settings give different links only at exactly
/// 340 000 kHz. See rule 4 in the architecture document.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScramblingThreshold {
    /// Rates above 340 000 kHz (the default).
    #[default]
    Above340,
    /// Rates at or above 340 000 kHz.
    AtOrAbove340,
}

/// What the sink declares about TMDS scrambling.
///
/// [`NONE`](Self::NONE) is a sink without HDMI Forum capabilities, such as a DVI or
/// HDMI 1.x sink. With the `display-types` feature, the facts convert from the sink's
/// HF-SCDB (`From<&HdmiForumSinkCap>`, and `From<Option<&HdmiForumSinkCap>>` where an
/// absent block means [`NONE`](Self::NONE)). A caller may clear a fact for a sink known to
/// misbehave.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkTmdsFacts {
    /// The sink has an SCDC (`SCDC_Present`), so its `TMDS_Config` can be written.
    pub scdc_present: bool,
    /// The sink supports scrambling: SCDC present and a non-zero
    /// `Max_TMDS_Character_Rate`, as Linux derives `scrambling.supported`. Scrambling at
    /// any rate needs it.
    pub scrambling_supported: bool,
    /// The sink declares scrambling at or below 340 Mcsc (`LTE_340Mcsc_scramble`). It
    /// counts only with SCDC and [`scrambling_supported`](Self::scrambling_supported).
    pub lte_340mcsc_scramble: bool,
}

impl SinkTmdsFacts {
    /// A sink without HDMI Forum capabilities: no SCDC and no scrambling.
    pub const NONE: Self = Self::new(false, false, false);

    /// Facts stated by the caller.
    #[must_use]
    pub const fn new(
        scdc_present: bool,
        scrambling_supported: bool,
        lte_340mcsc_scramble: bool,
    ) -> Self {
        Self {
            scdc_present,
            scrambling_supported,
            lte_340mcsc_scramble,
        }
    }
}

#[cfg(feature = "display-types")]
impl From<&display_types::cea861::HdmiForumSinkCap> for SinkTmdsFacts {
    /// `scrambling_supported` is `scdc_present` and a non-zero `max_tmds_rate_mhz`;
    /// the other two facts are copied.
    fn from(cap: &display_types::cea861::HdmiForumSinkCap) -> Self {
        Self::new(
            cap.scdc_present,
            cap.scdc_present && cap.max_tmds_rate_mhz != 0,
            cap.lte_340mcsc_scramble,
        )
    }
}

#[cfg(feature = "display-types")]
impl From<Option<&display_types::cea861::HdmiForumSinkCap>> for SinkTmdsFacts {
    /// An absent HF-SCDB gives [`SinkTmdsFacts::NONE`].
    fn from(cap: Option<&display_types::cea861::HdmiForumSinkCap>) -> Self {
        cap.map_or(Self::NONE, Self::from)
    }
}

/// What the source's PHY can do.
///
/// Built from the PHY's capabilities report (`From<&TmdsPhyCapabilities>`), or stated by
/// the caller with [`new`](Self::new) and the `with_…` setters.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceTmdsFacts {
    /// The source has a TMDS scrambler.
    pub scrambling: bool,
    /// The source can scramble at or below 340 Mcsc. Counts only with
    /// [`scrambling`](Self::scrambling).
    pub low_rate_scrambling: bool,
}

impl SourceTmdsFacts {
    /// Facts stated by the caller. The source is taken to have a scrambler, as in 0.1, so
    /// an existing caller's results do not change; [`with_scrambling`](Self::with_scrambling)
    /// says otherwise.
    #[must_use]
    pub const fn new(low_rate_scrambling: bool) -> Self {
        Self {
            scrambling: true,
            low_rate_scrambling,
        }
    }

    /// Sets whether the source has a TMDS scrambler.
    #[must_use]
    pub const fn with_scrambling(mut self, scrambling: bool) -> Self {
        self.scrambling = scrambling;
        self
    }
}

impl From<&hdmi_hal::phy::TmdsPhyCapabilities> for SourceTmdsFacts {
    /// The PHY's capabilities report: whether it has a scrambler, and whether it can use it
    /// at or below 340 Mcsc.
    fn from(caps: &hdmi_hal::phy::TmdsPhyCapabilities) -> Self {
        Self {
            scrambling: caps.scrambling,
            low_rate_scrambling: caps.scrambles_at_low_rate(),
        }
    }
}

/// The caller's choices where HDMI sources differ.
///
/// [`DEFAULT`](Self::DEFAULT) (also [`Default`]): low-rate scrambling permitted, threshold
/// [`ScramblingThreshold::Above340`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TmdsPolicy {
    /// Permit scrambling at or below the threshold when the source and the sink can.
    pub low_rate_scrambling: bool,
    /// Which rate first requires scrambling and 1/40.
    pub threshold: ScramblingThreshold,
}

impl TmdsPolicy {
    /// Low-rate scrambling permitted, threshold [`ScramblingThreshold::Above340`].
    pub const DEFAULT: Self = Self {
        low_rate_scrambling: true,
        threshold: ScramblingThreshold::Above340,
    };

    /// This policy with low-rate scrambling permitted or not.
    #[must_use]
    pub const fn with_low_rate_scrambling(self, low_rate_scrambling: bool) -> Self {
        Self {
            low_rate_scrambling,
            ..self
        }
    }

    /// This policy with the given threshold.
    #[must_use]
    pub const fn with_threshold(self, threshold: ScramblingThreshold) -> Self {
        Self { threshold, ..self }
    }
}

impl Default for TmdsPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The two fields of the sink's `TMDS_Config` (SCDC offset 0x20): what is written, and
/// what a read returns.
///
/// The bit layout belongs to the SCDC implementation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkTmdsConfig {
    /// `Scrambling_Enable`.
    pub scrambling_enable: bool,
    /// `TMDS_Bit_Clock_Ratio`: set for 1/40, clear for 1/10.
    pub high_tmds_clock_ratio: bool,
}

impl SinkTmdsConfig {
    /// The register's contents.
    #[must_use]
    pub const fn new(scrambling_enable: bool, high_tmds_clock_ratio: bool) -> Self {
        Self {
            scrambling_enable,
            high_tmds_clock_ratio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_facts_none_is_all_clear() {
        assert_eq!(SinkTmdsFacts::NONE, SinkTmdsFacts::new(false, false, false));
    }

    #[test]
    fn sink_facts_new_keeps_field_order() {
        let f = SinkTmdsFacts::new(true, false, true);
        assert!(f.scdc_present);
        assert!(!f.scrambling_supported);
        assert!(f.lte_340mcsc_scramble);
    }

    #[test]
    fn policy_default_matches_const() {
        assert_eq!(TmdsPolicy::default(), TmdsPolicy::DEFAULT);
        const { assert!(TmdsPolicy::DEFAULT.low_rate_scrambling) };
        assert_eq!(TmdsPolicy::DEFAULT.threshold, ScramblingThreshold::Above340);
        assert_eq!(
            ScramblingThreshold::default(),
            ScramblingThreshold::Above340
        );
    }

    #[test]
    fn policy_builders_change_one_field() {
        let p = TmdsPolicy::DEFAULT.with_low_rate_scrambling(false);
        assert!(!p.low_rate_scrambling);
        assert_eq!(p.threshold, ScramblingThreshold::Above340);

        let p = TmdsPolicy::DEFAULT.with_threshold(ScramblingThreshold::AtOrAbove340);
        assert!(p.low_rate_scrambling);
        assert_eq!(p.threshold, ScramblingThreshold::AtOrAbove340);
    }

    #[test]
    fn source_facts_and_sink_config_new() {
        assert!(SourceTmdsFacts::new(true).low_rate_scrambling);
        let c = SinkTmdsConfig::new(true, false);
        assert!(c.scrambling_enable);
        assert!(!c.high_tmds_clock_ratio);
    }

    #[test]
    fn constructors_are_const() {
        const SINK: SinkTmdsFacts = SinkTmdsFacts::new(true, true, true);
        const SOURCE: SourceTmdsFacts = SourceTmdsFacts::new(false);
        const POLICY: TmdsPolicy = TmdsPolicy::DEFAULT
            .with_low_rate_scrambling(false)
            .with_threshold(ScramblingThreshold::AtOrAbove340);
        const CONFIG: SinkTmdsConfig = SinkTmdsConfig::new(false, true);
        const { assert!(SINK.scdc_present && !SOURCE.low_rate_scrambling) };
        const { assert!(!POLICY.low_rate_scrambling && CONFIG.high_tmds_clock_ratio) };
    }

    #[cfg(feature = "display-types")]
    mod from_display_types {
        use super::*;
        use display_types::cea861::{HdmiForumFrl, HdmiForumSinkCap};

        /// A block with the three fields under test set and every other field clear.
        fn cap(scdc: bool, max_mhz: u16, lte: bool) -> HdmiForumSinkCap {
            HdmiForumSinkCap::new(
                1,
                max_mhz,
                scdc,
                false,
                false,
                false,
                lte,
                false,
                false,
                false,
                HdmiForumFrl::NotSupported,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                None,
                None,
                None,
            )
        }

        #[test]
        fn every_combination() {
            // (SCDC_Present, Max_TMDS_Character_Rate in MHz, LTE_340Mcsc_scramble)
            //   → (scdc_present, scrambling_supported, lte_340mcsc_scramble)
            let table = [
                ((false, 0, false), (false, false, false)),
                ((false, 0, true), (false, false, true)),
                ((false, 600, false), (false, false, false)),
                ((false, 600, true), (false, false, true)),
                ((true, 0, false), (true, false, false)),
                ((true, 0, true), (true, false, true)),
                ((true, 600, false), (true, true, false)),
                ((true, 600, true), (true, true, true)),
            ];
            for ((scdc, max_mhz, lte), (f_scdc, f_supported, f_lte)) in table {
                assert_eq!(
                    SinkTmdsFacts::from(&cap(scdc, max_mhz, lte)),
                    SinkTmdsFacts::new(f_scdc, f_supported, f_lte),
                    "{scdc} {max_mhz} {lte}"
                );
            }
        }

        #[test]
        fn absent_block_is_none() {
            assert_eq!(SinkTmdsFacts::from(None), SinkTmdsFacts::NONE);
            let c = cap(true, 600, true);
            assert_eq!(
                SinkTmdsFacts::from(Some(&c)),
                SinkTmdsFacts::new(true, true, true)
            );
        }
    }
}
