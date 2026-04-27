use serde::{Deserialize, Serialize};

/// Per-cell amp mode for the routing matrix. Each cell of `RouteMatrix` carries
/// one of these to select what kind of non-linear processing is applied to the
/// signal as it travels along that send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AmpMode {
    #[default]
    Linear,
    Vactrol,
    Schmitt,
    Slew,
    Stiction,
}

impl AmpMode {
    /// Human-readable name; used by the cell popup and any future tooltips.
    pub fn label(self) -> &'static str {
        match self {
            AmpMode::Linear   => "Linear",
            AmpMode::Vactrol  => "Vactrol",
            AmpMode::Schmitt  => "Schmitt",
            AmpMode::Slew     => "Slew",
            AmpMode::Stiction => "Stiction",
        }
    }
}

/// Per-cell numeric parameters shared across all amp modes. Each mode reads only
/// the subset relevant to it (e.g. `Vactrol` ignores `slew_db_per_s`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AmpCellParams {
    /// Strength of the amp effect. 0 = bypass, 1 = full, >1 = exaggerated. Range 0..2.
    pub amount: f32,
    /// Schmitt on-threshold (0..1 magnitude); also the Stiction step size.
    pub threshold: f32,
    /// Vactrol release time in milliseconds.
    pub release_ms: f32,
    /// Slew maximum change rate in dB per second.
    pub slew_db_per_s: f32,
}

// Manual `Default` impl rather than `#[derive(Default)]`: every field has a
// non-zero neutral value, and `f32::default()` would silently produce a fully-
// silent cell. Keeping the values inline here also serves as the spec.
impl Default for AmpCellParams {
    fn default() -> Self {
        Self { amount: 1.0, threshold: 0.5, release_ms: 100.0, slew_db_per_s: 60.0 }
    }
}
