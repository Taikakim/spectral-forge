//! Modulation Ring overlay (S/H, Sync, Legato).
//!
//! Phase 1 scaffolding — the widget and the state machine ship now;
//! the toggles only become active once BPM/sync infra (Phase 4) is in
//! place. See `ideas/next-gen-modules/01-global-infrastructure.md` § 8.

use nih_plug_egui::egui::{self, Color32, Pos2, Stroke, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModRingToggle {
    SampleHold,
    Sync16,
    Legato,
}

impl ModRingToggle {
    pub fn bit(self) -> u8 {
        match self {
            ModRingToggle::SampleHold => 0b001,
            ModRingToggle::Sync16     => 0b010,
            ModRingToggle::Legato     => 0b100,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModRingState {
    flags: u8,
}

impl ModRingState {
    pub fn is_set(&self, t: ModRingToggle) -> bool { (self.flags & t.bit()) != 0 }
    pub fn toggle(&mut self, t: ModRingToggle)   { self.flags ^= t.bit(); }
    pub fn set(&mut self, t: ModRingToggle)      { self.flags |= t.bit(); }
    pub fn clear(&mut self, t: ModRingToggle)    { self.flags &= !t.bit(); }
    pub fn is_empty(&self) -> bool               { self.flags == 0 }
    /// Phase 6.7 activates the ring toggles unconditionally. The audio thread
    /// reads ring state each block via `RingStateBank`; with no transport running,
    /// `crossed_tick_at_beat` with `last_beat=-1` always returns `true`, so the
    /// ring latches immediately and scales by 1.0 — a safe pass-through.
    pub fn toggles_enabled(&self) -> bool { true }
}

/// Draw the modulation ring overlay around an anchor point. Returns the
/// toggle that was clicked this frame, or None.
///
/// Interact IDs are derived from `key` (slot/curve/node) so the widget retains
/// identity across window resize, anchor reposition, and DPI changes — pixel
/// coordinates are not stable between frames and would invalidate egui's
/// per-widget click state.
pub fn mod_ring_overlay(
    ui:     &mut Ui,
    center: Pos2,
    key:    crate::dsp::modulation_ring::RingKey,
    state:  &ModRingState,
) -> Option<ModRingToggle> {
    let radius = crate::editor::theme::MOD_RING_RADIUS;
    let dot_radius = crate::editor::theme::MOD_RING_DOT_RADIUS;
    let painter = ui.painter();

    // Three dots at 12, 4, and 8 o'clock.
    let positions = [
        (ModRingToggle::SampleHold, Pos2::new(center.x, center.y - radius)),
        (ModRingToggle::Sync16,     Pos2::new(center.x + radius * 0.866, center.y + radius * 0.5)),
        (ModRingToggle::Legato,     Pos2::new(center.x - radius * 0.866, center.y + radius * 0.5)),
    ];

    let mut clicked = None;
    let enabled = state.toggles_enabled();
    for (toggle, pos) in positions {
        let lit = state.is_set(toggle);
        let fill = match (lit, enabled) {
            (true, true)   => crate::editor::theme::MOD_RING_LIT,
            (false, true)  => crate::editor::theme::MOD_RING_DIM,
            (_, false)     => crate::editor::theme::MOD_RING_DISABLED,
        };
        painter.circle_filled(pos, dot_radius, fill);
        painter.circle_stroke(pos, dot_radius, Stroke::new(1.0, Color32::BLACK));

        let hit = ui.interact(
            egui::Rect::from_center_size(pos, egui::vec2(dot_radius * 2.0, dot_radius * 2.0)),
            ui.id().with(("mod_ring", key.slot, key.curve, key.node, toggle as u8)),
            egui::Sense::click(),
        );
        if hit.clicked() && enabled {
            clicked = Some(toggle);
        }
    }

    clicked
}
