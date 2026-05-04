// THE ONLY file that defines visual constants. Reskin by editing this file.

use nih_plug_egui::egui::Color32;

// ─── LCH colour conversion ────────────────────────────────────────────────────

/// Convert CIE LCH (D65) to egui Color32, clamping out-of-gamut values.
/// L: 0–100, C: 0–150, H: 0–360 degrees.
fn lch_to_srgb(l: f32, c: f32, h_deg: f32) -> Color32 {
    let h = h_deg.to_radians();
    let a = c * h.cos();
    let b_lab = c * h.sin();
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b_lab / 200.0;
    let x = 0.95047 * lab_f_inv(fx);
    let y = 1.00000 * lab_f_inv(fy);
    let z = 1.08883 * lab_f_inv(fz);
    let r_lin =  3.2406 * x - 1.5372 * y - 0.4986 * z;
    let g_lin = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let b_lin =  0.0557 * x - 0.2040 * y + 1.0570 * z;
    Color32::from_rgb(linear_to_u8(r_lin), linear_to_u8(g_lin), linear_to_u8(b_lin))
}

#[inline] fn lab_f_inv(t: f32) -> f32 {
    const D: f32 = 6.0 / 29.0;
    if t > D { t * t * t } else { 3.0 * D * D * (t - 4.0 / 29.0) }
}

#[inline] fn linear_to_u8(v: f32) -> u8 {
    let e = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (e.clamp(0.0, 1.0) * 255.0).round() as u8
}

// ─── Per-curve colours ────────────────────────────────────────────────────────
// 7 curves, H equidistant: 0°, 51.4°, 102.9°, 154.3°, 205.7°, 257.1°, 308.6°

fn build_curve_colors() -> ([Color32; 7], [Color32; 7], [Color32; 7]) {
    let mut lit  = [Color32::WHITE; 7]; // L=75 C=50 — active
    let mut dim  = [Color32::WHITE; 7]; // L=30 C=50 — inactive
    let mut text = [Color32::WHITE; 7]; // L=15 C=30 — button text when active
    for i in 0..7 {
        let h = (i as f32) * (360.0 / 7.0);
        lit[i]  = lch_to_srgb(75.0, 50.0, h);
        dim[i]  = lch_to_srgb(30.0, 50.0, h);
        text[i] = lch_to_srgb(15.0, 30.0, h);
    }
    (lit, dim, text)
}

static CURVE_COLORS: std::sync::OnceLock<([Color32; 7], [Color32; 7], [Color32; 7])> =
    std::sync::OnceLock::new();

fn colors() -> &'static ([Color32; 7], [Color32; 7], [Color32; 7]) {
    CURVE_COLORS.get_or_init(build_curve_colors)
}

/// Lit (L=75) per-curve colour for curve index i.
pub fn curve_color_lit(i: usize) -> Color32  { colors().0[i.min(6)] }
/// Dim (L=30) per-curve colour for curve index i.
pub fn curve_color_dim(i: usize) -> Color32  { colors().1[i.min(6)] }
/// Dark text colour (L=15) to use on a lit curve-coloured button background.
pub fn curve_color_text_on(i: usize) -> Color32 { colors().2[i.min(6)] }

// ─── Fixed semantic colours ───────────────────────────────────────────────────

/// Pre-FX spectrum peak line (#7ad6d8 — turquoise).
pub const SPECTRUM_LINE: Color32 = Color32::from_rgb(0x7a, 0xd6, 0xd8);
/// Post-FX output line (#f8b6a4 — coral/salmon).
pub const POSTFX_LINE:   Color32 = Color32::from_rgb(0xf8, 0xb6, 0xa4);
/// Sidechain suppression gradient top (#b8ce95 — sage green).
pub const SC_LINE_A:     Color32 = Color32::from_rgb(0xb8, 0xce, 0x95);
/// Sidechain suppression gradient bottom (#eeb5e1 — lavender pink).
pub const SC_LINE_B:     Color32 = Color32::from_rgb(0xee, 0xb5, 0xe1);

pub const BG:            Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
pub const BG_RAISED:     Color32 = Color32::from_rgb(0x20, 0x20, 0x20);
pub const BG_FEEDBACK:   Color32 = Color32::from_rgb(0x14, 0x14, 0x1e);
pub const GRID_LINE:     Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const GRID_TEXT:     Color32 = Color32::from_rgb(0x45, 0x45, 0x45);
pub const TRUE_TIME_LINE:Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
pub const BORDER:        Color32 = Color32::from_rgb(0x00, 0x88, 0x80);
pub const LABEL_DIM:     Color32 = Color32::from_rgb(0x44, 0x88, 0x80);
/// Lit module slot color (Dynamics, selected).
pub const MODULE_COLOR_LIT: Color32 = Color32::from_rgb(0x50, 0xc0, 0xc4);
/// Dim module slot color (Dynamics, unselected).
pub const MODULE_COLOR_DIM: Color32 = Color32::from_rgb(0x20, 0x40, 0x41);
/// Geometry module dot color (teal/green, matches ModuleSpec color_lit).
pub const GEOMETRY_DOT_COLOR: Color32 = Color32::from_rgb(0x50, 0xb4, 0xa0);
/// Modulate module dot color (purple-magenta, matches ModuleSpec color_lit).
pub const MODULATE_DOT_COLOR: Color32 = Color32::from_rgb(180, 100, 200);
/// Circuit module — copper/orange for "analog component" feel.
pub const CIRCUIT_DOT_COLOR: Color32 = Color32::from_rgb(200, 140, 80);
/// Life module — warm green for "biology / fluid life" feel.
pub const LIFE_DOT_COLOR: Color32 = Color32::from_rgb(110, 185, 100);
/// Past module — muted violet for "temporal memory / echo" feel.
pub const PAST_DOT_COLOR: Color32 = Color32::from_rgb(0xa0, 0x80, 0xb0);
/// Kinetics module — warm orange for "force / momentum" feel.
pub const KINETICS_DOT_COLOR: Color32 = Color32::from_rgb(0xc8, 0x80, 0x40);

// ─── Freeze curve colours (4 equidistant, 30°, 120°, 210°, 300°) ─────────────

fn build_freeze_colors() -> ([Color32; 4], [Color32; 4]) {
    let mut lit = [Color32::WHITE; 4];
    let mut dim = [Color32::WHITE; 4];
    for i in 0..4 {
        let h = 30.0 + (i as f32) * 90.0;
        lit[i] = lch_to_srgb(75.0, 50.0, h);
        dim[i] = lch_to_srgb(30.0, 50.0, h);
    }
    (lit, dim)
}

static FREEZE_COLORS: std::sync::OnceLock<([Color32; 4], [Color32; 4])> =
    std::sync::OnceLock::new();
fn freeze_colors() -> &'static ([Color32; 4], [Color32; 4]) {
    FREEZE_COLORS.get_or_init(build_freeze_colors)
}

/// Lit (L=75) colour for freeze curve i.
pub fn freeze_color_lit(i: usize) -> Color32 { freeze_colors().0[i.min(3)] }
/// Dim (L=30) colour for freeze curve i.
pub fn freeze_color_dim(i: usize) -> Color32 { freeze_colors().1[i.min(3)] }

// ─── Phase curve colour (H=270°, purple) ──────────────────────────────────────

static PHASE_COLOR: std::sync::OnceLock<(Color32, Color32)> = std::sync::OnceLock::new();
fn phase_color_inner() -> &'static (Color32, Color32) {
    PHASE_COLOR.get_or_init(|| (lch_to_srgb(75.0, 50.0, 270.0), lch_to_srgb(30.0, 50.0, 270.0)))
}
pub fn phase_color_lit() -> Color32 { phase_color_inner().0 }
pub fn phase_color_dim() -> Color32 { phase_color_inner().1 }

// ─── SC level meter ───────────────────────────────────────────────────────────

pub const SC_METER_HEIGHT_PX: f32 = 4.0;
pub const SC_METER_WIDTH_PX:  f32 = 80.0;
pub const SC_METER_COLOR_LIT: Color32 = Color32::from_rgb(0xe0, 0xc0, 0x30);
pub const SC_METER_COLOR_DIM: Color32 = Color32::from_rgb(0x55, 0x48, 0x10);

// ─── Stroke widths & geometry ─────────────────────────────────────────────────

pub const STROKE_HAIRLINE: f32 = 0.5;
pub const STROKE_THIN:     f32 = 1.0;
pub const STROKE_BORDER:   f32 = 1.0;
pub const STROKE_CURVE:    f32 = 1.0;
pub const STROKE_MEDIUM:   f32 = 1.5;
pub const NODE_RADIUS:     f32 = 5.0;

// ─── Font size base values (at 1× scale) ─────────────────────────────────────

/// Grid axis labels (Hz / value markers).
pub const FONT_SIZE_GRID:         f32 = 9.0;
/// Cursor hover tooltip.
pub const FONT_SIZE_HOVER:        f32 = 10.0;
/// Compact button text (FFT/Scale choice buttons, slot-header labels).
pub const FONT_SIZE_BUTTON:       f32 = 10.0;
/// Widget labels (Tilt, Offset, Curv, etc.).
pub const FONT_SIZE_LABEL:        f32 = 9.0;
/// Primary button text (curve selector labels, slot header).
pub const FONT_SIZE_VALUE:        f32 = 11.0;
/// Secondary text (popup tertiary notes, disabled captions).
pub const FONT_SIZE_TINY:         f32 = 8.0;
/// FX matrix column header (top axis) labels.
pub const FONT_SIZE_MATRIX_AXIS:  f32 = 7.5;
/// FX matrix row-label text (left axis, slightly larger than column header).
pub const FONT_SIZE_MATRIX_ROW:   f32 = 8.5;
/// FX matrix cell text.
pub const FONT_SIZE_MATRIX_CELL:  f32 = 8.0;
/// FX matrix T/S virtual-row icon and self-send markers.
pub const FONT_SIZE_MATRIX_VROW:  f32 = 7.0;

// ─── Modulation Ring overlay ──────────────────────────────────────────────────

pub const MOD_RING_RADIUS:     f32     = 16.0;
pub const MOD_RING_DOT_RADIUS: f32     = 4.0;
pub const MOD_RING_LIT:      Color32 = Color32::from_rgb(0xff, 0xc8, 0x40);
pub const MOD_RING_DIM:      Color32 = Color32::from_rgb(0x60, 0x40, 0x18);
pub const MOD_RING_DISABLED: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);

// ─── Scaling helpers ──────────────────────────────────────────────────────────

/// Scale a layout measurement by the UI scale factor.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4.
#[inline]
pub fn scaled(base: f32, scale: f32) -> f32 { base * scale }

/// Scale a stroke width; snaps to 2× at scale ≥ 1.75 to avoid blurry sub-pixel rendering.
/// See docs/superpowers/specs/2026-04-23-ui-parameter-spec-design.md §4.
#[inline]
pub fn scaled_stroke(base: f32, scale: f32) -> f32 {
    if scale >= 1.75 { base * 2.0 } else { base * scale }
}

/// Amp-mode indicator dot colours, indexed by `AmpMode as usize`.
/// Linear is transparent (no dot drawn).
pub const AMP_DOT_COLORS: [Color32; 5] = [
    Color32::TRANSPARENT,                         // Linear
    Color32::from_rgb(0xff, 0xa6, 0x3d),          // Vactrol  — warm orange
    Color32::from_rgb(0x6d, 0xc7, 0xff),          // Schmitt  — cool blue
    Color32::from_rgb(0xa3, 0xff, 0x9d),          // Slew     — pale green
    Color32::from_rgb(0xb3, 0x8d, 0xff),          // Stiction — violet
];

pub const AMP_DOT_RADIUS: f32 = 2.5;

// ─── Help-box panel (right of FX matrix) ─────────────────────────────────────

pub const HELP_BOX_WIDTH:        f32     = 240.0;
pub const HELP_BOX_PADDING:      f32     = 8.0;
pub const FONT_SIZE_HELP_HEAD:   f32     = 12.0;
pub const FONT_SIZE_HELP_BODY:   f32     = 10.0;
pub const HELP_BOX_BG:           Color32 = Color32::from_rgb(20, 20, 24);
pub const HELP_BOX_BORDER:       Color32 = Color32::from_rgb(60, 60, 68);
pub const HELP_BOX_BODY:         Color32 = Color32::from_rgb(190, 190, 196);
pub const HELP_BOX_HEAD:         Color32 = Color32::from_rgb(230, 230, 236);
