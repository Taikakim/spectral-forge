//! Per-slot panel widget for the `RhythmModule`.
//!
//! Lives below the curve editor area (see `editor_ui.rs` panel-dispatch site)
//! and is wired through `ModuleSpec::panel_widget` for `ModuleType::Rhythm`
//! at `dsp/modules/mod.rs`. Renders an 8-voice × 8-step grid of clickable
//! cells the user paints to drive `RhythmModule`'s Arpeggiator mode. The
//! grid is persisted on `SpectralForgeParams::slot_arp_grid[slot]`; the
//! current rhythm mode label is read from `slot_rhythm_mode[slot]`.
//!
//! The widget runs on the GUI thread, so blocking `Mutex::lock()` is fine
//! here. The audio thread never sees this code path.

use nih_plug_egui::egui::{self, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::dsp::modules::{module_spec, ModuleType};
use crate::dsp::modules::rhythm::ArpGrid;
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

const CELL_SIZE: f32 = 18.0;
const PAD:       f32 =  2.0;
const VOICES:    usize = 8;
const STEPS:     usize = 8;

/// Render the per-slot Rhythm panel: a mode label and the 8×8 Arpeggiator step grid.
///
/// Signature matches `crate::dsp::modules::PanelWidgetFn`.
pub fn render(ui: &mut Ui, params: &SpectralForgeParams, slot: usize) {
    if slot >= 9 { return; }

    let scale = *params.ui_scale.lock();

    // Snapshot mode label without holding the mutex across the rest of the frame.
    let mode = params.slot_rhythm_mode.lock()[slot];
    ui.label(format!("Mode: {}", mode.label()));

    // Snapshot the grid so we don't hold the lock across egui interaction calls.
    let mut grid: ArpGrid = params.slot_arp_grid.lock()[slot];
    let mut changed = false;

    let total_w = STEPS as f32 * (CELL_SIZE + PAD) - PAD;
    let total_h = VOICES as f32 * (CELL_SIZE + PAD) - PAD;

    let (rect, _resp) =
        ui.allocate_exact_size(Vec2::new(total_w, total_h), Sense::hover());
    let painter = ui.painter_at(rect);
    let origin  = rect.min;

    let rhy_spec = module_spec(ModuleType::Rhythm);
    let cell_lit = rhy_spec.color_lit;
    let cell_dim = rhy_spec.color_dim;

    for v in 0..VOICES {
        for s in 0..STEPS {
            let cell_rect = Rect::from_min_size(
                origin + egui::vec2(
                    s as f32 * (CELL_SIZE + PAD),
                    v as f32 * (CELL_SIZE + PAD),
                ),
                Vec2::splat(CELL_SIZE),
            );

            let active = grid.voice_active_at(v, s);
            let fill = if active { cell_lit } else { cell_dim };
            let stroke = Stroke::new(th::scaled_stroke(th::STROKE_HAIRLINE, scale), th::GRID_LINE);
            painter.rect(cell_rect, 2.0, fill, stroke, StrokeKind::Outside);

            let id = ui.id().with(("rhythm_arp_cell", slot, v, s));
            let resp = ui.interact(cell_rect, id, Sense::click());
            if resp.clicked() {
                grid.toggle(v, s);
                changed = true;
            }
        }
    }

    if changed {
        params.slot_arp_grid.lock()[slot] = grid;
    }
}
