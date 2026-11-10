//! The inputs and register contents: `ClockRatio`, `ScramblingThreshold`, `SinkTmdsFacts`,
//! `SourceTmdsFacts`, `TmdsPolicy`, `SinkTmdsConfig`, `TmdsConfigReading` and
//! `ScramblingStatus`.

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
    /// The sink declares FRL (a non-zero `Max_FRL_Rate`), so its `Config_1` may hold FRL
    /// settings a bring-up clears first.
    pub frl_capable: bool,
}

impl SinkTmdsFacts {
    /// A sink without HDMI Forum capabilities: no SCDC and no scrambling.
    pub const NONE: Self = Self::new(false, false, false);

    /// Facts stated by the caller, with [`frl_capable`](Self::frl_capable) taken to be
    /// `scdc_present`.
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
            frl_capable: scdc_present,
        }
    }

    /// Sets whether the sink declares FRL. [`new`](Self::new) takes a sink with SCDC to, so
    /// a caller that never learns of this fact still gets the FRL clear before a bring-up:
    /// on a sink without FRL, a write to a register it does not use, where `false` would
    /// skip the clear silently. FRL needs SCDC, and the clear runs only on an SCDC sink.
    #[must_use]
    pub const fn with_frl_capable(mut self, frl_capable: bool) -> Self {
        self.frl_capable = frl_capable;
        self
    }
}

#[cfg(feature = "display-types")]
impl From<&display_types::cea861::HdmiForumSinkCap> for SinkTmdsFacts {
    /// `scrambling_supported` is `scdc_present` and a non-zero `max_tmds_rate_mhz`;
    /// `frl_capable` is a non-zero `max_frl_rate`; the other two facts are copied.
    fn from(cap: &display_types::cea861::HdmiForumSinkCap) -> Self {
        Self::new(
            cap.scdc_present,
            cap.scdc_present && cap.max_tmds_rate_mhz != 0,
            cap.lte_340mcsc_scramble,
        )
        .with_frl_capable(cap.max_frl_rate != display_types::cea861::HdmiForumFrl::NotSupported)
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
    /// The source's own highest TMDS character rate, in kHz.
    pub max_character_rate_khz: u32,
}

impl SourceTmdsFacts {
    /// Facts stated by the caller. The source is taken to have a scrambler and no rate
    /// ceiling of its own, as in 0.1, so an existing caller's results do not change;
    /// [`with_scrambling`](Self::with_scrambling) and
    /// [`with_max_character_rate_khz`](Self::with_max_character_rate_khz) say otherwise.
    #[must_use]
    pub const fn new(low_rate_scrambling: bool) -> Self {
        Self {
            scrambling: true,
            low_rate_scrambling,
            max_character_rate_khz: u32::MAX,
        }
    }

    /// Sets the source's own highest TMDS character rate, in kHz.
    #[must_use]
    pub const fn with_max_character_rate_khz(mut self, max_character_rate_khz: u32) -> Self {
        self.max_character_rate_khz = max_character_rate_khz;
        self
    }

    /// Sets whether the source has a TMDS scrambler.
    #[must_use]
    pub const fn with_scrambling(mut self, scrambling: bool) -> Self {
        self.scrambling = scrambling;
        self
    }
}

impl From<&hdmi_hal::phy::TmdsPhyCapabilities> for SourceTmdsFacts {
    /// The PHY's capabilities report: whether it has a scrambler, whether it can use it at
    /// or below 340 Mcsc, and its highest character rate.
    fn from(caps: &hdmi_hal::phy::TmdsPhyCapabilities) -> Self {
        Self {
            scrambling: caps.scrambling,
            low_rate_scrambling: caps.scrambles_at_low_rate(),
            max_character_rate_khz: caps.max_character_rate_khz,
        }
    }
}

/// The caller's choices where HDMI sources differ.
///
/// [`DEFAULT`](Self::DEFAULT) (also [`Default`]): low-rate scrambling permitted, threshold
/// [`ScramblingThreshold::Above340`], `TMDS_Config` written to any sink with SCDC.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TmdsPolicy {
    /// Permit scrambling at or below the threshold when the source and the sink can.
    pub low_rate_scrambling: bool,
    /// Which rate first requires scrambling and 1/40.
    pub threshold: ScramblingThreshold,
    /// Which sinks get the `TMDS_Config` write on a plain link.
    pub tmds_config_gate: TmdsConfigGate,
}

/// Which sinks get the `TMDS_Config` write on a plain link. A scrambled link always writes
/// it: the sink must be told.
///
/// The two differ only for a sink with SCDC that does not support scrambling, which never
/// runs at 1/40.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TmdsConfigGate {
    /// Any sink with SCDC (the default, chosen).
    #[default]
    AnyScdcSink,
    /// Only a sink that supports scrambling.
    ScramblingSink,
}

impl TmdsPolicy {
    /// Low-rate scrambling permitted, threshold [`ScramblingThreshold::Above340`].
    pub const DEFAULT: Self = Self {
        low_rate_scrambling: true,
        threshold: ScramblingThreshold::Above340,
        tmds_config_gate: TmdsConfigGate::AnyScdcSink,
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

    /// This policy with the given `TMDS_Config` gate.
    #[must_use]
    pub const fn with_tmds_config_gate(self, tmds_config_gate: TmdsConfigGate) -> Self {
        Self {
            tmds_config_gate,
            ..self
        }
    }
}

impl Default for TmdsPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The two fields of the sink's `TMDS_Config` (SCDC offset 0x20): what is written, and
/// what a read decodes to.
///
/// The bit layout belongs to the SCDC implementation. A read keeps the byte it decoded
/// beside this value, in [`TmdsConfigReading`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkTmdsConfig {
    /// `Scrambling_Enable`.
    pub scrambling_enable: bool,
    /// `TMDS_Bit_Clock_Ratio`: set for 1/40, clear for 1/10.
    pub high_tmds_clock_ratio: bool,
}

impl SinkTmdsConfig {
    /// The register's two fields.
    #[must_use]
    pub const fn new(scrambling_enable: bool, high_tmds_clock_ratio: bool) -> Self {
        Self {
            scrambling_enable,
            high_tmds_clock_ratio,
        }
    }
}

/// What a read of the sink's `TMDS_Config` returned: the two fields the SCDC
/// implementation decoded, and the byte as read, reserved bits included, so a sink that
/// sets them stays visible to a diagnostic tool.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TmdsConfigReading {
    /// The two fields.
    pub config: SinkTmdsConfig,
    /// The byte as read.
    pub raw: u8,
}

impl TmdsConfigReading {
    /// The fields an SCDC implementation decoded from `raw`, the byte it read.
    #[must_use]
    pub const fn new(config: SinkTmdsConfig, raw: u8) -> Self {
        Self { config, raw }
    }
}

/// What the sink reports in `Scrambling_Status` (SCDC offset 0x21): whether it detects
/// scrambled TMDS, and the byte that was read.
///
/// The byte keeps whatever the sink reports beside the status bit, so noise or reserved
/// bits stay visible to a diagnostic tool; the bit layout belongs to the SCDC
/// implementation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScramblingStatus {
    /// The sink detects scrambled TMDS.
    pub set: bool,
    raw: u8,
}

impl ScramblingStatus {
    /// The status as an SCDC implementation decoded it from `raw`, the byte it read.
    #[must_use]
    pub const fn new(set: bool, raw: u8) -> Self {
        Self { set, raw }
    }

    /// The byte that was read.
    #[must_use]
    pub const fn raw(&self) -> u8 {
        self.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_facts_none_is_all_clear() {
        assert_eq!(SinkTmdsFacts::NONE, SinkTmdsFacts::new(false, false, false));
        const { assert!(!SinkTmdsFacts::NONE.frl_capable) };
        // `new` takes an SCDC sink to declare FRL, so the FRL clear is not skipped silently.
        assert!(SinkTmdsFacts::new(true, true, true).frl_capable);
        assert!(!SinkTmdsFacts::new(false, true, true).frl_capable);
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
    fn a_reading_keeps_the_byte_beside_the_fields() {
        const BOTH: SinkTmdsConfig = SinkTmdsConfig::new(true, true);
        let reading = TmdsConfigReading::new(BOTH, 0xA3);
        assert_eq!(reading.config, BOTH);
        assert_eq!(reading.raw, 0xA3);
        assert_ne!(reading, TmdsConfigReading::new(BOTH, 0x03));
        // The fields compare structurally, so a const is a pattern, as in 0.1.
        assert!(matches!(reading.config, BOTH));
    }

    #[test]
    fn scrambling_status_keeps_the_byte() {
        const STATUS: ScramblingStatus = ScramblingStatus::new(true, 0x81);
        const { assert!(STATUS.set) };
        assert_eq!(STATUS.raw(), 0x81);
        assert_ne!(STATUS, ScramblingStatus::new(true, 0x01));
    }

    #[test]
    fn constructors_are_const() {
        const SINK: SinkTmdsFacts = SinkTmdsFacts::new(true, true, true);
        const SOURCE: SourceTmdsFacts = SourceTmdsFacts::new(false);
        const POLICY: TmdsPolicy = TmdsPolicy::DEFAULT
            .with_low_rate_scrambling(false)
            .with_threshold(ScramblingThreshold::AtOrAbove340);
        const CONFIG: SinkTmdsConfig = SinkTmdsConfig::new(false, true);
        const READ: TmdsConfigReading = TmdsConfigReading::new(CONFIG, 0x02);
        const { assert!(READ.config.high_tmds_clock_ratio && READ.raw == 0x02) };
        const { assert!(SINK.scdc_present && !SOURCE.low_rate_scrambling) };
        const { assert!(!POLICY.low_rate_scrambling && CONFIG.high_tmds_clock_ratio) };
    }

    #[cfg(feature = "display-types")]
    mod from_display_types {
        use super::*;
        use display_types::cea861::{HdmiForumFrl, HdmiForumSinkCap};

        /// A block with the three fields under test set and every other field clear.
        fn cap(scdc: bool, max_mhz: u16, lte: bool) -> HdmiForumSinkCap {
            cap_with_frl(scdc, max_mhz, lte, HdmiForumFrl::NotSupported)
        }

        fn cap_with_frl(
            scdc: bool,
            max_mhz: u16,
            lte: bool,
            frl: HdmiForumFrl,
        ) -> HdmiForumSinkCap {
            HdmiForumSinkCap::new(
                1, max_mhz, scdc, false, false, false, lte, false, false, false, frl, false, false,
                false, false, false, false, false, false, false, false, false, None, None, None,
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
                    SinkTmdsFacts::new(f_scdc, f_supported, f_lte).with_frl_capable(false),
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
                SinkTmdsFacts::new(true, true, true).with_frl_capable(false)
            );
        }

        #[test]
        fn frl_capable_follows_max_frl_rate() {
            let frl = cap_with_frl(true, 600, false, HdmiForumFrl::Rate6Gbps4Lanes);
            assert!(SinkTmdsFacts::from(&frl).frl_capable);
            assert!(!SinkTmdsFacts::from(&cap(true, 600, false)).frl_capable);
        }
    }
}
