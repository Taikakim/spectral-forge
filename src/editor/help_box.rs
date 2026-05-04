//! Help-box widget rendered right of the FX matrix.
//!
//! Shows the module's overview when a slot is selected (no curve focused),
//! or a per-curve summary when a curve is selected.
//! See spec docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8
//! and docs/superpowers/specs/2026-05-04-past-module-ux-design.md §4.

use std::borrow::Cow;

use nih_plug_egui::egui::{self, FontId, Frame, RichText, Stroke, Ui};

use crate::dsp::modules::{module_spec, CurveLayout, ModuleType};
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

/// Render the help-box. The caller positions it in the layout (e.g. right of
/// the matrix). Width is fixed via `th::HELP_BOX_WIDTH`; height grows with
/// content.
pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) {
    let editing_slot  = (*params.editing_slot.lock() as usize).min(8);
    let editing_curve = (*params.editing_curve.lock() as usize).min(7);
    let editing_type  = params.slot_module_types.lock()[editing_slot];
    let spec          = module_spec(editing_type);

    let layout = active_layout_for_slot(editing_type, params, editing_slot);

    let head: &str = spec.display_name;
    let body = body_text(layout.as_ref(), editing_type, editing_curve, spec.curve_labels);

    let pad = th::scaled(th::HELP_BOX_PADDING, scale).round() as i8;
    Frame::new()
        .fill(th::HELP_BOX_BG)
        .stroke(Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::HELP_BOX_BORDER))
        .inner_margin(egui::Margin { left: pad, right: pad, top: pad, bottom: pad })
        .show(ui, |ui| {
            ui.set_width(th::scaled(th::HELP_BOX_WIDTH, scale));
            ui.label(
                RichText::new(head)
                    .color(th::HELP_BOX_HEAD)
                    .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_HEAD, scale))),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(body.as_ref())
                    .color(th::HELP_BOX_BODY)
                    .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_BODY, scale))),
            );
        });
}

fn active_layout_for_slot(
    ty: ModuleType,
    params: &SpectralForgeParams,
    slot: usize,
) -> Option<CurveLayout> {
    let layout_fn = module_spec(ty).active_layout?;
    let mode_byte: u8 = match ty {
        ModuleType::Past => params.slot_past_mode.lock()[slot] as u8,
        // Future modules wired analogously when their UX overhauls land.
        _ => 0,
    };
    Some(layout_fn(mode_byte))
}

/// Resolve body text per the precedence:
///
/// 1. `layout.help_for(curve_idx)` if curve is in `layout.active` and returns non-empty.
/// 2. `layout.mode_overview` if Some — covers both "curve not in active" and
///    "curve in active but help_for empty" cases.
/// 3. Static module-level description (with curve label appended when known).
///
/// Returns `Cow` so the common static-text paths (1) and (2) avoid heap alloc.
fn body_text(
    layout_opt: Option<&CurveLayout>,
    editing_type: ModuleType,
    editing_curve: usize,
    curve_labels: &'static [&'static str],
) -> Cow<'static, str> {
    if let Some(layout) = layout_opt {
        if layout.active.contains(&(editing_curve as u8)) {
            let s = (layout.help_for)(editing_curve as u8);
            if !s.is_empty() {
                return Cow::Borrowed(s);
            }
        }
        if let Some(overview) = layout.mode_overview {
            return Cow::Borrowed(overview);
        }
    }
    static_description(editing_type, editing_curve, curve_labels)
}

/// Module-level fallback when no `active_layout` is provided OR when
/// `active_layout` returns no help for the focused curve. Falls through to
/// the module's `display_name` for unhandled `ModuleType`s — friendlier than
/// blank for new modules that haven't yet shipped a description.
fn static_description(
    ty: ModuleType,
    editing_curve: usize,
    curve_labels: &'static [&'static str],
) -> Cow<'static, str> {
    let module_text: &'static str = match ty {
        ModuleType::Past => "Past — Read-only access to a rolling buffer of recent spectral history. \
                             Pick a mode (right-click the slot) to choose how the buffer is replayed: \
                             Granular freezes selected bins by age, DecaySorter rearranges bins by how \
                             long they ring, Convolution blends current with delayed self, Reverse plays \
                             the buffer backward, Stretch plays it at variable speed.",
        ModuleType::Empty    => "No module assigned to this slot.",
        ModuleType::Master   => "Master output sums all routed slots into the plugin output.",
        ModuleType::Dynamics => "Per-bin dynamics processor.",
        ModuleType::Freeze   => "Spectral freeze — captures a moment of the spectrum and holds it.",
        _ => module_spec(ty).display_name,
    };
    if let Some(label) = curve_labels.get(editing_curve) {
        if !label.is_empty() {
            return Cow::Owned(format!("{}\n\nCurve: {}", module_text, label));
        }
    }
    Cow::Borrowed(module_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always_help(idx: u8) -> &'static str {
        match idx {
            0 => "curve-0-help",
            2 => "curve-2-help",
            _ => "",
        }
    }

    fn empty_help(_: u8) -> &'static str { "" }

    #[test]
    fn body_text_help_for_wins_when_curve_active_and_non_empty() {
        let layout = CurveLayout {
            active:          &[0, 2, 4],
            label_overrides: &[],
            help_for:        always_help,
            mode_overview:   Some("mode-overview"),
        };
        let body = body_text(Some(&layout), ModuleType::Past, 0, &["A", "B", "C", "D", "E"]);
        assert_eq!(body, "curve-0-help");
    }

    #[test]
    fn body_text_falls_through_to_mode_overview_when_curve_not_active() {
        let layout = CurveLayout {
            active:          &[0, 4],          // curve 2 NOT in active
            label_overrides: &[],
            help_for:        always_help,
            mode_overview:   Some("mode-overview"),
        };
        let body = body_text(Some(&layout), ModuleType::Past, 2, &["A", "B", "C", "D", "E"]);
        assert_eq!(body, "mode-overview");
    }

    #[test]
    fn body_text_falls_through_to_mode_overview_when_help_empty() {
        let layout = CurveLayout {
            active:          &[0, 1, 2],
            label_overrides: &[],
            help_for:        empty_help,
            mode_overview:   Some("mode-overview"),
        };
        let body = body_text(Some(&layout), ModuleType::Past, 1, &["A", "B", "C"]);
        assert_eq!(body, "mode-overview");
    }

    #[test]
    fn body_text_falls_through_to_static_when_no_layout() {
        let body = body_text(None, ModuleType::Past, 0, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
        // Past static description starts with "Past —"; curve label appended.
        assert!(body.starts_with("Past —"));
        assert!(body.contains("Curve: AMOUNT"));
    }
}
