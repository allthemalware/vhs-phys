//! Composite video: what a television's Y/C separator does to the deck's output.
//!
//! The deck's decoded luma and chroma are summed into one composite signal on the NTSC
//! subcarrier and separated again, the way a TV connected by a composite cable does. A
//! notch separator leaks fine luma detail into colour ("rainbow", cross-colour); a comb
//! separator leaks vertical colour edges into luma ("hanging dots", cross-luma); both
//! show the line- and frame-alternating dot pattern of the subcarrier ("dot crawl").
//! The 4 x fsc baseband grid makes the modulation trivial: the subcarrier advances 90
//! degrees per sample, 180 degrees per line (227.5 cycles) and 180 degrees per frame.

use crate::video::Baseband;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompositeDecoder {
    /// Luma notch at fsc and chroma band-pass: cross-colour on fine luma detail.
    #[default]
    Notch,
    /// One-line comb: separates by adding/subtracting adjacent lines.
    Comb1h,
    /// Two-line (three-line adaptive-free) comb.
    Comb2h,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CompositeParams {
    /// Off (the default): S-Video style output, no composite artefacts.
    pub enabled: bool,
    pub decoder: CompositeDecoder,
}

impl Default for CompositeParams {
    fn default() -> Self {
        Self { enabled: false, decoder: CompositeDecoder::Notch }
    }
}

/// Encode one frame's luma/chroma (in place) to a composite signal and decode it again
/// with the chosen separator. `frame_index` is the absolute frame number: the subcarrier
/// phase pattern repeats every two frames. Only the active picture is processed; sync,
/// blanking and burst are left alone.
pub fn roundtrip(bb: &mut Baseband, frame_index: i64, p: &CompositeParams) {
    let _ = (bb, frame_index, p);
    unimplemented!("composite round trip")
}
