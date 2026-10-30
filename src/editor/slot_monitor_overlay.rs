//! Live slot monitor overlay — dev-build only.
//!
//! Draws the edited slot's input (dim) and output (slot colour) peak spectra
//! over the curve view, plus change statistics: overall gain, change energy
//! (phase-aware), spread (share of bins changed by > ~1 dB), loudest bin
//! gain and spill (energy in bins the input left silent). A toggle in the
//! curve's top-right corner starts and stops the audio-side measurement.
//! Data comes from `dsp::slot_monitor` via a triple buffer.

#![cfg(feature = "dev-build")]

use std::sync::atomic::Ordering;

use nih_plug_egui::egui::{self, Color32, Pos2, Rect, Stroke, Ui};

use crate::bridge::MonitorLink;
use crate::dsp::pipeline::MAX_NUM_BINS;
use crate::dsp::slot_monitor::*;
use crate::editor::curve::{db_inner_rect, freq_to_x_max};
use crate::editor::theme as th;

#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut Ui,
    curve_rect: Rect,
    link: &MonitorLink,
    edit_slot: usize,
    slot_color: Color32,
    sample_rate: f32,
    fft_size: usize,
    db_min: f32,
    db_max: f32,
    scale: f32,
) {
    // Toggle in the top-right corner of the curve view.
    let mut on = link.enabled.load(Ordering::Relaxed);
    let toggle_rect = Rect::from_min_size(
        Pos2::new(
            curve_rect.right() - th::scaled(th::MONITOR_TOGGLE_W + th::MONITOR_INSET, scale),
            curve_rect.top() + th::scaled(th::MONITOR_INSET, scale),
        ),
        egui::vec2(th::scaled(th::MONITOR_TOGGLE_W, scale), th::scaled(th::MONITOR_TOGGLE_H, scale)),
    );
    let resp = ui.put(
        toggle_rect,
        egui::Checkbox::new(
            &mut on,
            egui::RichText::new("Monitor").size(th::scaled(th::FONT_SIZE_LABEL, scale)).color(th::LABEL_DIM),
        ),
    );
    if resp.changed() {
        link.enabled.store(on, Ordering::Relaxed);
    }
    crate::editor::help_box::track_help_strings(
        ui, &resp, "Slot monitor",
        "Live input (dim) vs output (slot colour) peak spectra of the edited slot, with gain, change, spread, loudest-bin gain and spill. Dev builds only.",
    );
    if !on { return; }

    let Some(mut rx) = link.rx.try_lock() else { return; };
    let data = rx.read();
    if data.len() < MONITOR_LEN { return; }
    let stats = &data[2 * MAX_NUM_BINS..];
    if stats[STAT_HOPS] < 1.0 || stats[STAT_SLOT] as usize != edit_slot { return; }
    let n = (stats[STAT_NUM_BINS] as usize).min(MAX_NUM_BINS).min(fft_size / 2 + 1);

    let painter = ui.painter();
    let input_col = th::LABEL_DIM;
    paint_mags(painter, curve_rect, &data[..n], input_col, th::STROKE_THIN, sample_rate, fft_size, db_min, db_max, scale);
    paint_mags(painter, curve_rect, &data[MAX_NUM_BINS..MAX_NUM_BINS + n], slot_color, th::STROKE_THIN, sample_rate, fft_size, db_min, db_max, scale);

    let text = format!(
        "gain {:+.1} dB · change {:.1} dB · spread {:.0}% · max bin {:+.1} dB · spill {:.1} dB",
        stats[STAT_GAIN_DB], stats[STAT_DELTA_DB], stats[STAT_SPREAD_PCT],
        stats[STAT_MAX_BIN_GAIN_DB].max(-99.0), stats[STAT_SPILL_DB].max(-99.0),
    );
    painter.text(
        Pos2::new(
            curve_rect.right() - th::scaled(2.0 * th::MONITOR_INSET, scale),
            curve_rect.top() + th::scaled(th::MONITOR_TEXT_Y, scale),
        ),
        egui::Align2::RIGHT_TOP,
        text,
        egui::FontId::proportional(th::scaled(th::FONT_SIZE_LABEL, scale)),
        th::LABEL_DIM,
    );
}

/// Raw FFT magnitudes → dBFS (×4/FFT, full-scale sine = 0 dB) on the spectrum axis.
#[allow(clippy::too_many_arguments)]
fn paint_mags(
    painter: &egui::Painter,
    rect: Rect,
    mags: &[f32],
    color: Color32,
    stroke: f32,
    sample_rate: f32,
    fft_size: usize,
    db_min: f32,
    db_max: f32,
    scale: f32,
) {
    if mags.len() < 2 || fft_size == 0 { return; }
    let inner = db_inner_rect(rect, scale);
    let max_hz = (sample_rate / 2.0).max(20_001.0);
    let norm = 4.0 / fft_size as f32;
    let range = (db_max - db_min).max(1.0);
    let s = Stroke::new(th::scaled_stroke(stroke, scale), color);
    let mut prev: Option<Pos2> = None;
    for (k, &m) in mags.iter().enumerate().skip(1) {
        let f_hz = (k as f32 * sample_rate / fft_size as f32).max(20.0);
        let x = freq_to_x_max(f_hz, max_hz, rect);
        let db = 20.0 * (m * norm).max(1e-12).log10();
        let y = inner.max.y - ((db - db_min) / range).clamp(0.0, 1.0) * inner.height();
        let p = Pos2::new(x, y);
        if let Some(q) = prev { painter.line_segment([q, p], s); }
        prev = Some(p);
    }
}
