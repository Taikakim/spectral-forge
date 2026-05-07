//! Help-box widget rendered right of the FX matrix.
//!
//! Three-tier help precedence:
//! 1. **Per-widget topic** — set transiently by `track_help()` while the user
//!    hovers/drags a parameter. Cleared at the top of every frame.
//! 2. **Per-curve / per-mode help** — derived from the focused slot's
//!    `module_spec().active_layout` for the focused curve.
//! 3. **Module fallback** — static module-level description with curve label.
//!
//! See spec docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §8
//! and docs/superpowers/specs/2026-05-04-past-module-ux-design.md §4.

use std::borrow::Cow;

use nih_plug_egui::egui::{self, FontId, Frame, RichText, Stroke, Ui};

use crate::dsp::modules::{module_spec, CurveLayout, ModuleType};
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

// ── HelpTopic — context-sensitive help focus ───────────────────────────────

/// A discrete help topic identifying which parameter or graph element the
/// user is currently interacting with. Set by `track_help()` per frame and
/// consumed by `draw()` to populate the help-box body.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HelpTopic {
    // Top bar
    PresetMenu,
    GraphCeil,
    PeakFalloff,
    ResetToDefault,
    HelpToggle,
    FftSize,
    UiScale,

    // Curve area
    CurveTab,
    CurveGraph,
    CurveNode,
    SlotName,

    // Sidechain strip
    ScGain,
    ScChannel,

    // Mode buttons
    ModuleMode,
    PastSortKey,
    ModulateRepel,
    ModulateScPositioned,

    // Master row
    InputGain,
    OutputGain,
    Mix,
    AutoMakeup,
    DeltaMonitor,
    MasterClip,
    MasterClipThreshold,

    // Dynamics panel knobs
    DynAttack,
    DynRelease,
    DynSensitivity,
    DynSuppressionWidth,

    // Per-curve transforms
    Offset,
    Tilt,
    Curvature,

    // Routing matrix
    MatrixCellSend,
    MatrixSlotSelect,
}

/// Map a topic to its help text.
pub fn topic_help_text(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::PresetMenu       => "Presets — load, save, rename, or browse the patch library. Files live in the user preset folder.",
        HelpTopic::GraphCeil        => "Spectrum display ceiling (dB). Sets the top of the visible curve area; only affects the display, not the audio.",
        HelpTopic::PeakFalloff      => "Peak-hold falloff (ms). 0 = no hold; higher values keep peaks visible longer in the spectrum overlay.",
        HelpTopic::ResetToDefault   => "Reset every parameter to its factory default and clear all module state. Cannot be undone.",
        HelpTopic::HelpToggle       => "Show or hide context-sensitive help in this panel.",
        HelpTopic::FftSize          => "FFT size — analysis window length. Larger windows give finer frequency resolution but slower transient response and more latency. Bitwig compensates the latency automatically.",
        HelpTopic::UiScale          => "UI zoom factor — scales the entire window.",

        HelpTopic::CurveTab         => "Click to switch which curve you're editing for this slot. The active curve is what the graph below shows and what the Offset/Tilt/Curv sliders modify.",
        HelpTopic::CurveGraph       => "Drag the dots to shape this curve. The X axis is frequency (log, 20 Hz–Nyquist); the Y axis is the curve's parameter (label shown on the left). Outer dots are shelves, inner dots are bell filters. Right-click a dot to flatten it.",
        HelpTopic::CurveNode        => "Drag this node to change the curve's value at this frequency. Drag past the top/bottom edge to reach the virtual −2..+2 range; a red triangle marks the off-rect direction.",
        HelpTopic::SlotName         => "Click to rename this slot. The name shows up in the routing matrix labels.",

        HelpTopic::ScGain           => "Sidechain input gain (dB) for this slot. −∞ disables the slot's sidechain entirely. Only relevant for SC-aware modules (Dynamics, etc.).",
        HelpTopic::ScChannel        => "Source for this slot's sidechain: Follow uses the host's SC bus; L+R sums; L/R/M/S taps an individual stream.",

        HelpTopic::ModuleMode       => "Module mode — switches between sub-algorithms within this module. Each mode rewires which curves are active and may add extra controls.",
        HelpTopic::PastSortKey      => "DecaySorter sort key — chooses which spectral feature determines bin reordering: by age, by magnitude, by spectral centroid, etc.",
        HelpTopic::ModulateRepel    => "Reverse the gravitational force in Modulate's Gravity mode (push instead of pull).",
        HelpTopic::ModulateScPositioned => "Use sidechain bins to position the gravity well in Modulate's Gravity mode.",

        HelpTopic::InputGain        => "Pre-FX input gain. Applied before any analysis or processing.",
        HelpTopic::OutputGain       => "Post-FX output gain. Applied after the wet/dry mix.",
        HelpTopic::Mix              => "Wet/dry blend. 0% = full dry (true bypass — bit-perfect). 100% = full wet.",
        HelpTopic::AutoMakeup       => "Auto makeup — automatically compensates for level loss from compression/expansion in dynamics-style modules.",
        HelpTopic::DeltaMonitor     => "Delta monitor — output only the difference between dry and wet (what the chain removed). Useful for hearing exactly what the FX is doing.",
        HelpTopic::MasterClip       => "Master soft-clip — final-stage saturation that limits output to a controlled ceiling. Off bypasses the clipper entirely.",
        HelpTopic::MasterClipThreshold => "Master clip threshold (dB). Signal above this magnitude gets soft-saturated; quieter content passes untouched.",

        HelpTopic::DynAttack        => "Dynamics global attack time (ms). Per-curve ATTACK multiplies this value, so this is the baseline at neutral curve gain.",
        HelpTopic::DynRelease       => "Dynamics global release time (ms). Per-curve RELEASE multiplies this value, so this is the baseline at neutral curve gain.",
        HelpTopic::DynSensitivity   => "Dynamics envelope follower sensitivity. Higher values track faster onsets but are more prone to chattering.",
        HelpTopic::DynSuppressionWidth => "Width of the per-bin suppression footprint. Wider = the bin's gain reduction also pulls down its neighbours, smoothing the spectral result.",

        HelpTopic::Offset           => "Offset — shifts the entire curve up or down. v=0 sits at the curve's natural value; v=±1 reaches the curve's display min/max via the calibrated axis_aware_lerp.",
        HelpTopic::Tilt             => "Tilt — frequency-dependent shift pivoted at 1 kHz. Negative tilts the curve down at high frequencies; positive tilts it up. Up to ±2 dB/oct.",
        HelpTopic::Curvature        => "Curvature — bends the tilt into a smoothstep S-curve, concentrating the change around the 1 kHz pivot. 0 = straight tilt; 1 = full S.",

        HelpTopic::MatrixCellSend   => "Routing matrix cell — send amplitude from this row's slot to this column's slot. 0 = off, 1 = unity. Drag to set; right-click for the amp popup.",
        HelpTopic::MatrixSlotSelect => "Click to focus this slot in the curve editor above. Right-click to assign a different module type.",
    }
}

const HELP_TOPIC_KEY: &str = "spectral_forge::help_topic";

fn topic_id() -> egui::Id { egui::Id::new(HELP_TOPIC_KEY) }

/// Track that `response` is being hovered/dragged/focused. If so, claim the
/// per-frame help focus for `topic`. Multiple widgets may call this in a
/// single frame; the last claim wins.
pub fn track_help(ui: &egui::Ui, response: &egui::Response, topic: HelpTopic) {
    if response.hovered() || response.dragged() || response.has_focus() {
        ui.ctx().data_mut(|d| d.insert_temp::<Option<HelpTopic>>(topic_id(), Some(topic)));
    }
}

/// Clear the per-frame help focus. Call once at the very top of the editor
/// frame so stale focus from a previous frame doesn't leak through.
pub fn reset_focus(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp::<Option<HelpTopic>>(topic_id(), None));
}

fn current_topic(ctx: &egui::Context) -> Option<HelpTopic> {
    ctx.data(|d| d.get_temp::<Option<HelpTopic>>(topic_id())).flatten()
}

// ── Render ────────────────────────────────────────────────────────────────

/// Render the help-box. The caller positions it in the layout (e.g. right of
/// the matrix). Width is fixed via `th::HELP_BOX_WIDTH`; height grows with
/// content. Renders a blank frame when `params.help_enabled` is false so the
/// layout doesn't collapse.
pub fn draw(ui: &mut Ui, params: &SpectralForgeParams, scale: f32) {
    let editing_slot  = (*params.editing_slot.lock() as usize).min(8);
    let editing_curve = (*params.editing_curve.lock() as usize).min(7);
    let editing_type  = params.slot_module_types.lock()[editing_slot];
    let spec          = module_spec(editing_type);

    let layout = active_layout_for_slot(editing_type, params, editing_slot);

    let help_on = params.help_enabled.value();
    let topic   = current_topic(ui.ctx());

    let (head, body): (&str, Cow<'static, str>) = if !help_on {
        ("Help (off)", Cow::Borrowed("Toggle the HELP button in the top bar to bring help back."))
    } else if let Some(t) = topic {
        (topic_head(t), Cow::Borrowed(topic_help_text(t)))
    } else {
        let h = spec.display_name;
        let b = body_text(layout.as_ref(), editing_type, editing_curve, spec.curve_labels);
        (h, b)
    };

    let pad = th::scaled(th::HELP_BOX_PADDING, scale).round() as i8;
    Frame::new()
        .fill(th::HELP_BOX_BG)
        .stroke(Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::HELP_BOX_BORDER))
        .inner_margin(egui::Margin { left: pad, right: pad, top: pad, bottom: pad })
        .show(ui, |ui| {
            ui.set_width(th::scaled(th::HELP_BOX_WIDTH, scale));
            ui.add(
                egui::Label::new(
                    RichText::new(head)
                        .color(th::HELP_BOX_HEAD)
                        .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_HEAD, scale))),
                ).wrap(),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(body.as_ref())
                        .color(th::HELP_BOX_BODY)
                        .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_BODY, scale))),
                ).wrap(),
            );
        });
}

/// Short heading shown above the body when a topic is focused.
fn topic_head(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::PresetMenu       => "Presets",
        HelpTopic::GraphCeil        => "Spectrum Ceiling",
        HelpTopic::PeakFalloff      => "Peak Falloff",
        HelpTopic::ResetToDefault   => "Reset to Default",
        HelpTopic::HelpToggle       => "Help",
        HelpTopic::FftSize          => "FFT Size",
        HelpTopic::UiScale          => "UI Scale",

        HelpTopic::CurveTab         => "Curve Selector",
        HelpTopic::CurveGraph       => "Curve Editor",
        HelpTopic::CurveNode        => "Curve Node",
        HelpTopic::SlotName         => "Slot Name",

        HelpTopic::ScGain           => "Sidechain Gain",
        HelpTopic::ScChannel        => "Sidechain Source",

        HelpTopic::ModuleMode       => "Module Mode",
        HelpTopic::PastSortKey      => "DecaySorter Key",
        HelpTopic::ModulateRepel    => "Modulate · Repel",
        HelpTopic::ModulateScPositioned => "Modulate · SC-Pos",

        HelpTopic::InputGain        => "Input Gain",
        HelpTopic::OutputGain       => "Output Gain",
        HelpTopic::Mix              => "Mix",
        HelpTopic::AutoMakeup       => "Auto Makeup",
        HelpTopic::DeltaMonitor     => "Delta Monitor",
        HelpTopic::MasterClip       => "Master Clip",
        HelpTopic::MasterClipThreshold => "Master Clip Threshold",

        HelpTopic::DynAttack        => "Dynamics · Attack",
        HelpTopic::DynRelease       => "Dynamics · Release",
        HelpTopic::DynSensitivity   => "Dynamics · Sensitivity",
        HelpTopic::DynSuppressionWidth => "Dynamics · Width",

        HelpTopic::Offset           => "Curve Offset",
        HelpTopic::Tilt             => "Curve Tilt",
        HelpTopic::Curvature        => "Curve Curvature",

        HelpTopic::MatrixCellSend   => "Matrix Send",
        HelpTopic::MatrixSlotSelect => "Slot",
    }
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

    #[test]
    fn topic_help_text_covers_all_variants() {
        // A spot-check that key topics resolve to non-empty strings.
        for t in [
            HelpTopic::FftSize, HelpTopic::Mix, HelpTopic::Offset,
            HelpTopic::Tilt, HelpTopic::Curvature, HelpTopic::CurveGraph,
            HelpTopic::MatrixCellSend, HelpTopic::HelpToggle,
        ] {
            assert!(!topic_help_text(t).is_empty(), "{:?} has empty help text", t);
            assert!(!topic_head(t).is_empty(),     "{:?} has empty head", t);
        }
    }
}
