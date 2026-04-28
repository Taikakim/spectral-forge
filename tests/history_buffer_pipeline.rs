use num_complex::Complex;
use spectral_forge::dsp::history_buffer::HistoryBuffer;

const NUM_BINS: usize = 1025;
const NUM_HOPS: usize = 200;

fn synthesize_frame(hop: usize) -> Vec<Complex<f32>> {
    // Bin 100 = stable sine; bin 200 = decaying ring; bin 300 = noise.
    let mut frame = vec![Complex::new(0.0, 0.0); NUM_BINS];
    frame[100] = Complex::new(1.0, 0.0);
    let env = (1.0 - hop as f32 / 50.0).max(0.0);
    frame[200] = Complex::from_polar(env, hop as f32 * 0.1);
    let noise_phase = (hop as f32 * 137.0).sin();
    frame[300] = Complex::from_polar(0.5, noise_phase);
    frame
}

#[test]
fn history_buffer_under_pipeline_load_is_finite_and_bounded() {
    let mut h = HistoryBuffer::new(2, 100, NUM_BINS);
    for hop in 0..NUM_HOPS {
        let frame = synthesize_frame(hop);
        h.write_hop(0, &frame);
        h.write_hop(1, &frame);
        h.advance_after_all_channels_written();
        h.clear_summary_cache();

        // Every 25 hops: poll the summary stats and validate.
        if hop % 25 == 24 {
            let decay = h.summary_decay_estimate(0);
            let decay_vec: Vec<f32> = decay.to_vec();
            drop(decay);
            let rms   = h.summary_rms_envelope(0);
            let rms_vec: Vec<f32> = rms.to_vec();
            drop(rms);
            let stab  = h.summary_if_stability(0);
            let stab_vec: Vec<f32> = stab.to_vec();
            drop(stab);
            for k in 0..NUM_BINS {
                assert!(decay_vec[k].is_finite(), "decay[{}] non-finite at hop {}", k, hop);
                assert!(rms_vec[k].is_finite(),   "rms[{}] non-finite at hop {}", k, hop);
                assert!(stab_vec[k].is_finite(),  "stab[{}] non-finite at hop {}", k, hop);
                assert!(decay_vec[k] >= 0.0 && decay_vec[k] <= 1000.0);
                assert!(rms_vec[k] >= 0.0 && rms_vec[k] <= 10.0);
                assert!(stab_vec[k] >= 0.0 && stab_vec[k] <= 1.0);
            }
        }
    }
    assert_eq!(h.frames_used(), 100, "buffer must saturate at capacity after enough writes");
}

#[test]
fn history_buffer_read_frame_returns_most_recent_after_full_load() {
    let mut h = HistoryBuffer::new(1, 50, NUM_BINS);
    for hop in 0..NUM_HOPS {
        let frame = synthesize_frame(hop);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }
    let most_recent = h.read_frame(0, 0).expect("most recent frame must exist");
    let expected = synthesize_frame(NUM_HOPS - 1);
    for k in 0..NUM_BINS {
        let dre = (most_recent[k].re - expected[k].re).abs();
        let dim = (most_recent[k].im - expected[k].im).abs();
        assert!(dre < 1e-5 && dim < 1e-5,
            "bin {}: most_recent ({}, {}) != expected ({}, {})",
            k, most_recent[k].re, most_recent[k].im, expected[k].re, expected[k].im);
    }
}

#[test]
fn history_buffer_summary_caches_repeat_calls() {
    let mut h = HistoryBuffer::new(1, 50, NUM_BINS);
    for hop in 0..NUM_HOPS {
        let frame = synthesize_frame(hop);
        h.write_hop(0, &frame);
        h.advance_after_all_channels_written();
    }
    let _warm = h.summary_decay_estimate(0); // first call computes
    drop(_warm);
    // Without invalidating, a second call should produce identical numerics.
    let a = h.summary_decay_estimate(0);
    let snapshot: Vec<f32> = a.to_vec();
    drop(a);
    let b = h.summary_decay_estimate(0);
    for k in 0..NUM_BINS {
        assert_eq!(snapshot[k], b[k], "cached value must be stable bin {}", k);
    }
}
