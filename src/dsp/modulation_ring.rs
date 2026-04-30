//! Per-node modulation ring state bank.
//!
//! `RingStateBank` stores `ModRingState` for every (slot, curve, node) triple
//! in a fixed-size array — no heap allocation, RT-safe clone.
//!
//! Key count: 9 slots × 7 curves × 6 nodes = 378.

use crate::editor::mod_ring::{ModRingState, ModRingToggle};

/// Total number of (slot, curve, node) triples.
pub const RING_KEY_COUNT: usize = 9 * 7 * 6;

/// Identifies one (slot, curve, node) triple.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingKey {
    pub slot:  u8,
    pub curve: u8,
    pub node:  u8,
}

/// Flat fixed-size array storing `ModRingState` for every (slot, curve, node).
///
/// Clone is a 378-byte `memcpy` — no heap involved. All methods are O(1) except
/// `entry_count` and `iter`, which are O(378).
#[derive(Clone)]
pub struct RingStateBank {
    entries: [ModRingState; RING_KEY_COUNT],
}

impl Default for RingStateBank {
    fn default() -> Self {
        Self { entries: [ModRingState::default(); RING_KEY_COUNT] }
    }
}

impl RingStateBank {
    /// Flat index for a given key. Panics in debug on out-of-range.
    #[inline]
    pub fn key_index(key: RingKey) -> usize {
        (key.slot as usize) * (7 * 6)
            + (key.curve as usize) * 6
            + (key.node as usize)
    }

    /// Number of entries that have at least one toggle set.
    pub fn entry_count(&self) -> usize {
        self.entries.iter().filter(|s| !s.is_empty()).count()
    }

    /// Read the state for a key.
    pub fn get(&self, key: RingKey) -> ModRingState {
        self.entries[Self::key_index(key)]
    }

    /// Set or clear a single toggle for a key.
    pub fn set_toggle(&mut self, key: RingKey, t: ModRingToggle, on: bool) {
        let idx = Self::key_index(key);
        let s = &mut self.entries[idx];
        if on { s.set(t); } else { s.clear(t); }
    }

    /// Iterate over non-empty (key, state) pairs without allocating.
    pub fn iter(&self) -> impl Iterator<Item = (RingKey, ModRingState)> + '_ {
        self.entries.iter().enumerate().filter_map(|(i, s)| {
            if s.is_empty() {
                None
            } else {
                let slot  = (i / (7 * 6)) as u8;
                let rem   = i % (7 * 6);
                let curve = (rem / 6) as u8;
                let node  = (rem % 6) as u8;
                Some((RingKey { slot, curve, node }, *s))
            }
        })
    }

    /// Reset all entries to the default (all toggles off).
    pub fn clear_all(&mut self) {
        for s in self.entries.iter_mut() {
            *s = ModRingState::default();
        }
    }
}

// ─── Audio-thread ring transform state ───────────────────────────────────────

/// Per-key audio-thread state for the modulation ring transform.
///
/// Lives exclusively on the Pipeline (audio thread); never shared with the GUI.
/// All sentinel values use `f32::NAN` or `-1.0` so that "not yet latched" is
/// detectable without an extra boolean field.
///
/// `RingTransformState` is `Copy` so a full `[RingTransformState; RING_KEY_COUNT]`
/// reset is a single `[Default::default(); RING_KEY_COUNT]` bulk copy — no heap
/// allocation, RT-safe.
#[derive(Clone, Copy, Debug)]
pub struct RingTransformState {
    latched_value:   f32,  // f32::NAN  = "not yet latched"
    last_latch_beat: f32,  // -1.0      = "never latched"
    prev_out_value:  f32,  // f32::NAN  = "no previous output"
}

impl Default for RingTransformState {
    fn default() -> Self {
        Self {
            latched_value:   f32::NAN,
            last_latch_beat: -1.0,
            prev_out_value:  f32::NAN,
        }
    }
}

impl RingTransformState {
    /// Returns `true` once a value has been latched.
    #[inline]
    pub fn is_latched(&self) -> bool {
        !self.latched_value.is_nan()
    }

    /// Returns the latched value, or `0.0` if not yet latched.
    #[inline]
    pub fn latched_value(&self) -> f32 {
        if self.latched_value.is_nan() { 0.0 } else { self.latched_value }
    }

    /// Returns the beat position at which the last latch occurred (`-1.0` if never).
    #[inline]
    pub fn last_latch_beat(&self) -> f32 {
        self.last_latch_beat
    }

    /// Record a new latch: capture `value` and stamp `beat`.
    #[inline]
    pub fn set_latched(&mut self, value: f32, beat: f32) {
        self.latched_value   = value;
        self.last_latch_beat = beat;
    }

    /// Store the most recent output value (used for Legato interpolation).
    #[inline]
    pub fn set_prev_out(&mut self, value: f32) {
        self.prev_out_value = value;
    }

    /// Returns the previous output value, or `0.0` if none has been stored.
    #[inline]
    pub fn prev_out(&self) -> f32 {
        if self.prev_out_value.is_nan() { 0.0 } else { self.prev_out_value }
    }
}
