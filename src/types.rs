//! The inputs and register contents: `ClockRatio`, `ScramblingThreshold`, `SinkTmdsFacts`,
//! `SourceTmdsFacts`, `TmdsPolicy` and `SinkTmdsConfig`.

/// The TMDS bit clock ratio: the TMDS clock runs at the bit rate divided by 10 or by 40.
///
/// Exhaustive: the sink's `TMDS_Bit_Clock_Ratio` is a single bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockRatio {
    /// The TMDS clock is the bit rate / 10 (the character rate).
    Div10,
    /// The TMDS clock is the bit rate / 40 (a quarter of the character rate).
    Div40,
}

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
    /// The sink can take scrambling above 340 Mcsc: SCDC present and a non-zero
    /// `Max_TMDS_Character_Rate`, as Linux derives it.
    pub scrambling_above_340: bool,
    /// The sink can take scrambling at or below 340 Mcsc (`LTE_340Mcsc_scramble`).
    pub lte_340mcsc_scramble: bool,
}

impl SinkTmdsFacts {
    /// A sink without HDMI Forum capabilities: no SCDC and no scrambling.
    pub const NONE: Self = Self::new(false, false, false);

    /// Facts stated by the caller.
    #[must_use]
    pub const fn new(
        scdc_present: bool,
        scrambling_above_340: bool,
        lte_340mcsc_scramble: bool,
    ) -> Self {
        Self {
            scdc_present,
            scrambling_above_340,
            lte_340mcsc_scramble,
        }
    }
}

#[cfg(feature = "display-types")]
impl From<&display_types::cea861::HdmiForumSinkCap> for SinkTmdsFacts {
    /// `scrambling_above_340` is `scdc_present` and a non-zero `max_tmds_rate_mhz`;
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
/// In 0.1 the caller states it; from 0.2 it is built from the PHY's capabilities report.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceTmdsFacts {
    /// The source can scramble at or below 340 Mcsc.
    pub low_rate_scrambling: bool,
}

impl SourceTmdsFacts {
    /// Facts stated by the caller.
    #[must_use]
    pub const fn new(low_rate_scrambling: bool) -> Self {
        Self {
            low_rate_scrambling,
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
        assert!(!f.scrambling_above_340);
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
            for scdc in [false, true] {
                for max_mhz in [0, 600] {
                    for lte in [false, true] {
                        let f = SinkTmdsFacts::from(&cap(scdc, max_mhz, lte));
                        assert_eq!(f.scdc_present, scdc);
                        assert_eq!(f.scrambling_above_340, scdc && max_mhz != 0);
                        assert_eq!(f.lte_340mcsc_scramble, lte);
                    }
                }
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
