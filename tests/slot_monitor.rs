//! Live slot monitor statistics (dsp::slot_monitor).

use num_complex::Complex;
use spectral_forge::dsp::pipeline::MAX_NUM_BINS;
use spectral_forge::dsp::slot_monitor::*;

fn publish(m: &SlotMonitor) -> Vec<f32> {
    let mut buf = vec![0.0f32; MONITOR_LEN];
    m.publish_into(3, &mut buf);
    buf
}

#[test]
fn identity_reports_no_change() {
    let mut m = SlotMonitor::new();
    let bins: Vec<Complex<f32>> = (0..1025).map(|k| Complex::new(1.0 + k as f32 * 0.01, 0.5)).collect();
    m.capture_pre(&bins);
    m.observe_post(&bins);
    let s = publish(&m);
    let st = &s[2 * MAX_NUM_BINS..];
    assert!(st[STAT_GAIN_DB].abs() < 1e-4);
    assert!(st[STAT_DELTA_DB] < -200.0, "no change energy");
    assert_eq!(st[STAT_SPREAD_PCT], 0.0);
    assert_eq!(st[STAT_SLOT], 3.0);
    assert_eq!(st[STAT_NUM_BINS], 1025.0);
    assert!((s[10] - bins[10].norm()).abs() < 1e-6, "input peak published");
}

#[test]
fn doubling_reports_six_db_everywhere_and_clear_resets() {
    let mut m = SlotMonitor::new();
    let bins = vec![Complex::new(1.0f32, 0.0); 513];
    let out: Vec<Complex<f32>> = bins.iter().map(|b| b * 2.0).collect();
    for _ in 0..4 {
        m.capture_pre(&bins);
        m.observe_post(&out);
    }
    let st = publish(&m)[2 * MAX_NUM_BINS..].to_vec();
    assert!((st[STAT_GAIN_DB] - 6.0206).abs() < 1e-3);
    assert!((st[STAT_MAX_BIN_GAIN_DB] - 6.0206).abs() < 1e-3);
    assert_eq!(st[STAT_SPREAD_PCT], 100.0);
    assert_eq!(st[STAT_HOPS], 4.0);

    m.clear();
    assert_eq!(m.hops(), 0);
    assert!(publish(&m)[..MAX_NUM_BINS].iter().all(|&v| v == 0.0));
}
