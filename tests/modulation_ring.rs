use spectral_forge::dsp::modulation_ring::{RingKey, RingStateBank, RING_KEY_COUNT};
use spectral_forge::editor::mod_ring::ModRingToggle;

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
