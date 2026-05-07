use nih_plug_egui::egui::{self, Pos2, Ui};
use crate::dsp::modules::{module_spec, GainMode, ModuleType};
use crate::editor::theme as th;
use crate::params::SpectralForgeParams;

/// Ephemeral state for the module assignment popup.
/// Stored in egui temp data under key `ui.id().with("module_popup")`.
#[derive(Clone)]
pub struct PopupState {
    pub open: bool,
    pub slot: usize,
    pub pos:  Pos2,
}

impl Default for PopupState {
    fn default() -> Self {
        Self { open: false, slot: 0, pos: Pos2::ZERO }
    }
}

/// Count how many T/S Split modules are currently active across all slots.
fn ts_split_count(types: &[ModuleType; 9]) -> usize {
    types.iter().filter(|&&t| t == ModuleType::TransientSustainedSplit).count()
}

const ASSIGNABLE: &[ModuleType] = &[
    ModuleType::Dynamics,
    ModuleType::Freeze,
    ModuleType::PhaseSmear,
    ModuleType::Contrast,
    ModuleType::Gain,
    ModuleType::MidSide,
    ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic,
    ModuleType::Future,
    ModuleType::Punch,
    ModuleType::Rhythm,
    ModuleType::Geometry,
    ModuleType::Modulate,
    ModuleType::Circuit,
    ModuleType::Past,
    ModuleType::Kinetics,
    ModuleType::Harmony,
];

/// Render the popup if open. Call every frame from the main UI closure.
/// Returns `Some(slot)` if a module was just assigned (curves need republishing), else `None`.
///
/// UI parameter contract: see docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4
/// `scale` is the frame-scoped UI scale factor; font sizes flow through `th::scaled`.
pub fn show_popup(
    ui:     &mut Ui,
    params: &SpectralForgeParams,
    scale:  f32,
) -> Option<usize> {
    let key = ui.id().with("module_popup");
    let state: PopupState = ui.data(|d| d.get_temp(key).unwrap_or_default());
    if !state.open { return None; }

    let types = *params.slot_module_types.lock();
    let ts_count = ts_split_count(&types);
    let slot = state.slot;

    let mut assigned_slot: Option<usize> = None;
    let mut new_state = state.clone();

    let area_response = egui::Area::new(egui::Id::new("module_popup_area"))
        .fixed_pos(state.pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(140.0);
                ui.label(
                    egui::RichText::new("Assign module")
                        .color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_LABEL, scale))
                );
                ui.separator();

                for &ty in ASSIGNABLE {
                    let spec = module_spec(ty);
                    let is_ts = ty == ModuleType::TransientSustainedSplit;
                    let ts_full = is_ts && ts_count >= 2 && types[slot] != ty;
                    let enabled = !ts_full;

                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(10.0, 10.0), egui::Sense::hover()
                            );
                            ui.painter().rect_filled(rect, 2.0, spec.color_lit);
                            let resp = ui.button(spec.display_name);
                            crate::editor::help_box::track_help_strings(
                                ui, &resp, spec.display_name, module_browse_help(ty),
                            );
                            if resp.clicked() {
                                assign_module(params, slot, ty);
                                new_state.open = false;
                                assigned_slot = Some(slot);
                            }
                        });
                    });

                    if ts_full {
                        ui.label(
                            egui::RichText::new("(max 2 active)")
                                .color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_TINY, scale))
                        );
                    }
                }

                ui.separator();
                let remove_resp = ui.button("Remove module");
                crate::editor::help_box::track_help_strings(
                    ui, &remove_resp, "Remove module",
                    "Set this slot to Empty. Curves and transforms reset, the slot's audio passes through unchanged, and any sends from/to it stop carrying signal.",
                );
                if remove_resp.clicked() {
                    assign_module(params, slot, ModuleType::Empty);
                    new_state.open = false;
                    assigned_slot = Some(slot);
                }

                ui.separator();
                ui.label(
                    egui::RichText::new("DSP change takes effect\non host restart.")
                        .color(th::LABEL_DIM).size(th::scaled(th::FONT_SIZE_TINY, scale))
                );
            });
        });

    // Close on click outside: left-click only (right-click opens the popup, any_click would close
    // it on the same frame it was opened)
    if ui.ctx().input(|i| i.pointer.primary_clicked())
        && !area_response.response.contains_pointer()
    {
        new_state.open = false;
    }

    ui.data_mut(|d| d.insert_temp(key, new_state));
    assigned_slot
}

/// Open the popup for a slot at the given screen position.
pub fn open_popup(ui: &mut Ui, slot: usize, pos: Pos2) {
    let key = ui.id().with("module_popup");
    ui.data_mut(|d| d.insert_temp(key, PopupState { open: true, slot, pos }));
}

/// Yields `(curve_index, kind, value)` triples for every transform FloatParam
/// that needs to be reset on module switch. `kind` is "tilt" | "offset" |
/// "curvature". Caller (editor_ui.rs) iterates these and writes each via
/// `setter.set_parameter`. Mirrors the per-curve graph_node reset block.
///
/// The offset reset value is module-aware: curves where `natural_at_max` is
/// true default to `+1.0` (so the user loads at `y_max` and slides down),
/// while all other curves reset to `0.0`.  Tilt and curvature always reset
/// to `0.0`.
///
/// The `slot` parameter is kept for API consistency but is unused internally;
/// the new-module type and gain-mode carry all necessary context.
pub fn transform_reset_pairs(
    slot:      usize,
    module_ty: ModuleType,
    gain_mode: GainMode,
) -> impl Iterator<Item = (usize, &'static str, f32)> {
    let _ = slot;
    (0..7).flat_map(move |c| {
        let cfg = crate::editor::curve_config::curve_display_config(module_ty, c, gain_mode);
        let offset_default = if cfg.natural_at_max { 1.0_f32 } else { 0.0_f32 };
        [
            (c, "tilt",      0.0_f32),
            (c, "offset",    offset_default),
            (c, "curvature", 0.0_f32),
        ]
    })
}

/// Assign a module type to a slot: update slot_module_types, reset slot_curve_nodes, set name.
fn assign_module(params: &SpectralForgeParams, slot: usize, ty: ModuleType) {
    params.slot_module_types.lock()[slot] = ty;
    // Default the slot name to the module's display name (skip Empty and Master which keep
    // their existing names).
    let spec = module_spec(ty);
    if ty != ModuleType::Empty && ty != ModuleType::Master {
        let b = spec.display_name.as_bytes();
        let len = b.len().min(32);
        let mut names = params.slot_names.lock();
        names[slot].fill(0);
        names[slot][..len].copy_from_slice(&b[..len]);
    }
    let mut nodes = params.slot_curve_nodes.lock();
    for c in 0..7 {
        nodes[slot][c] = crate::editor::curve::default_nodes_for_module_curve(ty, c);
    }
    // Reset tilt/offset/curvature FloatParam smoothers for this slot.
    // assign_module has no ParamSetter access so we reset the smoothers directly;
    // the audio thread reads tilt/offset via smoothed.next(), so this takes effect
    // on the next processing block without host notification.
    //
    // Offset resets to +1.0 for natural-at-max curves (y_natural == y_max) of the
    // newly assigned module, matching the T3 FloatParam default set at plugin load.
    // All other transforms reset to 0.0.
    let gain_mode = params.slot_gain_mode.lock()[slot];
    for c in 0..7 {
        if let Some(p) = params.tilt_param(slot, c) {
            p.smoothed.reset(0.0);
        }
        if let Some(p) = params.offset_param(slot, c) {
            let cfg = crate::editor::curve_config::curve_display_config(ty, c, gain_mode);
            let offset_default = if cfg.natural_at_max { 1.0_f32 } else { 0.0_f32 };
            p.smoothed.reset(offset_default);
        }
        if let Some(p) = params.curvature_param(slot, c) {
            p.smoothed.reset(0.0);
        }
    }
    // Reset editing_curve to 0 if it's now out of range.
    let num_c = spec.num_curves;
    let mut ec = params.editing_curve.lock();
    if (*ec as usize) >= num_c && num_c > 0 {
        *ec = 0;
    }
}

/// One-paragraph "what does this module do" help shown when the user is
/// browsing the module-selector popup. Intentionally short — once a module
/// is assigned, the help-box's per-curve text takes over.
fn module_browse_help(ty: ModuleType) -> &'static str {
    match ty {
        ModuleType::Dynamics                => "Dynamics — per-bin compressor/expander. Each FFT bin has its own threshold, ratio, attack, release, and knee, all curve-driven. Sidechain: yes.",
        ModuleType::Freeze                  => "Freeze — capture a moment of the spectrum and hold it. Per-bin length, threshold, portamento, resistance. Sidechain: yes.",
        ModuleType::PhaseSmear              => "Phase Smear — per-bin phase randomization. Dissolves transients into smear; turns percussion into pads. Sidechain: yes.",
        ModuleType::Contrast                => "Contrast — sharpens spectral peaks and deepens valleys via per-bin upward expansion / downward compression. Sidechain: no.",
        ModuleType::Gain                    => "Gain — per-bin spectral gain shaping. Add / Subtract / Pull / Match modes change how the GAIN curve is applied. Sidechain: yes (Pull/Match).",
        ModuleType::MidSide                 => "Mid/Side — per-bin balance, expansion, decorrelation, transient steering, and pan. Sidechain: no.",
        ModuleType::TransientSustainedSplit => "T/S Split — splits the slot's input into transient and sustained streams that feed virtual rows in the routing matrix. Max 2 active. Sidechain: no.",
        ModuleType::Harmonic                => "Harmonic — pass-through that computes harmonic-grouping data for downstream Harmony slots. No curves. Sidechain: no.",
        ModuleType::Future                  => "Future — print-through and pre-echo from spectral history. Curve-driven leak/echo amplitudes. Sidechain: no.",
        ModuleType::Punch                   => "Punch — sidechain-driven spectral carving with neighbour fill. Direct mode carves at SC peaks; Inverse at troughs. Sidechain: yes (required).",
        ModuleType::Rhythm                  => "Rhythm — host-tempo-locked spectral gating. Euclidean / Arpeggiator / Phase Reset modes. Sidechain: no.",
        ModuleType::Geometry                => "Geometry — physical-resonator carving. Chladni nodal lines or a Helmholtz cavity. Sidechain: no.",
        ModuleType::Modulate                => "Modulate — bin modulators. Phase Phaser, Bin Swapper, RM/FM, Diode RM, Ground Loop, Gravity, PLL Tear, FM Network. Sidechain: yes (RM/FM, Diode RM, etc.).",
        ModuleType::Circuit                 => "Circuit — analog-component-modelled per-bin nonlinearities. Vactrol, Schmitt, BBD, transformer, sag, drift, crosstalk, slew, bias fuzz, crossover. Sidechain: no.",
        ModuleType::Life                    => "Life — physics-of-matter spectral effects. Viscosity, surface tension, crystallization, Archimedes' principle, non-Newtonian fluid, stiction, yield, capillary, sandpaper, Brownian motion. Sidechain: no.",
        ModuleType::Past                    => "Past — read-only access to a rolling spectral history buffer. Granular freeze, decay-sorter, convolution, reverse, stretch. Sidechain: no.",
        ModuleType::Kinetics                => "Kinetics — physical motion models. Hooke springs, gravity wells, inertial mass, orbital phase, ferromagnetism, thermal expansion, tuning fork, diamagnetism. Sidechain: yes (Gravity Well, Inertial Mass with SC source).",
        ModuleType::Harmony                 => "Harmony — pitch-class and partial restructuring. Chordification, undertone, companding, formant rotation, lifter, inharmonic, harmonic generator, shuffler. Sidechain: no.",
        ModuleType::Empty                   => "Empty — no module assigned. The slot passes through unchanged.",
        ModuleType::Master                  => "Master — output sum stage; cannot be reassigned.",
    }
}
