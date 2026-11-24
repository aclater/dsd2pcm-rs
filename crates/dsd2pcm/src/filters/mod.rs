// SPDX-License-Identifier: MIT OR Apache-2.0
//! The DSD64 to PCM filters to choose from ([`Filter`]).

pub(crate) mod pysacd;
mod pysacd_taps;

/// The DSD64 to PCM filter and quantiser.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// scarletbook-pcm 0.7.0's filters (every rate and [`crate::Cutoff`]), rounded without
    /// dither: the same bytes as scarletbook-pcm.
    #[default]
    Default,
    /// pysacd's filter: three Kaiser-windowed FIR stages in `f32` and a TPDF-dithered
    /// 24-bit quantiser, bit-exact with pysacd's reference build (`csrc/dsp.c` compiled with
    /// `-O2 -ffp-contract=off`), each area one stream. 88.2 and 176.4 kHz only, with the
    /// cutoff left at [`crate::Cutoff::Standard`] (the response is fixed). The dither is
    /// seeded per Super Audio CD frame from its index in the area; files count frames from
    /// their start, the frames API from [`crate::frames::Run::at_frame`].
    Pysacd,
    /// The precise filter: the 30, 40 kHz and wide cutoffs at every rate, like
    /// [`Filter::Default`], with exact integer arithmetic, the same linear phase and centring,
    /// and the same quantiser (rounded, not dithered), but at least 150 dB of stopband
    /// wherever ultrasonic content would fold into the passband (scarletbook-pcm's filters:
    /// about 110 dB) and less passband ripple. It costs more CPU time; see
    /// `docs/pcm-quality.md`.
    Precise,
}

impl Filter {
    /// The decimator's taps for the filters that run on it.
    pub(crate) fn design(self) -> crate::decimator::Design {
        match self {
            Filter::Precise => crate::decimator::Design::Precise,
            _ => crate::decimator::Design::Default,
        }
    }
}
