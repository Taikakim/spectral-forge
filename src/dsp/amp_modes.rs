use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AmpCellParams {
    pub amount:        f32,   // 0..2 — strength of the amp effect (0 = bypass, 1 = full, >1 = exaggerated)
    pub threshold:     f32,   // 0..1 magnitude — Schmitt on-threshold; Stiction step
    pub release_ms:    f32,   // Vactrol release time
    pub slew_db_per_s: f32,   // Slew max change rate
}

impl Default for AmpCellParams {
    fn default() -> Self {
        Self { amount: 1.0, threshold: 0.5, release_ms: 100.0, slew_db_per_s: 60.0 }
    }
}
