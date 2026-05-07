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
        HelpTopic::Tilt             => "Tilt — frequency-dependent shift pivoted at 1 kHz. Negative tilts the curve down at high frequencies; positive tilts it up. Up to ±4 dB/oct (near-diagonal at v = ±1).",
        HelpTopic::Curvature        => "Curvature — bends the tilt into a smoothstep S-curve, concentrating the change around the 1 kHz pivot. 0 = straight tilt; 1 = full S.",

        HelpTopic::MatrixCellSend   => "Routing matrix cell — send amplitude from this row's slot to this column's slot. 0 = off, 1 = unity. Drag to set; right-click for the amp popup.",
        HelpTopic::MatrixSlotSelect => "Click to focus this slot in the curve editor above. Right-click to assign a different module type.",
    }
}

const HELP_PENDING_KEY:   &str = "spectral_forge::help_pending";
const HELP_PRESENTED_KEY: &str = "spectral_forge::help_presented";

fn pending_id()   -> egui::Id { egui::Id::new(HELP_PENDING_KEY) }
fn presented_id() -> egui::Id { egui::Id::new(HELP_PRESENTED_KEY) }

/// Per-frame help focus claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelpFocus {
    /// Lookup by enum (static head + body via topic_head / topic_help_text).
    Topic(HelpTopic),
    /// Free-form head and body. Owned `String` so the same code path serves
    /// both static literals and per-frame dynamic text (e.g. routing matrix
    /// `Slot 1 -> Slot 9` summaries). When `yellow_prefix` is `Some(word)`
    /// the renderer paints `word` in yellow before `body`.
    Custom { head: String, body: String, yellow_prefix: Option<String> },
}

/// Track that `response` is being hovered/dragged/focused. If so, claim the
/// pending help focus for `topic`. Pending is promoted to presented at the
/// next frame's `promote_focus` call so claims from popups (rendered after
/// the help-box draws) still appear.
pub fn track_help(ui: &egui::Ui, response: &egui::Response, topic: HelpTopic) {
    set_focus_if_active(ui, response, HelpFocus::Topic(topic));
}

/// Track free-form help text for `response`. Accepts anything `Into<String>`
/// so static literals and dynamic `format!(...)` summaries share the path.
pub fn track_help_strings(
    ui: &egui::Ui,
    response: &egui::Response,
    head: impl Into<String>,
    body: impl Into<String>,
) {
    if response.hovered() || response.dragged() || response.has_focus() {
        let focus = HelpFocus::Custom {
            head: head.into(), body: body.into(), yellow_prefix: None,
        };
        ui.ctx().data_mut(|d| d.insert_temp::<Option<HelpFocus>>(pending_id(), Some(focus)));
    }
}

/// Same as `track_help_strings` but renders `prefix` (a single short word)
/// in yellow before `body`. Used for feedback-routing cells in the matrix.
pub fn track_help_strings_yellow(
    ui: &egui::Ui,
    response: &egui::Response,
    head: impl Into<String>,
    body: impl Into<String>,
    prefix: impl Into<String>,
) {
    if response.hovered() || response.dragged() || response.has_focus() {
        let focus = HelpFocus::Custom {
            head: head.into(), body: body.into(),
            yellow_prefix: Some(prefix.into()),
        };
        ui.ctx().data_mut(|d| d.insert_temp::<Option<HelpFocus>>(pending_id(), Some(focus)));
    }
}

fn set_focus_if_active(ui: &egui::Ui, response: &egui::Response, focus: HelpFocus) {
    if response.hovered() || response.dragged() || response.has_focus() {
        ui.ctx().data_mut(|d| d.insert_temp::<Option<HelpFocus>>(pending_id(), Some(focus)));
    }
}

/// Resolve per-curve help for any module/mode/curve triple. Used by widgets
/// that want to preview a curve's help while the user is browsing curve
/// tabs or other module-aware controls.
pub fn curve_help_text(ty: ModuleType, mode_byte: u8, curve_idx: usize) -> &'static str {
    let s = multi_mode_curve_help(ty, mode_byte, curve_idx);
    if !s.is_empty() {
        return s;
    }
    single_mode_curve_help(ty, curve_idx)
}

/// Promote the previous frame's pending focus into the presented slot, then
/// clear pending so this frame's widgets start with an empty claim. Call
/// once at the very top of the editor frame.
///
/// The 1-frame delay lets popups (rendered after `help_box::draw` in the
/// frame order) still update the help-box — their writes from frame N
/// surface in frame N+1's render. Imperceptible at refresh-rate cadence.
pub fn promote_focus(ctx: &egui::Context) {
    let pending = ctx.data(|d| d.get_temp::<Option<HelpFocus>>(pending_id())).flatten();
    ctx.data_mut(|d| {
        d.insert_temp::<Option<HelpFocus>>(presented_id(), pending);
        d.insert_temp::<Option<HelpFocus>>(pending_id(), None);
    });
}

fn current_focus(ctx: &egui::Context) -> Option<HelpFocus> {
    ctx.data(|d| d.get_temp::<Option<HelpFocus>>(presented_id())).flatten()
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

    let (mode_byte, layout) = active_layout_for_slot(editing_type, params, editing_slot);

    let help_on = params.help_enabled.value();
    let focus   = current_focus(ui.ctx());

    let (head, body, yellow_prefix): (Cow<'static, str>, Cow<'static, str>, Option<String>) = if !help_on {
        (Cow::Borrowed("Help (off)"),
         Cow::Borrowed("Toggle the HELP button in the top bar to bring help back."),
         None)
    } else {
        match focus {
            Some(HelpFocus::Topic(t)) =>
                (Cow::Borrowed(topic_head(t)), Cow::Borrowed(topic_help_text(t)), None),
            Some(HelpFocus::Custom { head, body, yellow_prefix }) =>
                (Cow::Owned(head), Cow::Owned(body), yellow_prefix),
            None => {
                let h = Cow::Borrowed(spec.display_name);
                let b = body_text(layout.as_ref(), mode_byte, editing_type, editing_curve, spec.curve_labels);
                (h, b, None)
            }
        }
    };

    let pad = th::scaled(th::HELP_BOX_PADDING, scale).round() as i8;
    let inner_w = th::scaled(th::HELP_BOX_WIDTH, scale);
    Frame::new()
        .fill(th::HELP_BOX_BG)
        .stroke(Stroke::new(th::scaled_stroke(th::STROKE_BORDER, scale), th::HELP_BOX_BORDER))
        .inner_margin(egui::Margin { left: pad, right: pad, top: pad, bottom: pad })
        .show(ui, |ui| {
            ui.set_width(inner_w);
            ui.add(
                egui::Label::new(
                    RichText::new(head.as_ref())
                        .color(th::HELP_BOX_HEAD)
                        .font(FontId::proportional(th::scaled(th::FONT_SIZE_HELP_HEAD, scale))),
                ).wrap(),
            );
            ui.add_space(4.0);
            // Body. Always rendered through a LayoutJob with explicit wrap
            // width — `Label::wrap()` and `ui.available_width()` both
            // proved unreliable inside this Frame (parent UI width leaks
            // through). Hard-coding the wrap target to the same value we
            // gave to `set_width` (minus the symmetric inner margin) is
            // the only reliable way to keep body text bounded.
            let body_font  = FontId::proportional(th::scaled(th::FONT_SIZE_HELP_BODY, scale));
            let body_color = th::HELP_BOX_BODY;
            let yellow     = egui::Color32::from_rgb(0xff, 0xc8, 0x40);
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width      = (inner_w - 2.0 * pad as f32).max(40.0);
            job.wrap.break_anywhere = false;
            if let Some(prefix) = yellow_prefix {
                job.append(
                    &format!("{} ", prefix),
                    0.0,
                    egui::TextFormat {
                        font_id: body_font.clone(),
                        color:   yellow,
                        ..Default::default()
                    },
                );
            }
            job.append(
                body.as_ref(),
                0.0,
                egui::TextFormat {
                    font_id: body_font,
                    color:   body_color,
                    ..Default::default()
                },
            );
            ui.add(egui::Label::new(job));
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
) -> (u8, Option<CurveLayout>) {
    let mode_byte: u8 = match ty {
        ModuleType::Past     => params.slot_past_mode.lock()[slot]     as u8,
        ModuleType::Future   => params.slot_future_mode.lock()[slot]   as u8,
        ModuleType::Circuit  => params.slot_circuit_mode.lock()[slot]  as u8,
        ModuleType::Life     => params.slot_life_mode.lock()[slot]     as u8,
        ModuleType::Modulate => params.slot_modulate_mode.lock()[slot] as u8,
        ModuleType::Rhythm   => params.slot_rhythm_mode.lock()[slot]   as u8,
        ModuleType::Punch    => params.slot_punch_mode.lock()[slot]    as u8,
        ModuleType::Harmony  => params.slot_harmony_mode.lock()[slot]  as u8,
        ModuleType::Geometry => params.slot_geometry_mode.lock()[slot] as u8,
        ModuleType::Kinetics => params.slot_kinetics_mode.lock()[slot] as u8,
        _ => 0,
    };
    (mode_byte, module_spec(ty).active_layout.map(|f| f(mode_byte)))
}

/// Resolve body text per the precedence:
///
/// 1. `layout.help_for(curve_idx)` if curve is in `layout.active` and returns non-empty.
/// 2. `multi_mode_curve_help(ty, mode_byte, curve_idx)` — centralized lookup
///    for multi-mode modules whose `help_for` returns empty.
/// 3. `layout.mode_overview` if Some — covers "curve not in active" cases.
/// 4. Static module-level description (with curve label appended when known).
///
/// Returns `Cow` so the common static-text paths avoid heap alloc.
fn body_text(
    layout_opt: Option<&CurveLayout>,
    mode_byte: u8,
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
            let s2 = multi_mode_curve_help(editing_type, mode_byte, editing_curve);
            if !s2.is_empty() {
                return Cow::Borrowed(s2);
            }
        }
        if let Some(overview) = layout.mode_overview {
            return Cow::Borrowed(overview);
        }
    }
    static_description(editing_type, editing_curve, curve_labels)
}

/// Module-level fallback when no `active_layout` is provided OR when
/// `active_layout` returns no help for the focused curve. First tries a
/// per-curve description from `single_mode_curve_help`; falls back to a
/// short module description with the curve label appended.
fn static_description(
    ty: ModuleType,
    editing_curve: usize,
    curve_labels: &'static [&'static str],
) -> Cow<'static, str> {
    let curve_help = single_mode_curve_help(ty, editing_curve);
    if !curve_help.is_empty() {
        return Cow::Borrowed(curve_help);
    }

    let module_text: &'static str = match ty {
        ModuleType::Past => "Past — read-only access to a rolling buffer of recent spectral history. \
                             Pick a mode in the slot row to choose how the buffer is replayed. \
                             Sidechain: not used.",
        ModuleType::Empty    => "No module assigned to this slot. Empty slots pass-through with no processing — the wet signal stays bit-perfect through this stage.",
        ModuleType::Master   => "Master output — sums every slot routed into row 9 (Master) into the plugin output. The master soft-clipper (top bar CLIP toggle) sits at the very last stage.",
        ModuleType::Dynamics => "Dynamics — per-bin compressor/expander. Each FFT bin has its own threshold, ratio, attack, release, and knee, all driven by curves. Sidechain: yes — sidechain envelope steers detection per-bin.",
        ModuleType::Freeze   => "Freeze — captures a moment of the spectrum and holds it. Per-bin threshold + length determine which bins freeze and for how long; portamento glides into the freeze, resistance shapes the magnitude blend. Sidechain: yes — sidechain magnitude can drive the freeze trigger.",
        ModuleType::PhaseSmear => "Phase Smear — randomizes per-bin phase to dissolve transients and turn percussive material into smear. Sidechain: yes — sidechain steers the amount per-bin.",
        ModuleType::Contrast   => "Spectral Contrast — sharpens spectral peaks and deepens valleys via a per-bin upward expander/downward compressor. Sidechain: no.",
        ModuleType::Gain       => "Gain — per-bin spectral gain shaping. The Add/Subtract/Pull/Match selector below changes how the GAIN curve is applied. Sidechain: yes (Pull/Match modes).",
        ModuleType::MidSide    => "Mid/Side — per-bin mid/side balance, M/S expansion, phase decorrelation, transient steering, and stereo pan. Sidechain: no.",
        ModuleType::TransientSustainedSplit    => "T/S Split — splits the slot's input into transient and sustained streams that feed virtual rows in the routing matrix (slot N + 'T'/'S'). Sidechain: no.",
        ModuleType::Harmonic   => "Harmonic — placeholder slot type that passes through; the harmonic-grouping data it computes is consumed by other modules. No curves. Sidechain: no.",
        _ => module_spec(ty).display_name,
    };
    if let Some(label) = curve_labels.get(editing_curve) {
        if !label.is_empty() {
            return Cow::Owned(format!("{}\n\nCurve: {}", module_text, label));
        }
    }
    Cow::Borrowed(module_text)
}

/// Per-curve help for multi-mode modules whose `help_for` is the empty
/// stub. Mode discriminants follow each module's enum order. Returns "" if
/// no entry — caller falls back to `mode_overview`.
fn multi_mode_curve_help(ty: ModuleType, mode: u8, curve_idx: usize) -> &'static str {
    match (ty, mode, curve_idx) {
        // ── Modulate (SC: yes — RM/FM, DiodeRM, Ground Loop, Gravity read sidechain) ──
        // 0=PhasePhaser  curves: AMOUNT(0), RATE(2), THRESH(3), AMPGATE(4), MIX(5)
        (ModuleType::Modulate, 0, 0) => "Phase Phaser · AMOUNT — phase rotation depth applied per bin per hop. 0 = no rotation; 1 = full π rotation per LFO cycle.",
        (ModuleType::Modulate, 0, 2) => "Phase Phaser · RATE — speed of the per-bin phase LFO. Sub-Hz at 0, fast at 1. Sets how quickly the phase comb sweeps.",
        (ModuleType::Modulate, 0, 3) => "Phase Phaser · THRESH — magnitude gate. Only bins above this level get phase-rotated; quieter bins pass through clean.",
        (ModuleType::Modulate, 0, 4) => "Phase Phaser · AMPGATE — amplitude smoother on rotation depth so loud transients aren't smeared as aggressively as steady tones.",
        (ModuleType::Modulate, 0, 5) => "Phase Phaser · MIX — wet/dry between rotated and original phase, per-bin.",
        // 1=BinSwapper  curves: AMOUNT(0), REACH(1), THRESH(3), MIX(5)
        (ModuleType::Modulate, 1, 0) => "Bin Swapper · AMOUNT — fraction of bin energy displaced to its REACH-offset partner. 0 = no swap, 1 = full swap.",
        (ModuleType::Modulate, 1, 1) => "Bin Swapper · REACH — bin offset to swap with. Small = neighbour swap; large = drag energy across the spectrum.",
        (ModuleType::Modulate, 1, 3) => "Bin Swapper · THRESH — gate. Only bins above this magnitude participate in swaps.",
        (ModuleType::Modulate, 1, 5) => "Bin Swapper · MIX — wet/dry between swapped and original.",
        // 2=RmFmMatrix  curves: AMOUNT(0), REACH(1), THRESH(3), MIX(5)  — sidechain-driven
        (ModuleType::Modulate, 2, 0) => "RM/FM Matrix · AMOUNT — blends ring-mod (low) vs frequency-mod (high) at the sidechain carrier. SC-required: feed a carrier into this slot's sidechain.",
        (ModuleType::Modulate, 2, 1) => "RM/FM Matrix · REACH — RM/FM modulation depth scaler. Higher = more aggressive sidebands and FM character.",
        (ModuleType::Modulate, 2, 3) => "RM/FM Matrix · THRESH — sidechain magnitude floor. SC bins below this don't drive modulation; only the loud parts of the carrier punch through.",
        (ModuleType::Modulate, 2, 5) => "RM/FM Matrix · MIX — wet/dry between modulated and dry signal.",
        // 3=DiodeRm  curves: AMOUNT(0), REACH(1), THRESH(3), MIX(5)  — sidechain-driven
        (ModuleType::Modulate, 3, 0) => "Diode RM · AMOUNT — ring-mod depth, gated by an asymmetric diode-style nonlinearity. SC-required for the carrier.",
        (ModuleType::Modulate, 3, 1) => "Diode RM · REACH — carrier-mismatch leak. Higher = more clean carrier bleeds through (4-quadrant modulator analogue).",
        (ModuleType::Modulate, 3, 3) => "Diode RM · THRESH — diode closure level. Below threshold the diode doesn't conduct, so RM only kicks in on louder content.",
        (ModuleType::Modulate, 3, 5) => "Diode RM · MIX — wet/dry between modulated and dry.",
        // 4=GroundLoop  curves: AMOUNT(0), REACH(1), RATE(2), THRESH(3), MIX(5)
        (ModuleType::Modulate, 4, 0) => "Ground Loop · AMOUNT — depth of injected mains-hum harmonics intermodulated with the bin content.",
        (ModuleType::Modulate, 4, 1) => "Ground Loop · REACH — number of harmonic partials of the hum frequency to inject (1, 2, …).",
        (ModuleType::Modulate, 4, 2) => "Ground Loop · RATE — selects 50 Hz vs 60 Hz mains frequency (and any in-between for non-realistic sweeps).",
        (ModuleType::Modulate, 4, 3) => "Ground Loop · THRESH — sag gate. Hum is sprayed only when bin energy crosses this level, so ambient sections stay clean.",
        (ModuleType::Modulate, 4, 5) => "Ground Loop · MIX — wet/dry of hum-injected vs original.",
        // 5=GravityPhaser  curves: AMOUNT(0), REACH(1), THRESH(3), AMPGATE(4), MIX(5) — SC-positioned with the toggle below
        (ModuleType::Modulate, 5, 0) => "Gravity Phaser · AMOUNT — strength of the gravitational pull on per-bin phase momentum. Higher = bins are dragged harder toward the well.",
        (ModuleType::Modulate, 5, 1) => "Gravity Phaser · REACH — well width in bins. Wide wells affect more of the spectrum simultaneously.",
        (ModuleType::Modulate, 5, 3) => "Gravity Phaser · THRESH — amplitude gate. Bins below this aren't pulled.",
        (ModuleType::Modulate, 5, 4) => "Gravity Phaser · AMPGATE — secondary magnitude-gate strength. Pair with Repel (slot row) to push instead of pull, and SC-pos to drive the well centre from sidechain.",
        (ModuleType::Modulate, 5, 5) => "Gravity Phaser · MIX — wet/dry between pulled and original phase.",
        // 6=PllTear  curves: AMOUNT(0), REACH(1), RATE(2), THRESH(3), MIX(5)
        (ModuleType::Modulate, 6, 0) => "PLL Tear · AMOUNT — chaotic-noise emission scaling on bins that lose lock. 0 = no tear, 1 = aggressive lock-loss artefacts.",
        (ModuleType::Modulate, 6, 1) => "PLL Tear · REACH — coupling between adjacent bin PLLs. Higher = neighbour bins influence each other's lock state.",
        (ModuleType::Modulate, 6, 2) => "PLL Tear · RATE — PLL loop bandwidth. Slow loops lock tightly; fast loops are jittery and tear more readily.",
        (ModuleType::Modulate, 6, 3) => "PLL Tear · THRESH — magnitude floor. Bins below don't run a PLL.",
        (ModuleType::Modulate, 6, 5) => "PLL Tear · MIX — wet/dry between PLL-processed and original.",
        // 7=FmNetwork  curves: AMOUNT(0), REACH(1), AMPGATE-as-COEFFICIENT(4), MIX(5)
        (ModuleType::Modulate, 7, 0) => "FM Network · AMOUNT — modulation index between detected partial pairs. Higher = more sidebands per pair.",
        (ModuleType::Modulate, 7, 1) => "FM Network · REACH — partial detection magnitude threshold. Only bins above this strength qualify as carriers.",
        (ModuleType::Modulate, 7, 4) => "FM Network · COEFFICIENT — partial-pair AM modulation depth. Modulates one partial's amplitude with another's carrier.",
        (ModuleType::Modulate, 7, 5) => "FM Network · MIX — wet/dry between modulated and original.",

        // ── Kinetics (SC: yes — Gravity Well and Inertial Mass read sidechain) ──
        // 0=Hooke  curves: STRENGTH(0), REACH(2), DAMPING(3), MIX(4)
        (ModuleType::Kinetics, 0, 0) => "Hooke · STRENGTH — spring constant. Higher = stiffer springs pulling adjacent bins toward each other; lower = looser coupling.",
        (ModuleType::Kinetics, 0, 2) => "Hooke · REACH — sympathetic-spring radius in harmonics. 0 = neighbour-only diffusion; >0 adds harmonic resonators that vibrate when the fundamental rings.",
        (ModuleType::Kinetics, 0, 3) => "Hooke · DAMPING — friction on the spring system. Low = oscillates forever; high = quick settling.",
        (ModuleType::Kinetics, 0, 4) => "Hooke · MIX — wet/dry of the spring-coupled vs raw spectrum.",
        // 1=GravityWell  curves: STRENGTH(0), MASS(1), REACH(2), DAMPING(3), MIX(4)  SC option
        (ModuleType::Kinetics, 1, 0) => "Gravity Well · STRENGTH — Newtonian gravitational constant. Higher = bins fall toward the well harder. Well centre source picked in the popup (Static / Sidechain / MIDI).",
        (ModuleType::Kinetics, 1, 1) => "Gravity Well · MASS — per-bin mass. Heavier bins resist being pulled; lighter ones snap into the well faster.",
        (ModuleType::Kinetics, 1, 2) => "Gravity Well · REACH — well width in bins. Wide wells pull a chord into a single tone; narrow wells pick a single peak.",
        (ModuleType::Kinetics, 1, 3) => "Gravity Well · DAMPING — friction on infall. Low = bins overshoot and oscillate; high = bins settle into the well.",
        (ModuleType::Kinetics, 1, 4) => "Gravity Well · MIX — wet/dry between the gravitationally-bent spectrum and the original.",
        // 2=InertialMass  curves: MASS(1), MIX(4) — writes physics.mass; sidechain steers (with mass-source popup)
        (ModuleType::Kinetics, 2, 1) => "Inertial Mass · MASS — per-bin mass written to BinPhysics for downstream Kinetics slots. Mass source picked in the popup (Static curve / Sidechain rate).",
        (ModuleType::Kinetics, 2, 4) => "Inertial Mass · MIX — wet/dry. This mode primarily writes mass into BinPhysics; MIX scales that contribution.",
        // 3=OrbitalPhase  curves: STRENGTH(0), MIX(4)
        (ModuleType::Kinetics, 3, 0) => "Orbital Phase · STRENGTH — per-hop alpha rotation magnitude on satellite bins around detected spectral peaks (planets). Satellites rotate opposite to planets.",
        (ModuleType::Kinetics, 3, 4) => "Orbital Phase · MIX — wet/dry of the orbital-rotated vs original phase.",
        // 4=Ferromagnetism  curves: STRENGTH(0), REACH(2), DAMPING(3), MIX(4)
        (ModuleType::Kinetics, 4, 0) => "Ferromagnetism · STRENGTH — alignment force pulling neighbour bin phases toward the nearest spectral peak's phase.",
        (ModuleType::Kinetics, 4, 2) => "Ferromagnetism · REACH — alignment-window radius around each peak. Wider = more bins phase-lock to the peak.",
        (ModuleType::Kinetics, 4, 3) => "Ferromagnetism · DAMPING — phase resistance. Low = neighbours snap; high = neighbours drift slowly.",
        (ModuleType::Kinetics, 4, 4) => "Ferromagnetism · MIX — wet/dry of aligned vs raw phase.",
        // 5=ThermalExpansion  curves: STRENGTH(0), DAMPING(3), MIX(4)
        (ModuleType::Kinetics, 5, 0) => "Thermal Expansion · STRENGTH — heat-input rate from signal energy. Loud bins heat up faster.",
        (ModuleType::Kinetics, 5, 3) => "Thermal Expansion · DAMPING — cooling rate. Low = heat lingers and bins stay detuned; high = quick cool-down.",
        (ModuleType::Kinetics, 5, 4) => "Thermal Expansion · MIX — wet/dry of the heat-driven detune vs original.",
        // 6=TuningFork  curves: STRENGTH(0), REACH(2), MIX(4)
        (ModuleType::Kinetics, 6, 0) => "Tuning Fork · STRENGTH — peak-detection threshold and sympathetic-modulation depth on bins near the fork.",
        (ModuleType::Kinetics, 6, 2) => "Tuning Fork · REACH — radius around the fork peak that picks up sympathetic resonance.",
        (ModuleType::Kinetics, 6, 4) => "Tuning Fork · MIX — wet/dry of the resonance-driven vs original.",
        // 7=Diamagnet  curves: STRENGTH(0), REACH(2), DAMPING(3), MIX(4)
        (ModuleType::Kinetics, 7, 0) => "Diamagnet · STRENGTH — carving depth at expelled-energy bins. Energy is conserved by redistributing 1/d into neighbours.",
        (ModuleType::Kinetics, 7, 2) => "Diamagnet · REACH — redistribution radius. Wider = energy spreads into more neighbour bins.",
        (ModuleType::Kinetics, 7, 3) => "Diamagnet · DAMPING — friction on the carving/redistribution dynamics.",
        (ModuleType::Kinetics, 7, 4) => "Diamagnet · MIX — wet/dry of the diamagnetic vs original spectrum.",

        // ── Harmony (SC: no) ──
        // 0=Chordification  active = [0=AMOUNT, 1=THRESHOLD, 3=SPREAD, 5=MIX]
        (ModuleType::Harmony, 0, 0) => "Chordification · AMOUNT — pull strength toward the nearest of 24 major/minor chord templates. 0 = no snap, 1 = full pitch-class quantize.",
        (ModuleType::Harmony, 0, 1) => "Chordification · THRESHOLD — minimum bin magnitude to vote in the chromagram. Higher = stronger peaks decide the chord.",
        (ModuleType::Harmony, 0, 3) => "Chordification · SPREAD — snap radius around each in-chord pitch class. Tight = aggressive; wide = subtle harmonization.",
        (ModuleType::Harmony, 0, 5) => "Chordification · MIX — wet/dry of chordified spectrum vs original.",
        // 1=Undertone  active = [0=AMOUNT, 1=THRESHOLD, 3=SPREAD, 4=COEFFICIENT, 5=MIX]
        (ModuleType::Harmony, 1, 0) => "Undertone · AMOUNT — depth of generated sub-octave partials below detected peaks.",
        (ModuleType::Harmony, 1, 1) => "Undertone · THRESHOLD — peak detection level for which bins seed sub-partials.",
        (ModuleType::Harmony, 1, 3) => "Undertone · SPREAD — sub-partial decay rate. Slow decay gives a fatter, more sustained sub-octave.",
        (ModuleType::Harmony, 1, 4) => "Undertone · COEFFICIENT — selects hum frequency weighting (off / 50 / 60 / 120 Hz) blended with the sub-octave generation.",
        (ModuleType::Harmony, 1, 5) => "Undertone · MIX — wet/dry of sub-rich vs original.",
        // 2=Companding  active = [0=AMOUNT, 4=COEFFICIENT, 5=MIX]
        (ModuleType::Harmony, 2, 0) => "Companding · AMOUNT — per-bin gate strength on harmonic-overtone bins relative to fundamentals (uses ctx.harmonic_groups).",
        (ModuleType::Harmony, 2, 4) => "Companding · COEFFICIENT — attenuation depth applied to overtones. 0 = leave overtones alone; 1 = silence them.",
        (ModuleType::Harmony, 2, 5) => "Companding · MIX — wet/dry.",
        // 3=FormantRotation  active = [0=AMOUNT, 4=COEFFICIENT, 5=MIX]
        (ModuleType::Harmony, 3, 0) => "Formant Rotation · AMOUNT — overall depth of harmonic shift while preserving the spectral envelope (formants).",
        (ModuleType::Harmony, 3, 4) => "Formant Rotation · COEFFICIENT — harmonic-shift ratio between 0.5× and 2.0×. 1.0 = no shift.",
        (ModuleType::Harmony, 3, 5) => "Formant Rotation · MIX — wet/dry.",
        // 4=Lifter  active = [0=AMOUNT, 3=SPREAD, 4=COEFFICIENT, 5=MIX]
        (ModuleType::Harmony, 4, 0) => "Lifter · AMOUNT — overall blend depth of cepstrum-shaped vs original.",
        (ModuleType::Harmony, 4, 3) => "Lifter · SPREAD — low-quefrency window scaling (the envelope/formant region of the cepstrum).",
        (ModuleType::Harmony, 4, 4) => "Lifter · COEFFICIENT — high-quefrency window scaling (the pitch region of the cepstrum). Independent envelope/pitch shaping.",
        (ModuleType::Harmony, 4, 5) => "Lifter · MIX — wet/dry.",
        // 5=Inharmonic  active = [0=AMOUNT, 1=THRESHOLD, 4=COEFFICIENT, 5=MIX]
        (ModuleType::Harmony, 5, 0) => "Inharmonic · AMOUNT — strength of partial detuning toward the chosen frequency grid.",
        (ModuleType::Harmony, 5, 1) => "Inharmonic · THRESHOLD — partial-detection floor.",
        (ModuleType::Harmony, 5, 4) => "Inharmonic · COEFFICIENT — selects the inharmonicity model (Stiffness / Bessel / Prime) and its strength parameter.",
        (ModuleType::Harmony, 5, 5) => "Inharmonic · MIX — wet/dry.",
        // 6=HarmonicGenerator  active = [0=AMOUNT, 1=THRESHOLD, 3=SPREAD, 4=COEFFICIENT, 5=MIX]
        (ModuleType::Harmony, 6, 0) => "Harmonic Generator · AMOUNT — per-partial amplitude scaling on synthesised harmonics.",
        (ModuleType::Harmony, 6, 1) => "Harmonic Generator · THRESHOLD — fundamental-detection magnitude floor.",
        (ModuleType::Harmony, 6, 3) => "Harmonic Generator · SPREAD — partial decay; controls how loud each subsequent harmonic is relative to the fundamental.",
        (ModuleType::Harmony, 6, 4) => "Harmonic Generator · COEFFICIENT — number of harmonics to synthesise.",
        (ModuleType::Harmony, 6, 5) => "Harmonic Generator · MIX — wet/dry of synthesised harmonics added to the spectrum.",
        // 7=Shuffler  active = [0=AMOUNT, 1=THRESHOLD, 3=SPREAD, 5=MIX]
        (ModuleType::Harmony, 7, 0) => "Shuffler · AMOUNT — fraction of bins randomly swapped per hop.",
        (ModuleType::Harmony, 7, 1) => "Shuffler · THRESHOLD — gate. Quiet bins pass through; only loud bins are swap candidates.",
        (ModuleType::Harmony, 7, 3) => "Shuffler · SPREAD — maximum swap reach. Small = neighbour swaps (chorus-like); large = wholesale spectral scramble.",
        (ModuleType::Harmony, 7, 5) => "Shuffler · MIX — wet/dry of shuffled vs original.",

        // ── Life (SC: no) ──
        // 0=Viscosity  active = [0=AMOUNT, 4=MIX]
        (ModuleType::Life, 0, 0) => "Viscosity · AMOUNT — diffusion coefficient. Higher = more spectral smoothing (energy bleeds across adjacent bins like fluid resistance).",
        (ModuleType::Life, 0, 4) => "Viscosity · MIX — wet/dry.",
        // 1=SurfaceTension  active = [0=AMOUNT, 1=THRESHOLD, 3=REACH, 4=MIX]
        (ModuleType::Life, 1, 0) => "Surface Tension · AMOUNT — coalescence strength. Higher = stronger peaks pull energy from weaker neighbours within reach.",
        (ModuleType::Life, 1, 1) => "Surface Tension · THRESHOLD — peak detection floor. Only bins above this can pull from neighbours.",
        (ModuleType::Life, 1, 3) => "Surface Tension · REACH — pull radius in bins. Larger = peaks consume more of the surrounding spectrum.",
        (ModuleType::Life, 1, 4) => "Surface Tension · MIX — wet/dry.",
        // 2=Crystallization  active = [0=AMOUNT, 1=THRESHOLD, 2=SPEED, 4=MIX]
        (ModuleType::Life, 2, 0) => "Crystallization · AMOUNT — phase-lock growth rate. Sustained tonal bins accumulate a crystallization envelope; AMOUNT scales how fast it builds.",
        (ModuleType::Life, 2, 1) => "Crystallization · THRESHOLD — magnitude floor for which bins crystallize.",
        (ModuleType::Life, 2, 2) => "Crystallization · SPEED — LP decay rate of the crystallization envelope. Low = crystals persist; high = they melt quickly.",
        (ModuleType::Life, 2, 4) => "Crystallization · MIX — wet/dry of crystallized vs original phase.",
        // 3=Archimedes  active = [0=AMOUNT, 1=THRESHOLD, 4=MIX]
        (ModuleType::Life, 3, 0) => "Archimedes · AMOUNT — global ducking strength. When total spectral magnitude exceeds capacity, all bins are scaled down by AMOUNT × overflow.",
        (ModuleType::Life, 3, 1) => "Archimedes · THRESHOLD — capacity ceiling. Total-magnitude exceeding this triggers proportional ducking.",
        (ModuleType::Life, 3, 4) => "Archimedes · MIX — wet/dry of ducked vs raw.",
        // 4=NonNewtonian  active = [0=AMOUNT, 1=THRESHOLD, 4=MIX]
        (ModuleType::Life, 4, 0) => "Non-Newtonian · AMOUNT — rate-limit aggression. Higher = fast-changing magnitudes are clamped harder; slow signals always pass.",
        (ModuleType::Life, 4, 1) => "Non-Newtonian · THRESHOLD — velocity threshold above which the oobleck solidifies.",
        (ModuleType::Life, 4, 4) => "Non-Newtonian · MIX — wet/dry.",
        // 5=Stiction  active = [0=AMOUNT, 1=THRESHOLD, 2=SPEED, 4=MIX]
        (ModuleType::Life, 5, 0) => "Stiction · AMOUNT — friction strength. Higher = static friction more aggressively holds slow-moving bins.",
        (ModuleType::Life, 5, 1) => "Stiction · THRESHOLD — velocity threshold separating static (held) and kinetic (free) regimes.",
        (ModuleType::Life, 5, 2) => "Stiction · SPEED — decay rate for held-static bins. Slow speeds let bins ring out; fast pulls them to silence.",
        (ModuleType::Life, 5, 4) => "Stiction · MIX — wet/dry.",
        // 6=Yield  active = [0=AMOUNT, 1=THRESHOLD, 2=SPEED, 4=MIX]
        (ModuleType::Life, 6, 0) => "Yield · AMOUNT — tear strength. Bins exceeding threshold get phase scrambled and magnitude clamped — louder content tears more violently.",
        (ModuleType::Life, 6, 1) => "Yield · THRESHOLD — tear-point magnitude. Above this, the fabric rips.",
        (ModuleType::Life, 6, 2) => "Yield · SPEED — heal rate. Torn bins gradually restore over this time.",
        (ModuleType::Life, 6, 4) => "Yield · MIX — wet/dry.",
        // 7=Capillary  active = [0=AMOUNT, 1=THRESHOLD, 2=SPEED, 3=REACH, 4=MIX]
        (ModuleType::Life, 7, 0) => "Capillary · AMOUNT — wicking strength. Energy drains upward through the spectrum from source bins to harmonic destinations.",
        (ModuleType::Life, 7, 1) => "Capillary · THRESHOLD — wicking gate. Bins below this don't act as donors.",
        (ModuleType::Life, 7, 2) => "Capillary · SPEED — wicking rate. Sets how fast donors drain into receivers.",
        (ModuleType::Life, 7, 3) => "Capillary · REACH — number of harmonic destinations above each source bin.",
        (ModuleType::Life, 7, 4) => "Capillary · MIX — wet/dry.",
        // 8=Sandpaper  active = [0=AMOUNT, 1=THRESHOLD, 3=REACH, 4=MIX]
        (ModuleType::Life, 8, 0) => "Sandpaper · AMOUNT — phase-friction intensity. Sparks of granular noise emit upward when bins rub against each other.",
        (ModuleType::Life, 8, 1) => "Sandpaper · THRESHOLD — friction floor. Quiet bins don't generate sparks.",
        (ModuleType::Life, 8, 3) => "Sandpaper · REACH — spark spread radius into upper bins.",
        (ModuleType::Life, 8, 4) => "Sandpaper · MIX — wet/dry.",
        // 9=Brownian  active = [0=AMOUNT, 4=MIX]
        (ModuleType::Life, 9, 0) => "Brownian · AMOUNT — temperature scaling on the per-bin random-walk. Hotter = wilder magnitude jitter.",
        (ModuleType::Life, 9, 4) => "Brownian · MIX — wet/dry.",

        // ── Circuit (SC: no) ──
        // CircuitMode order from popup: Crossover(0), SpectralSchmitt(1), BbdBins(2), Vactrol(3),
        //                               TransformerSaturation(4), PowerSag(5), ComponentDrift(6),
        //                               PcbCrosstalk(7), SlewDistortion(8), BiasFuzz(9)
        // Most modes use AMOUNT(0), THRESH(1), SPREAD(2), RELEASE(3), MIX(4) selectively.
        (ModuleType::Circuit, 0, 0) => "Crossover Distortion · AMOUNT — diode deadzone strength. Higher = wider silenced region around zero, more sputtering on tails.",
        (ModuleType::Circuit, 0, 1) => "Crossover Distortion · THRESH — diode threshold (deadzone width). Bins below this are silenced, bins above re-emerge smoothly.",
        (ModuleType::Circuit, 0, 3) => "Crossover Distortion · RELEASE — re-emergence smoothing time when bins cross out of the deadzone.",
        (ModuleType::Circuit, 0, 4) => "Crossover Distortion · MIX — wet/dry.",
        (ModuleType::Circuit, 1, 0) => "Spectral Schmitt · AMOUNT — latch attenuation depth on bins below the lower trip point.",
        (ModuleType::Circuit, 1, 1) => "Spectral Schmitt · THRESH — upper trip point. Bins above latch on; the lower trip sits below this with hysteresis.",
        (ModuleType::Circuit, 1, 3) => "Spectral Schmitt · RELEASE — latch decay time when a bin drops below the lower trip.",
        (ModuleType::Circuit, 1, 4) => "Spectral Schmitt · MIX — wet/dry.",
        (ModuleType::Circuit, 2, 0) => "BBD Bins · AMOUNT — bucket-brigade depth (how aggressively each stage's LP smooths the magnitude).",
        (ModuleType::Circuit, 2, 4) => "BBD Bins · MIX — wet/dry of the 4-stage delayed/dithered output vs original.",
        (ModuleType::Circuit, 3, 0) => "Vactrol · AMOUNT — soft-saturation drive applied via the cascaded fast/slow opto-coupler caps. Reads BinPhysics flux.",
        (ModuleType::Circuit, 3, 3) => "Vactrol · RELEASE — slow-cap time constant. Longer = more pronounced opto-coupler ringing on transient releases.",
        (ModuleType::Circuit, 3, 4) => "Vactrol · MIX — wet/dry.",
        (ModuleType::Circuit, 4, 0) => "Transformer Saturation · AMOUNT — tanh saturation drive (heavy CPU mode).",
        (ModuleType::Circuit, 4, 1) => "Transformer Saturation · THRESH — saturation knee threshold.",
        (ModuleType::Circuit, 4, 2) => "Transformer Saturation · SPREAD — flux leak strength to neighbour bins (analogue-style flux coupling).",
        (ModuleType::Circuit, 4, 3) => "Transformer Saturation · RELEASE — magnitude one-pole time constant for the saturation envelope.",
        (ModuleType::Circuit, 4, 4) => "Transformer Saturation · MIX — wet/dry.",
        (ModuleType::Circuit, 5, 0) => "Power Sag · AMOUNT — sag depth. Loud sustained bins drive a sag envelope that pulls all bins down proportionally.",
        (ModuleType::Circuit, 5, 1) => "Power Sag · THRESH — energy threshold above which sag accumulates; reads BinPhysics temperature.",
        (ModuleType::Circuit, 5, 3) => "Power Sag · RELEASE — recovery time after the load lifts.",
        (ModuleType::Circuit, 5, 4) => "Power Sag · MIX — wet/dry.",
        (ModuleType::Circuit, 6, 0) => "Component Drift · AMOUNT — LFSR-driven random per-bin gain wander depth. Reads/writes BinPhysics temperature.",
        (ModuleType::Circuit, 6, 1) => "Component Drift · THRESH — heat threshold above which bins drift further (hot bins drift more).",
        (ModuleType::Circuit, 6, 3) => "Component Drift · RELEASE — drift smoothing time. Faster = noisier wobble; slower = oceanic detune.",
        (ModuleType::Circuit, 6, 4) => "Component Drift · MIX — wet/dry.",
        (ModuleType::Circuit, 7, 0) => "PCB Crosstalk · AMOUNT — leak strength via a 3-tap symmetric stencil into adjacent bins (analogue trace-coupling analogue).",
        (ModuleType::Circuit, 7, 2) => "PCB Crosstalk · SPREAD — stencil width / asymmetry between left/right neighbour leak amounts.",
        (ModuleType::Circuit, 7, 4) => "PCB Crosstalk · MIX — wet/dry.",
        (ModuleType::Circuit, 8, 0) => "Slew Distortion · AMOUNT — rate-limit aggression. Slowed transients spit excess slew energy out as phase scramble.",
        (ModuleType::Circuit, 8, 1) => "Slew Distortion · THRESH — slew rate ceiling (any rate above this gets clipped + scrambled).",
        (ModuleType::Circuit, 8, 4) => "Slew Distortion · MIX — wet/dry.",
        (ModuleType::Circuit, 9, 0) => "Bias Fuzz · AMOUNT — clip amount against the asymmetric top-rail.",
        (ModuleType::Circuit, 9, 1) => "Bias Fuzz · THRESH — top-rail gain. Reads/writes BinPhysics bias.",
        (ModuleType::Circuit, 9, 2) => "Bias Fuzz · SPREAD — bias-leak between adjacent bins.",
        (ModuleType::Circuit, 9, 3) => "Bias Fuzz · RELEASE — bias envelope time constant. Slow = sustained DC offset character; fast = jittery.",
        (ModuleType::Circuit, 9, 4) => "Bias Fuzz · MIX — wet/dry.",

        // ── Punch (SC: yes — both modes carve at sidechain peaks/troughs) ──
        // Punch curves: AMOUNT(0), WIDTH(1), FILL_MODE(2), AMP_FILL(3), HEAL(4), MIX(5)
        (ModuleType::Punch, 0, 0) => "Punch · Direct · AMOUNT — notch depth at sidechain spectral peaks. Carves room for the sidechain's loudest frequencies in this signal.",
        (ModuleType::Punch, 0, 1) => "Punch · Direct · WIDTH — notch width per peak in bins. Wider = deeper carved valley.",
        (ModuleType::Punch, 0, 2) => "Punch · Direct · FILL_MODE — selects the fill kernel for neighbours: 0 = Gaussian, 1 = triangle, 2 = boxcar, etc.",
        (ModuleType::Punch, 0, 3) => "Punch · Direct · AMP_FILL — boost depth applied to neighbours of carved bins. Compensates the carved energy by pushing it sideways.",
        (ModuleType::Punch, 0, 4) => "Punch · Direct · HEAL — recovery time after the SC peak dissolves. Slow heal = ducking that lingers.",
        (ModuleType::Punch, 0, 5) => "Punch · Direct · MIX — wet/dry of carved-and-filled vs original.",
        (ModuleType::Punch, 1, 0) => "Punch · Inverse · AMOUNT — same carve mechanic as Direct, but at sidechain spectral troughs (carves where SC is quietest).",
        (ModuleType::Punch, 1, 1) => "Punch · Inverse · WIDTH — notch width.",
        (ModuleType::Punch, 1, 2) => "Punch · Inverse · FILL_MODE — fill kernel choice.",
        (ModuleType::Punch, 1, 3) => "Punch · Inverse · AMP_FILL — neighbour boost depth.",
        (ModuleType::Punch, 1, 4) => "Punch · Inverse · HEAL — recovery time.",
        (ModuleType::Punch, 1, 5) => "Punch · Inverse · MIX — wet/dry.",

        // ── Rhythm (SC: no — drives entirely from host transport / Sync 1/16) ──
        // Rhythm curves: AMOUNT(0), DIVISION(1), ATTACK_FADE(2), TARGET_PHASE(3), MIX(4)
        (ModuleType::Rhythm, 0, 0) => "Euclidean · AMOUNT — pulse intensity. 0 = inaudible, 1 = full gating.",
        (ModuleType::Rhythm, 0, 1) => "Euclidean · DIVISION — number of pulses (k) and steps (n). Maps the curve value to k/n via Bjorklund's algorithm for evenly-distributed pulses.",
        (ModuleType::Rhythm, 0, 2) => "Euclidean · ATTACK_FADE — step-edge ramp time. Fast = clicky gating, slow = smooth tremolo-like pulses.",
        (ModuleType::Rhythm, 0, 4) => "Euclidean · MIX — wet/dry.",
        (ModuleType::Rhythm, 1, 0) => "Arpeggiator · AMOUNT — voice-gate depth. The 8-voice step sequencer gates spectral peaks per step.",
        (ModuleType::Rhythm, 1, 1) => "Arpeggiator · DIVISION — sequencer step count. Higher = faster arpeggio.",
        (ModuleType::Rhythm, 1, 2) => "Arpeggiator · ATTACK_FADE — gate ramp-up time per step.",
        (ModuleType::Rhythm, 1, 4) => "Arpeggiator · MIX — wet/dry.",
        (ModuleType::Rhythm, 2, 0) => "Phase Reset · AMOUNT — strength of the per-bin phase snap toward TARGET_PHASE at each step boundary.",
        (ModuleType::Rhythm, 2, 1) => "Phase Reset · DIVISION — step count over the bar.",
        (ModuleType::Rhythm, 2, 2) => "Phase Reset · ATTACK_FADE — edge window. Very small windows give tight, percussive snaps; wider windows smear the reset.",
        (ModuleType::Rhythm, 2, 3) => "Phase Reset · TARGET_PHASE — phase target in [-π, +π]. 0 = align all bins to zero phase (maximum impulse-like transient).",
        (ModuleType::Rhythm, 2, 4) => "Phase Reset · MIX — wet/dry.",

        // ── Geometry (SC: no) ──
        // Curves: AMOUNT(0), MODE_CAP(1), DAMP_REL(2), THRESH(3), MIX(4)
        (ModuleType::Geometry, 0, 0) => "Chladni · AMOUNT — emphasis depth at the plate's nodal-line frequencies. Carves the spectrum along the chosen Chladni pattern.",
        (ModuleType::Geometry, 0, 1) => "Chladni · MODE_CAP — maximum modal mode index. Higher = denser nodal pattern.",
        (ModuleType::Geometry, 0, 2) => "Chladni · DAMP_REL — modal damping. Low = sharp resonances; high = blurred pattern.",
        (ModuleType::Geometry, 0, 4) => "Chladni · MIX — wet/dry.",
        (ModuleType::Geometry, 1, 0) => "Helmholtz · AMOUNT — resonator strength on a single tuned cavity-frequency band.",
        (ModuleType::Geometry, 1, 1) => "Helmholtz · MODE_CAP — neck length / cavity volume — sets the resonant frequency.",
        (ModuleType::Geometry, 1, 2) => "Helmholtz · DAMP_REL — Q-control on the resonator. Low damping = ringing tone; high = subtle bump.",
        (ModuleType::Geometry, 1, 3) => "Helmholtz · THRESH — overflow trigger. Energy above this threshold drives the cavity into resonance.",
        (ModuleType::Geometry, 1, 4) => "Helmholtz · MIX — wet/dry.",

        _ => "",
    }
}

/// Per-curve help for single-mode modules (those without `active_layout`).
/// Returns "" if no entry — caller falls back to the module summary.
fn single_mode_curve_help(ty: ModuleType, curve_idx: usize) -> &'static str {
    match (ty, curve_idx) {
        // Dynamics — 6 curves, all per-bin. SC: yes.
        (ModuleType::Dynamics, 0) => "Dynamics · THRESHOLD — per-bin level above which compression engages. Drawn in dBFS via the calibrated curve display. Sidechain (when patched) feeds the detector instead of the through signal.",
        (ModuleType::Dynamics, 1) => "Dynamics · RATIO — per-bin compression ratio. 1:1 = no compression; 4:1 = strong; 20:1 = brick wall. Negative is an upward-expander direction.",
        (ModuleType::Dynamics, 2) => "Dynamics · ATTACK — per-bin attack-time multiplier on the global Atk knob (slot row). 1.0 = global value; values up/down scale per bin so high frequencies can react faster than low.",
        (ModuleType::Dynamics, 3) => "Dynamics · RELEASE — per-bin release-time multiplier on the global Rel knob. Same scaling as ATTACK.",
        (ModuleType::Dynamics, 4) => "Dynamics · KNEE — soft-knee width in dB. 0 = hard knee; higher widens the smooth transition above THRESHOLD so compression engages gradually.",
        (ModuleType::Dynamics, 5) => "Dynamics · MIX — per-bin wet/dry of compressed vs unprocessed for that bin. 100% = full compressed output.",

        // Freeze — 5 curves. SC: yes.
        (ModuleType::Freeze, 0) => "Freeze · LENGTH — per-bin freeze duration. Longer = the captured moment holds for more hops before releasing back to live audio.",
        (ModuleType::Freeze, 1) => "Freeze · THRESHOLD — per-bin gate. Bins below threshold pass live; bins at/above are eligible to freeze. Sidechain (when patched) replaces the bin's magnitude in the comparison.",
        (ModuleType::Freeze, 2) => "Freeze · PORTAMENTO — glide time from the live magnitude to the frozen magnitude when a bin enters freeze. Range 0..750 ms (neutral curve gain = 150 ms). 0 ms = instant snap.",
        (ModuleType::Freeze, 3) => "Freeze · RESISTANCE — how strongly the frozen bin resists being overwritten by louder live audio. 0 = a louder transient retriggers the freeze; 1 = the freeze is locked.",
        (ModuleType::Freeze, 4) => "Freeze · MIX — per-bin wet/dry between the frozen and live signal.",

        // PhaseSmear — 3 curves. SC: yes.
        (ModuleType::PhaseSmear, 0) => "Phase Smear · AMOUNT — per-bin phase-randomization depth. 0 = unchanged; 1 = full phase scramble (transients turn into smear and pads become diffuse). Sidechain steers the smear strength when patched.",
        (ModuleType::PhaseSmear, 1) => "Phase Smear · PEAK HOLD — peak-envelope hold time per bin. Smearing follows a peak follower with this hold time so quiet bins are smeared longer than the transient that just rang them.",
        (ModuleType::PhaseSmear, 2) => "Phase Smear · MIX — per-bin wet/dry between smeared and original phase.",

        // Contrast — 1 curve. SC: no.
        (ModuleType::Contrast, 0) => "Contrast · AMOUNT — per-bin transient/spectral-contrast amount. Positive sharpens (peaks louder, valleys quieter); negative softens. The Sens / Width knobs in the Dynamics group below shape the detector envelope.",

        // Gain — 2 curves; PEAK HOLD only meaningful in Pull/Match. SC: yes for Pull/Match.
        (ModuleType::Gain, 0) => "Gain · GAIN — per-bin spectral gain. Add / Subtract / Pull / Match modes (selector below) change how this curve is applied: Add adds to input, Subtract carves, Pull pulls toward the curve over time, Match shapes input to the curve.",
        (ModuleType::Gain, 1) => "Gain · PEAK HOLD — peak-envelope hold time per bin. Active only in Pull and Match modes; sets how long the peak follower latches before falling back, controlling the speed of the pull/match action.",

        // MidSide — 5 curves. SC: no.
        (ModuleType::MidSide, 0) => "Mid/Side · BALANCE — per-bin mid/side gain balance. v=0 leaves mid/side untouched; v<0 favours mid (mono-up the bin); v>0 favours side (widen the bin).",
        (ModuleType::MidSide, 1) => "Mid/Side · EXPANSION — per-bin M/S width expansion. Positive widens the side relative to mid, negative narrows.",
        (ModuleType::MidSide, 2) => "Mid/Side · DECORREL — per-bin phase decorrelation between L and R. Higher = wider stereo image at that frequency by injecting phase difference between channels.",
        (ModuleType::MidSide, 3) => "Mid/Side · TRANSIENT — per-bin transient bias toward mid. Useful for keeping kicks/snares mono while widening the surrounding pads.",
        (ModuleType::MidSide, 4) => "Mid/Side · PAN — per-bin pan offset. v=0 centres; v=±1 pans fully L/R for that bin only.",

        // TsSplit — 1 curve. SC: no.
        (ModuleType::TransientSustainedSplit, 0) => "T/S Split · SENSITIVITY — per-bin transient/sustained discriminator sensitivity. Higher = bins are more readily classed as transient. The split feeds two virtual rows in the routing matrix (look for the slot's 'T' and 'S' rows beneath it).",

        _ => "",
    }
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
        let body = body_text(Some(&layout), 0u8, ModuleType::Past, 0, &["A", "B", "C", "D", "E"]);
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
        let body = body_text(Some(&layout), 0u8, ModuleType::Past, 2, &["A", "B", "C", "D", "E"]);
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
        let body = body_text(Some(&layout), 0u8, ModuleType::Past, 1, &["A", "B", "C"]);
        assert_eq!(body, "mode-overview");
    }

    #[test]
    fn body_text_falls_through_to_static_when_no_layout() {
        let body = body_text(None, 0u8, ModuleType::Past, 0, &["AMOUNT", "TIME", "THRESHOLD", "SPREAD", "MIX"]);
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
