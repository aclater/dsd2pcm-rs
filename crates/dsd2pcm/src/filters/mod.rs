// SPDX-License-Identifier: MIT OR Apache-2.0
//! The DSD64 to PCM filters to choose from ([`Filter`]).

pub(crate) mod pysacd;
mod pysacd_taps;

/// The DSD64 to PCM filter and quantiser. [`Filter::default`] is [`Filter::Precise`].
///
/// The variants name what a filter is for, not which one is the default: [`Filter::Fast`]
/// is the default of 0.1.x (scarletbook-pcm 0.7.0's filters), still byte-identical to
/// scarletbook-pcm, and [`Filter::Precise`] is the default since 0.2.0.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// The precise filter (the default): the 30, 40 kHz and wide cutoffs at every rate,
    /// like [`Filter::Fast`], with exact integer arithmetic, the same linear phase and
    /// centring, and the same quantiser (rounded, not dithered), but at least 150 dB of
    /// stopband wherever ultrasonic content would fold into the passband (scarletbook-pcm's
    /// filters: about 110 dB) and less passband ripple. SINAD with a quiet source 142 to
    /// 145 dB, against about 122 dB for [`Filter::Fast`]; it costs about 1.8 times the CPU
    /// time of [`Filter::Fast`] on one core (1.5 to 2.3 times). See `docs/pcm-quality.md`
    /// and `docs/performance.md`.
    #[default]
    Precise,
    /// Lower CPU, lower quality: scarletbook-pcm 0.7.0's filters (every rate and
    /// [`crate::Cutoff`]), rounded without dither, the same bytes as scarletbook-pcm. The
    /// default of 0.1.x. Its stopband (about 110 dB) limits SINAD to about 122 dB with a
    /// quiet source (116 dB with the wide cutoff); see `docs/pcm-quality.md`.
    Fast,
    /// pysacd's filter: its three Kaiser-windowed FIR stages and TPDF-dithered 24-bit
    /// quantiser, computed in `f64` (pysacd computes in `f32`), the same bytes at every SIMD
    /// level, each area one stream. 88.2 and 176.4 kHz only, with the
    /// cutoff left at [`crate::Cutoff::Standard`] (the response is fixed). The dither is
    /// seeded per Super Audio CD frame from its index in the area; files count frames from
    /// their start, the frames API from [`crate::frames::Run::at_frame`].
    Pysacd,
}

impl Filter {
    /// The decimator's taps for the filters that run on it.
    pub(crate) fn design(self) -> crate::decimator::Design {
        match self {
            Filter::Precise => crate::decimator::Design::Precise,
            Filter::Fast | Filter::Pysacd => crate::decimator::Design::Fast,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Filter;

    #[test]
    fn default_is_precise() {
        assert_eq!(Filter::default(), Filter::Precise);
        assert_eq!(
            crate::PcmOptions::new(crate::PcmRate::Hz88200).filter,
            Filter::Precise
        );
        let cfg = crate::frames::PcmConfig::new(crate::PcmRate::Hz88200, crate::Cutoff::Low, 2);
        assert_eq!(cfg.filter, Filter::Precise);
    }
}
