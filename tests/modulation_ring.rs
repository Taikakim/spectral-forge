use spectral_forge::dsp::modulation_ring::{
    apply_ring, crossed_tick_at_beat, RingApplyArgs, RingKey, RingStateBank, RingTransformState,
    RING_KEY_COUNT,
};
use spectral_forge::editor::mod_ring::{ModRingState, ModRingToggle};

/// The bank must start with all entries empty (all flags zero).
#[test]
fn bank_starts_empty() {
    let bank = RingStateBank::default();
    assert_eq!(bank.entry_count(), 0, "fresh bank must have zero non-empty entries");
}

/// set_toggle / get round-trips a single toggle without disturbing others.
#[test]
fn set_and_get_round_trip() {
    let mut bank = RingStateBank::default();
    let key = RingKey { slot: 3, curve: 2, node: 4 };

    // Slot 3 / curve 2 / node 4 — set SampleHold on.
    bank.set_toggle(key, ModRingToggle::SampleHold, true);
    let s = bank.get(key);
    assert!(s.is_set(ModRingToggle::SampleHold), "SampleHold should be set");
    assert!(!s.is_set(ModRingToggle::Sync16),    "Sync16 should remain clear");
    assert!(!s.is_set(ModRingToggle::Legato),    "Legato should remain clear");

    // A different key must be unaffected.
    let other = RingKey { slot: 0, curve: 0, node: 0 };
    assert!(bank.get(other).is_empty(), "unrelated key must stay empty");
}

/// iter() yields exactly the non-empty entries and recovers the original key.
#[test]
fn iter_yields_non_empty_entries_only() {
    let mut bank = RingStateBank::default();

    let k1 = RingKey { slot: 0, curve: 0, node: 0 };
    let k2 = RingKey { slot: 8, curve: 6, node: 5 };

    bank.set_toggle(k1, ModRingToggle::Legato,     true);
    bank.set_toggle(k2, ModRingToggle::Sync16,     true);

    let entries: Vec<_> = bank.iter().collect();
    assert_eq!(entries.len(), 2, "iter must yield exactly 2 non-empty entries");

    let keys: Vec<RingKey> = entries.iter().map(|(k, _)| *k).collect();
    assert!(keys.contains(&k1), "k1 must appear in iter");
    assert!(keys.contains(&k2), "k2 must appear in iter");

    // Verify the recovered states.
    for (k, s) in &entries {
        if *k == k1 {
            assert!(s.is_set(ModRingToggle::Legato), "k1 state must have Legato set");
        } else {
            assert!(s.is_set(ModRingToggle::Sync16), "k2 state must have Sync16 set");
        }
    }
}

// ─── Legato ramp tests ───────────────────────────────────────────────────────

/// With Legato on and a previous output value stored, apply_ring must produce a
/// linear ramp from `prev_out` → `target` across the block, with the final
/// sample landing exactly on `target`.
#[test]
fn legato_ramps_between_latched_values() {
    let mut state = RingTransformState::default();

    // Build a ring state with SampleHold + Legato set; Sync16 off (period = 1 beat).
    let mut ring = ModRingState::default();
    ring.set(ModRingToggle::SampleHold);
    ring.set(ModRingToggle::Legato);

    // First block: input 0.0 → latches 0.0, no prev_out yet → step-fill (NaN guard).
    let args0 = RingApplyArgs {
        ring,
        input_value:   0.0,
        current_beat:  0.0,
        block_samples: 4,
    };
    let mut out0 = [f32::NAN; 4];
    apply_ring(&mut state, args0, &mut out0);
    // prev_out was NaN → no ramp, step fill with 0.0
    assert_eq!(out0, [0.0, 0.0, 0.0, 0.0], "first block: no prev → step fill");

    // Second block: beat moves to 1.1 → tick at 1.0 crossed → re-latch to 1.0.
    // prev_out = 0.0; target = 1.0; n = 4.
    // Expected ramp: 0.25, 0.50, 0.75, 1.00
    let args1 = RingApplyArgs {
        ring,
        input_value:   1.0,
        current_beat:  1.1,
        block_samples: 4,
    };
    let mut out1 = [f32::NAN; 4];
    apply_ring(&mut state, args1, &mut out1);
    let expected = [0.25_f32, 0.50, 0.75, 1.00];
    for (i, (&got, &exp)) in out1.iter().zip(expected.iter()).enumerate() {
        assert!((got - exp).abs() < 1e-6, "sample {i}: got {got}, expected {exp}");
    }
}

/// With Legato OFF, a new latched value must appear as an immediate step change
/// (all samples equal to the target) — same as the Task 4 behaviour.
#[test]
fn legato_off_means_step_change() {
    let mut state = RingTransformState::default();

    // SampleHold on, Legato OFF.
    let mut ring = ModRingState::default();
    ring.set(ModRingToggle::SampleHold);

    // First block: latch 0.0.
    let args0 = RingApplyArgs {
        ring,
        input_value:   0.0,
        current_beat:  0.0,
        block_samples: 4,
    };
    let mut out0 = [f32::NAN; 4];
    apply_ring(&mut state, args0, &mut out0);

    // Second block: cross tick → latch 1.0; Legato off → step fill.
    let args1 = RingApplyArgs {
        ring,
        input_value:   1.0,
        current_beat:  1.1,
        block_samples: 4,
    };
    let mut out1 = [f32::NAN; 4];
    apply_ring(&mut state, args1, &mut out1);
    assert_eq!(out1, [1.0, 1.0, 1.0, 1.0], "without Legato, must be step fill");
}

/// Total number of addressable keys is 9 * 7 * 6 = 378.
#[test]
fn ring_key_count_is_correct() {
    assert_eq!(RING_KEY_COUNT, 9 * 7 * 6);
    assert_eq!(RING_KEY_COUNT, 378);
}

/// clear_all resets a populated bank back to zero entries.
#[test]
fn clear_all_empties_bank() {
    let mut bank = RingStateBank::default();
    bank.set_toggle(RingKey { slot: 1, curve: 1, node: 1 }, ModRingToggle::SampleHold, true);
    bank.set_toggle(RingKey { slot: 7, curve: 5, node: 3 }, ModRingToggle::Sync16,     true);
    assert_eq!(bank.entry_count(), 2);

    bank.clear_all();
    assert_eq!(bank.entry_count(), 0, "clear_all must empty the bank");
}

/// A freshly default-constructed `RingTransformState` must be in the "not yet latched" state.
#[test]
fn ring_transform_state_default_is_unlatched() {
    let s = RingTransformState::default();
    assert!(!s.is_latched());
    assert_eq!(s.latched_value(), 0.0);
    assert_eq!(s.last_latch_beat(), -1.0);
}

// ─── apply_ring tests ─────────────────────────────────────────────────────────

/// With S/H on and Sync16 off, the first block (unlatched state) latches the
/// input value and holds it across all 64 samples. A second call whose beat
/// does not cross a quarter-note boundary must not update the latched value.
#[test]
fn sample_hold_latches_first_value_holds_until_next_tick() {
    let mut state = RingTransformState::default();

    // Build a ring state with SampleHold set, Sync16 clear.
    let mut ring = ModRingState::default();
    ring.set(ModRingToggle::SampleHold);

    // First block: beat 0.0, input 0.5.
    let args0 = RingApplyArgs {
        ring,
        input_value:   0.5,
        current_beat:  0.0,
        block_samples: 64,
    };
    let mut out0 = [0.0_f32; 64];
    apply_ring(&mut state, args0, &mut out0);
    assert_eq!(out0[0],  0.5, "first sample should be the latched value");
    assert_eq!(out0[63], 0.5, "last sample should also be the latched value");

    // Second block: beat moves to 0.10 (period = 1.0 beat, no tick crossed).
    // Input changes to 0.9, but the held value should remain 0.5.
    let args1 = RingApplyArgs {
        ring,
        input_value:   0.9,
        current_beat:  0.10,
        block_samples: 64,
    };
    let mut out1 = [0.0_f32; 64];
    apply_ring(&mut state, args1, &mut out1);
    assert_eq!(out1[0],  0.5, "value must remain held (no tick crossed)");
    assert_eq!(out1[63], 0.5, "value must remain held across all samples");
}

/// With no toggles set, `apply_ring` is a pure pass-through: every sample
/// equals `input_value` and the state latches on every call.
#[test]
fn no_toggles_means_pure_passthrough() {
    let mut state = RingTransformState::default();
    let ring = ModRingState::default(); // all toggles off

    // First call: input 0.3.
    let args0 = RingApplyArgs {
        ring,
        input_value:   0.3,
        current_beat:  0.0,
        block_samples: 64,
    };
    let mut out0 = [0.0_f32; 64];
    apply_ring(&mut state, args0, &mut out0);
    assert_eq!(out0[0],  0.3);
    assert_eq!(out0[63], 0.3);

    // Second call: input changes to 0.7.  No S/H → value is immediately passed through.
    let args1 = RingApplyArgs {
        ring,
        input_value:   0.7,
        current_beat:  0.10,
        block_samples: 64,
    };
    let mut out1 = [0.0_f32; 64];
    apply_ring(&mut state, args1, &mut out1);
    assert_eq!(out1[0],  0.7, "without S/H, new input must pass through immediately");
    assert_eq!(out1[63], 0.7);
}

// ─── Sync 1/16 tick math ─────────────────────────────────────────────────────

#[test]
fn sync_16_tick_math_no_cross() {
    // 16th-note period = 0.25 beats.
    // last_beat = 1.0, current_beat = 1.20 → no boundary crossed.
    assert!(!crossed_tick_at_beat(1.0, 1.20, 0.25));
}

#[test]
fn sync_16_tick_math_cross_one() {
    // last = 1.0, current = 1.30 → crossed 1.25.
    assert!(crossed_tick_at_beat(1.0, 1.30, 0.25));
}

#[test]
fn sync_16_tick_math_first_block_treats_as_cross() {
    // last_beat = -1.0 (never latched) ⇒ first valid block always crosses.
    assert!(crossed_tick_at_beat(-1.0, 0.5, 0.25));
}

#[test]
fn sync_16_tick_math_handles_loop_wrap() {
    // Bitwig loops: last_beat = 3.95 (end of bar), current_beat = 0.05 (next bar start).
    // Treat any backward jump as a cross.
    assert!(crossed_tick_at_beat(3.95, 0.05, 0.25));
}

/// Regression guard (Task 7): the bank wired into the editor is the same bank
/// the Pipeline reads — a lookup by node must return what was stored.
#[test]
fn ring_state_bank_lookup_by_node_returns_set_state() {
    use spectral_forge::dsp::modulation_ring::{RingStateBank, RingKey};
    use spectral_forge::editor::ModRingToggle;
    let mut bank = RingStateBank::default();
    let key = RingKey { slot: 3, curve: 1, node: 4 };
    bank.set_toggle(key, ModRingToggle::Legato, true);
    assert!(bank.get(key).is_set(ModRingToggle::Legato));
}
