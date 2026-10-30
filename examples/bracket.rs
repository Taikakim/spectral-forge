//! Automatic parameter bracketing: every module × mode × curve, swept across the
//! offset slider and the curve-node gain range, on realistic STFT input.
//!
//! ```text
//! cargo run --release --example bracket -- [--threads N] [--only TEXT] [--hops N]
//!                                           [--out DIR] [--wav TEXT]
//! ```
//!
//! For each job (module, mode, curve, stimulus) it runs
//! - **static brackets**: a fresh module per setting — 13 offset-slider positions
//!   (−1…+1, what the Offset knob does) and 7 curve-node gains (−18…+18 dB, what
//!   dragging a node does) — all other curves at their default gain of 1.0;
//! - **a live tweak**: one module processing continuously while the curve ramps
//!   from its lowest to highest gain and back, to catch spikes caused by motion
//!   (state blow-ups, zipper jumps) rather than by the setting itself.
//!
//! Per run it measures overall gain, peak bin gain (loudest output bin vs the
//! loudest input its ±2-bin neighbourhood has ever had), output-peak change,
//! spread (share of the spectrum changed by > 1 dB), spill (energy where the
//! input had none) and non-finite bins, then flags cliffs, spikes, motion
//! spikes, dead curves, hidden effects and non-neutral defaults.
//!
//! Outputs in `--out` (default `target/bracket`): `settings.csv`, `live.csv`,
//! `baselines.csv`, `findings.csv`. `--wav TEXT` also renders the node brackets
//! of matching jobs ("Module/Mode" contains TEXT) to WAV for listening.
//!
//! Stimuli are generated at real levels and analysed exactly like the plugin:
//! Hann window, unnormalised forward real FFT, hop = FFT/4.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use num_complex::Complex;
use realfft::RealFftPlanner;

use spectral_forge::dsp::bin_physics::BinPhysics;
use spectral_forge::dsp::cepstrum::CepstrumBuf;
use spectral_forge::dsp::chromagram::compute_chromagram;
use spectral_forge::dsp::harmonic_groups::{HarmonicGroup, HarmonicGroupBuf};
use spectral_forge::dsp::history_buffer::HistoryBuffer;
use spectral_forge::dsp::instantaneous_freq::compute_instantaneous_freq;
use spectral_forge::dsp::modules::{
    circuit::CircuitMode, contrast::ContrastMode, future::FutureMode, geometry::GeometryMode,
    harmonic::HarmonicMode, harmony::HarmonyMode, kinetics::KineticsMode, life::LifeMode,
    modulate::ModulateMode, past::PastMode, punch::PunchMode, rhythm::RhythmMode,
    create_module, module_spec, GainMode, ModuleContext, ModuleType, SpectralModule,
};
use spectral_forge::editor::curve::{display_curve_idx, gain_to_display, runtime_anchors};
use spectral_forge::editor::curve_config::curve_display_config;
use spectral_forge::params::{FxChannelTarget, StereoLink};

const SR: f32 = 48_000.0;
const FFT: usize = 2048;
const HOP: usize = FFT / 4;
const NB: usize = FFT / 2 + 1;
/// Synthesis normalisation for Hann² overlap-add at 75 % overlap.
const OLA_NORM: f32 = 2.0 / (3.0 * FFT as f32);

/// Offset-slider positions swept (what the Offset knob covers).
const OFFSETS: [f32; 13] = [
    -1.0, -0.8333, -0.6667, -0.5, -0.3333, -0.1667, 0.0, 0.1667, 0.3333, 0.5, 0.6667, 0.8333, 1.0,
];
/// Curve-node gains swept: a drawn node spans ±18 dB.
const NODE_GAINS: [f32; 7] = [0.1259, 0.25, 0.5, 1.0, 2.0, 4.0, 7.943];

// Thresholds for findings.
const CLIFF_DB: f32 = 12.0;
/// A loud output bin this far above anything near it in the input (10×).
const SPIKE_BIN_DB: f32 = 20.0;
const SPIKE_PEAK_DB: f32 = 12.0;
const MOTION_DB: f32 = 12.0;
const DEAD_DB: f32 = 0.2;
const HIDDEN_DB: f32 = 1.0;
const DEFAULT_GAIN_DB: f32 = 1.0;
const DEFAULT_SPREAD_PCT: f32 = 10.0;

// ── Stimuli ───────────────────────────────────────────────────────────────

struct Stimulus {
    name: &'static str,
    /// [hop][ch] analysed input bins.
    bins:   Vec<[Vec<Complex<f32>>; 2]>,
    /// [hop] sidechain magnitudes (raw STFT magnitude of the SC signal, L).
    sc:     Vec<Vec<f32>>,
    ifreq:  Vec<[Vec<f32>; 2]>,
    groups: Vec<[Vec<HarmonicGroup>; 2]>,
    chroma: Vec<[[f32; 12]; 2]>,
    cep:    Vec<[Vec<f32>; 2]>,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

fn noise(len: usize, seed: u64) -> Vec<f32> {
    // White noise through a gentle one-pole to tilt it pink-ish; RMS ≈ −20 dBFS.
    let mut r = Rng(seed);
    let mut lp = 0.0f32;
    let raw: Vec<f32> = (0..len).map(|_| { let w = r.next(); lp = 0.97 * lp + 0.03 * w; w * 0.5 + lp * 4.0 }).collect();
    let rms = (raw.iter().map(|v| v * v).sum::<f32>() / len as f32).sqrt().max(1e-9);
    raw.iter().map(|v| v * 0.1 / rms).collect()
}

fn chord(len: usize) -> Vec<f32> {
    // Three sawtooth voices (A2, C#3-ish, E3), 20 partials each, peak ≈ −6 dBFS.
    let mut out = vec![0.0f32; len];
    for &f0 in &[110.0f32, 138.59, 164.81] {
        for n in 1..=20 {
            let f = f0 * n as f32;
            if f > SR * 0.45 { break; }
            let w = 2.0 * std::f32::consts::PI * f / SR;
            for (i, s) in out.iter_mut().enumerate() { *s += (w * i as f32).sin() / n as f32; }
        }
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    out.iter().map(|v| v * 0.5 / peak).collect()
}

fn drums(len: usize) -> Vec<f32> {
    // Kick on 1/3, snare on 2/4, hats on 8ths at 120 BPM.
    let mut r = Rng(0x5eed);
    let beat = (SR * 0.5) as usize;
    let mut out = vec![0.0f32; len];
    for (i, s) in out.iter_mut().enumerate() {
        let pos = i % (beat * 4);
        let b = pos / beat;
        let t = (pos % beat) as f32 / SR;
        if b == 0 || b == 2 {
            let f = 50.0 + 90.0 * (-t * 30.0).exp();
            *s += (2.0 * std::f32::consts::PI * f * t).sin() * (-t * 9.0).exp() * 0.8;
        } else {
            *s += r.next() * (-t * 25.0).exp() * 0.4;
        }
        let th = (i % (beat / 2)) as f32 / SR;
        *s += r.next() * (-th * 120.0).exp() * 0.15;
    }
    out
}

fn stft(x: &[f32], hops: usize, fft: &dyn realfft::RealToComplex<f32>, window: &[f32]) -> Vec<Vec<Complex<f32>>> {
    let mut frame = vec![0.0f32; FFT];
    (0..hops).map(|h| {
        for i in 0..FFT { frame[i] = x[h * HOP + i] * window[i]; }
        let mut spec = fft.make_output_vec();
        fft.process(&mut frame, &mut spec).unwrap();
        spec
    }).collect()
}

fn build_stimulus(name: &'static str, hops: usize) -> Stimulus {
    let len = hops * HOP + FFT;
    let (l, r, sc): (Vec<f32>, Vec<f32>, Vec<f32>) = match name {
        "noise" => (noise(len, 1), noise(len, 2), drums(len)),
        "chord" => {
            let c = chord(len + 11);
            (c[11..].to_vec(), c[..len].iter().map(|v| v * 0.8).collect(), drums(len))
        }
        _ => {
            let d = drums(len + 7);
            (d[7..].to_vec(), d[..len].iter().map(|v| v * 0.9).collect(), noise(len, 3))
        }
    };
    let window: Vec<f32> = (0..FFT)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT - 1) as f32).cos()))
        .collect();
    let mut planner = RealFftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(FFT);
    let sl = stft(&l, hops, fwd.as_ref(), &window);
    let sr = stft(&r, hops, fwd.as_ref(), &window);
    let ss = stft(&sc, hops, fwd.as_ref(), &window);

    let mut bins = Vec::with_capacity(hops);
    let mut ifreq = Vec::with_capacity(hops);
    let mut groups = Vec::with_capacity(hops);
    let mut chroma = Vec::with_capacity(hops);
    let mut cep = Vec::with_capacity(hops);
    let mut prev_phase = [vec![0.0f32; NB], vec![0.0f32; NB]];
    let mut gbuf = [HarmonicGroupBuf::new(), HarmonicGroupBuf::new()];
    let mut cbuf = CepstrumBuf::new(FFT);
    for h in 0..hops {
        let frame = [sl[h].clone(), sr[h].clone()];
        let mut ifr = [vec![0.0f32; NB], vec![0.0f32; NB]];
        let mut grp: [Vec<HarmonicGroup>; 2] = [Vec::new(), Vec::new()];
        let mut chr = [[0.0f32; 12]; 2];
        let mut cp: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        for ch in 0..2 {
            let phase: Vec<f32> = frame[ch].iter().map(|c| c.arg()).collect();
            compute_instantaneous_freq(&prev_phase[ch], &phase, &mut ifr[ch], SR, HOP, FFT);
            prev_phase[ch] = phase;
            gbuf[ch].detect(&frame[ch], &ifr[ch], SR, FFT);
            grp[ch] = gbuf[ch].groups().to_vec();
            compute_chromagram(&frame[ch], Some(ifr[ch].as_slice()), SR, FFT, &mut chr[ch]);
            cbuf.compute_from_bins(&frame[ch]);
            cp[ch] = cbuf.quefrency().to_vec();
        }
        bins.push(frame);
        ifreq.push(ifr);
        groups.push(grp);
        chroma.push(chr);
        cep.push(cp);
    }
    let sc = ss.iter().map(|f| f.iter().map(|c| c.norm()).collect()).collect();
    Stimulus { name, bins, sc, ifreq, groups, chroma, cep }
}

// ── Modes ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Mode {
    Single,
    Gain(GainMode),
    Future(FutureMode),
    Punch(PunchMode),
    Rhythm(RhythmMode),
    Geometry(GeometryMode),
    Modulate(ModulateMode),
    Harmonic(HarmonicMode),
    Circuit(CircuitMode),
    Life(LifeMode),
    Past(PastMode),
    Contrast(ContrastMode),
    Kinetics(KineticsMode),
    Harmony(HarmonyMode),
}

impl Mode {
    fn apply(self, m: &mut dyn SpectralModule) {
        match self {
            Mode::Single => {}
            Mode::Gain(x) => m.set_gain_mode(x),
            Mode::Future(x) => m.set_future_mode(x),
            Mode::Punch(x) => m.set_punch_mode(x),
            Mode::Rhythm(x) => m.set_rhythm_mode(x),
            Mode::Geometry(x) => m.set_geometry_mode(x),
            Mode::Modulate(x) => m.set_modulate_mode(x),
            Mode::Harmonic(x) => m.set_harmonic_mode(x),
            Mode::Circuit(x) => m.set_circuit_mode(x),
            Mode::Life(x) => m.set_life_mode(x),
            Mode::Past(x) => m.set_past_mode(x),
            Mode::Contrast(x) => m.set_contrast_mode(x),
            Mode::Kinetics(x) => m.set_kinetics_mode(x),
            Mode::Harmony(x) => m.set_harmony_mode(x),
        }
    }

    fn byte(self) -> u8 {
        match self {
            Mode::Single => 0,
            Mode::Gain(x) => x as u8,
            Mode::Future(x) => x as u8,
            Mode::Punch(x) => x as u8,
            Mode::Rhythm(x) => x as u8,
            Mode::Geometry(x) => x as u8,
            Mode::Modulate(x) => x as u8,
            Mode::Harmonic(x) => x as u8,
            Mode::Circuit(x) => x as u8,
            Mode::Life(x) => x as u8,
            Mode::Past(x) => x as u8,
            Mode::Contrast(x) => x as u8,
            Mode::Kinetics(x) => x as u8,
            Mode::Harmony(x) => x as u8,
        }
    }

    fn name(self) -> String {
        match self {
            Mode::Single => "-".into(),
            Mode::Gain(x) => format!("{x:?}"),
            Mode::Future(x) => format!("{x:?}"),
            Mode::Punch(x) => format!("{x:?}"),
            Mode::Rhythm(x) => format!("{x:?}"),
            Mode::Geometry(x) => format!("{x:?}"),
            Mode::Modulate(x) => format!("{x:?}"),
            Mode::Harmonic(x) => format!("{x:?}"),
            Mode::Circuit(x) => format!("{x:?}"),
            Mode::Life(x) => format!("{x:?}"),
            Mode::Past(x) => format!("{x:?}"),
            Mode::Contrast(x) => format!("{x:?}"),
            Mode::Kinetics(x) => format!("{x:?}"),
            Mode::Harmony(x) => format!("{x:?}"),
        }
    }

    fn gain_mode(self) -> GainMode {
        if let Mode::Gain(g) = self { g } else { GainMode::Add }
    }
}

fn modes_for(ty: ModuleType) -> Vec<Mode> {
    use Mode::*;
    match ty {
        ModuleType::Gain => [GainMode::Add, GainMode::Subtract, GainMode::Pull, GainMode::Match].map(Gain).to_vec(),
        ModuleType::Future => [FutureMode::PrintThrough, FutureMode::PreEcho].map(Future).to_vec(),
        ModuleType::Punch => [PunchMode::Direct, PunchMode::Inverse, PunchMode::SelfPunch].map(Punch).to_vec(),
        ModuleType::Rhythm => [RhythmMode::Euclidean, RhythmMode::Arpeggiator, RhythmMode::PhaseReset].map(Rhythm).to_vec(),
        ModuleType::Geometry => [GeometryMode::Chladni, GeometryMode::Helmholtz].map(Geometry).to_vec(),
        ModuleType::Modulate => [
            ModulateMode::PhasePhaser, ModulateMode::BinSwapper, ModulateMode::RmFmMatrix,
            ModulateMode::DiodeRm, ModulateMode::GroundLoop, ModulateMode::GravityPhaser,
            ModulateMode::PllTear, ModulateMode::FmNetwork, ModulateMode::SlewLag,
        ].map(Modulate).to_vec(),
        ModuleType::Harmonic => [HarmonicMode::HpSplit, HarmonicMode::PartialEq, HarmonicMode::Exciter].map(Harmonic).to_vec(),
        ModuleType::Circuit => [
            CircuitMode::BbdBins, CircuitMode::SpectralSchmitt, CircuitMode::CrossoverDistortion,
            CircuitMode::Vactrol, CircuitMode::TransformerSaturation, CircuitMode::PowerSag,
            CircuitMode::ComponentDrift, CircuitMode::PcbCrosstalk, CircuitMode::SlewDistortion,
            CircuitMode::BiasFuzz,
        ].map(Circuit).to_vec(),
        ModuleType::Life => [
            LifeMode::Viscosity, LifeMode::SurfaceTension, LifeMode::Crystallization,
            LifeMode::Archimedes, LifeMode::NonNewtonian, LifeMode::Stiction, LifeMode::Yield,
            LifeMode::Capillary, LifeMode::Sandpaper, LifeMode::Brownian,
        ].map(Life).to_vec(),
        ModuleType::Past => [
            PastMode::Granular, PastMode::DecaySorter, PastMode::Convolution,
            PastMode::Reverse, PastMode::Stretch,
        ].map(Past).to_vec(),
        ModuleType::Contrast => [ContrastMode::Spatial, ContrastMode::Temporal, ContrastMode::Tilt].map(Contrast).to_vec(),
        ModuleType::Kinetics => [
            KineticsMode::Hooke, KineticsMode::GravityWell, KineticsMode::InertialMass,
            KineticsMode::OrbitalPhase, KineticsMode::Ferromagnetism, KineticsMode::ThermalExpansion,
            KineticsMode::TuningFork, KineticsMode::Diamagnet,
        ].map(Kinetics).to_vec(),
        ModuleType::Harmony => [
            HarmonyMode::Chordification, HarmonyMode::Undertone, HarmonyMode::Companding,
            HarmonyMode::FormantRotation, HarmonyMode::Lifter, HarmonyMode::Inharmonic,
            HarmonyMode::HarmonicGenerator, HarmonyMode::Shuffler,
        ].map(Harmony).to_vec(),
        _ => vec![Single],
    }
}

const MODULES: [ModuleType; 18] = [
    ModuleType::Dynamics, ModuleType::Freeze, ModuleType::PhaseSmear, ModuleType::Contrast,
    ModuleType::Gain, ModuleType::MidSide, ModuleType::TransientSustainedSplit,
    ModuleType::Harmonic, ModuleType::Future, ModuleType::Punch, ModuleType::Rhythm,
    ModuleType::Geometry, ModuleType::Modulate, ModuleType::Circuit, ModuleType::Life,
    ModuleType::Past, ModuleType::Kinetics, ModuleType::Harmony,
];

// ── Running a module ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct Frame {
    e_in: f64,
    e_out: f64,
    /// Σ|out − in|² — sees phase-only processing that magnitude metrics miss.
    e_diff: f64,
    peak_bin_gain_db: f32,
    in_max: f32,
    out_max: f32,
    changed: u32,
    active: u32,
    spill_e: f64,
    nonfinite: u32,
}

/// Frequency neighbourhood (± bins) for the spike reference.
const REF_BINS: usize = 2;
/// Only output bins within this of the run's input peak count as spikes.
const SPIKE_LOUD: f32 = 0.1; // −20 dB

/// Per-channel running maximum of input magnitude over ±REF_BINS since the
/// start of the run: the loudest the input ever was near each bin.
struct RefRing { max_near: Vec<f32>, run_peak: f32 }

impl RefRing {
    fn new() -> Self { Self { max_near: vec![0.0; NB], run_peak: 0.0 } }

    fn push(&mut self, input: &[Complex<f32>]) {
        for k in 0..NB {
            let lo = k.saturating_sub(REF_BINS);
            let hi = (k + REF_BINS).min(NB - 1);
            let m = input[lo..=hi].iter().fold(0.0f32, |m, c| m.max(c.norm()));
            self.max_near[k] = self.max_near[k].max(m);
            self.run_peak = self.run_peak.max(m);
        }
    }

    fn reference(&self, k: usize) -> f32 { self.max_near[k] }
}

impl Frame {
    /// `peak_bin_gain_db` is the loudest output bin (within −20 dB of the run's
    /// input peak) relative to the loudest input its ±2-bin neighbourhood has
    /// ever had. Holding, freezing or delaying stays ≤ 0 dB; amplification and
    /// moving loud energy into a region that never had it read as positive.
    fn measure(input: &[Complex<f32>], output: &[Complex<f32>], refs: &RefRing) -> Self {
        let mut f = Frame { peak_bin_gain_db: -200.0, ..Default::default() };
        f.in_max = input.iter().fold(0.0f32, |m, c| m.max(c.norm()));
        let floor = (f.in_max * 1e-3).max(1e-9); // −60 dB re frame peak
        let loud = refs.run_peak * SPIKE_LOUD;
        for (k, (a, b)) in input.iter().zip(output).enumerate() {
            if !b.re.is_finite() || !b.im.is_finite() { f.nonfinite += 1; continue; }
            let (ma, mb) = (a.norm(), b.norm());
            let md = (b - a).norm();
            f.e_in += (ma as f64).powi(2);
            f.e_out += (mb as f64).powi(2);
            f.e_diff += (md as f64).powi(2);
            f.out_max = f.out_max.max(mb);
            if mb > loud && mb > 1e-9 {
                let g = 20.0 * (mb / refs.reference(k).max(refs.run_peak * 1e-3).max(1e-9)).log10();
                f.peak_bin_gain_db = f.peak_bin_gain_db.max(g);
            }
            if ma > floor {
                f.active += 1;
                // Complex change > ~1 dB equivalent (catches phase-only edits too).
                if md > 0.12 * ma { f.changed += 1; }
            } else if mb > floor {
                f.spill_e += (mb as f64).powi(2);
            }
        }
        f
    }
}

#[derive(Clone, Copy, Default)]
struct Summary {
    gain_db: f32,
    /// Energy of (out − in) relative to input: how much the slot changed the signal.
    delta_db: f32,
    peak_bin_gain_db: f32,
    peak_out_db: f32,
    spread_pct: f32,
    spill_db: f32,
    nonfinite: u32,
}

fn db10(x: f64) -> f32 { (10.0 * x.max(1e-30).log10()) as f32 }

fn summarise(frames: &[[Frame; 2]]) -> Summary {
    let (mut e_in, mut e_out, mut spill, mut e_diff) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut in_max, mut out_max) = (0.0f32, 0.0f32);
    let (mut changed, mut active, mut nonfinite) = (0u64, 0u64, 0u32);
    let mut peak = -200.0f32;
    for fr in frames {
        for f in fr {
            e_in += f.e_in; e_out += f.e_out; spill += f.spill_e; e_diff += f.e_diff;
            in_max = in_max.max(f.in_max); out_max = out_max.max(f.out_max);
            changed += f.changed as u64; active += f.active as u64;
            nonfinite += f.nonfinite;
            peak = peak.max(f.peak_bin_gain_db);
        }
    }
    Summary {
        gain_db: db10(e_out / e_in.max(1e-30)),
        delta_db: db10(e_diff / e_in.max(1e-30)),
        peak_bin_gain_db: peak,
        peak_out_db: 20.0 * (out_max.max(1e-12) / in_max.max(1e-12)).log10(),
        spread_pct: if active > 0 { changed as f32 / active as f32 * 100.0 } else { 0.0 },
        spill_db: db10(spill / e_in.max(1e-30)),
        nonfinite,
    }
}

struct RunOut {
    frames: Vec<[Frame; 2]>,
    audio:  Option<Vec<[Vec<Complex<f32>>; 2]>>,
}

/// Process `stim` through a fresh module. `gain_at(hop, curve)` gives each
/// curve's (flat) gain for that hop.
fn run_module(
    ty: ModuleType, mode: Mode, stim: &Stimulus, gain_at: &dyn Fn(usize, usize) -> f32, keep_audio: bool,
) -> RunOut {
    let spec = module_spec(ty);
    let mut module = create_module(ty, SR, FFT);
    mode.apply(module.as_mut());
    let nc = module.num_curves();
    let mut curves = vec![vec![1.0f32; NB]; nc];
    let mut phys = [BinPhysics::new(), BinPhysics::new()];
    let mut history = (ty == ModuleType::Past)
        .then(|| HistoryBuffer::new(2, (4.0 * SR / HOP as f32) as usize, NB));
    let mut buf = vec![Complex::new(0.0f32, 0.0); NB];
    let mut supp = vec![0.0f32; NB];
    let mut refs_ring = [RefRing::new(), RefRing::new()];
    let hops = stim.bins.len();
    let mut frames = Vec::with_capacity(hops);
    let mut audio = keep_audio.then(Vec::new);

    for h in 0..hops {
        for (c, cv) in curves.iter_mut().enumerate() { cv.fill(gain_at(h, c)); }
        let refs: Vec<&[f32]> = curves.iter().map(|v| v.as_slice()).collect();
        let mut fr = [Frame::default(); 2];
        let mut outs: [Vec<Complex<f32>>; 2] = [Vec::new(), Vec::new()];
        for ch in 0..2 {
            let input = &stim.bins[h][ch];
            buf.copy_from_slice(input);
            let mut ctx = ModuleContext::new(SR, FFT, NB, 10.0, 100.0, 1.0, 1.0, false, false);
            ctx.bpm = 120.0;
            ctx.beat_position = h as f64 * HOP as f64 / SR as f64 * 2.0;
            if spec.needs_instantaneous_freq || spec.needs_harmonic_groups {
                ctx.instantaneous_freq = Some(stim.ifreq[h][ch].as_slice());
            }
            if spec.needs_harmonic_groups { ctx.harmonic_groups = Some(stim.groups[h][ch].as_slice()); }
            if spec.needs_chromagram { ctx.chromagram = Some(&stim.chroma[h][ch]); }
            if spec.needs_cepstrum { ctx.cepstrum_buf = Some(stim.cep[h][ch].as_slice()); }
            if let Some(hb) = history.as_ref() {
                hb.clear_summary_cache();
                ctx.history = Some(hb);
            }
            let sc = spec.supports_sidechain.then(|| stim.sc[h].as_slice());
            let physics = if spec.writes_bin_physics {
                phys[ch].reset_active(NB, SR, FFT);
                Some(&mut phys[ch])
            } else {
                None
            };
            module.process(ch, StereoLink::Linked, FxChannelTarget::All,
                           &mut buf, sc, &refs, &mut supp, physics, &ctx);
            refs_ring[ch].push(input);
            fr[ch] = Frame::measure(input, &buf, &refs_ring[ch]);
            if keep_audio { outs[ch] = buf.clone(); }
        }
        if let Some(hb) = history.as_mut() {
            for ch in 0..2 { hb.write_hop(ch, &stim.bins[h][ch]); }
            hb.advance_after_all_channels_written();
        }
        frames.push(fr);
        if let Some(a) = audio.as_mut() { a.push(outs); }
    }
    RunOut { frames, audio }
}

// ── Jobs ──────────────────────────────────────────────────────────────────

struct Job { ty: ModuleType, mode: Mode, curve: usize, stim: usize }

struct Setting { kind: &'static str, offset: f32, gain: f32 }

fn settings_for(ty: ModuleType, mode: Mode, curve: usize) -> (Vec<Setting>, &'static str, usize) {
    let cfg = curve_display_config(ty, curve, mode.gain_mode());
    let didx = display_curve_idx(ty, curve, mode.gain_mode());
    let anchors = runtime_anchors(&cfg, didx, 4.0, -160.0, 0.0, 10.0, 100.0);
    let mut s: Vec<Setting> = OFFSETS.iter()
        .map(|&o| Setting { kind: "offset", offset: o, gain: (cfg.offset_fn)(1.0, o, anchors) })
        .collect();
    s.extend(NODE_GAINS.iter().map(|&g| Setting { kind: "node", offset: f32::NAN, gain: g }));
    (s, cfg.y_label, didx)
}

fn curve_label(ty: ModuleType, mode: Mode, curve: usize) -> (String, bool) {
    let spec = module_spec(ty);
    let base = spec.curve_labels.get(curve).copied().unwrap_or("?");
    match spec.active_layout.map(|f| f(mode.byte())) {
        Some(l) => {
            let label = l.label_overrides.iter().find(|(i, _)| *i as usize == curve)
                .map(|(_, s)| *s).unwrap_or(base);
            (label.to_string(), l.active.contains(&(curve as u8)))
        }
        None => (base.to_string(), true),
    }
}

struct JobResult {
    settings: String,
    live: String,
    findings: Vec<String>,
    baseline: Option<(ModuleType, String, &'static str, Summary)>,
}

fn csv_esc(s: &str) -> String { if s.contains(',') { format!("\"{s}\"") } else { s.to_string() } }

fn run_job(job: &Job, stims: &[Stimulus], measure_from: usize, wav: Option<(&str, &Path)>) -> JobResult {
    let stim = &stims[job.stim];
    let (ty, mode, c) = (job.ty, job.mode, job.curve);
    let mname = format!("{:?}", ty);
    let modename = mode.name();
    let (label, declared_active) = curve_label(ty, mode, c);
    let (settings, unit, didx) = settings_for(ty, mode, c);
    let tag = format!("{mname}/{modename}");
    let want_wav = wav.map(|(f, _)| tag.contains(f)).unwrap_or(false);

    let mut out = JobResult { settings: String::new(), live: String::new(), findings: Vec::new(), baseline: None };
    let mut finding = |sev: &str, kind: &str, detail: String| {
        out.findings.push(format!("{sev},{kind},{mname},{modename},{c},{},{},{}",
            csv_esc(&label), stim.name, csv_esc(&detail)));
    };

    // Static brackets.
    let mut sums: Vec<Summary> = Vec::with_capacity(settings.len());
    let mut rows = String::new();
    for (i, s) in settings.iter().enumerate() {
        let g = s.gain;
        let keep = want_wav && s.kind == "node";
        let run = run_module(ty, mode, stim, &|_, cc| if cc == c { g } else { 1.0 }, keep);
        let sum = summarise(&run.frames[measure_from.min(run.frames.len())..]);
        let disp = gain_to_display(didx, g, 10.0, 100.0, -160.0, 0.0, 4.0);
        let _ = writeln!(rows, "{mname},{modename},{c},{},{},{},{:.4},{:.5},{:.3},{},{:.2},{:.2},{:.2},{:.2},{:.1},{:.1},{}",
            csv_esc(&label), stim.name, s.kind, s.offset, g, disp, unit,
            sum.gain_db, sum.delta_db, sum.peak_bin_gain_db, sum.peak_out_db, sum.spread_pct, sum.spill_db, sum.nonfinite);
        if let (Some(audio), Some((_, dir))) = (run.audio, wav) {
            let name = format!("{mname}_{modename}_c{c}_{}_{}_g{:.3}.wav", stim.name, s.kind, g)
                .replace('/', "-");
            write_wav(&dir.join(name), &audio);
        }
        if i == 6 && job.curve == 0 {
            // offset 0 with every curve at 1.0 is the module's default state
            // (natural-at-max offsets default to +1, which maps gain 1.0 → 1.0 too).
            out.baseline = Some((ty, modename.clone(), stim.name, sum));
        }
        sums.push(sum);
    }
    out.settings = rows;

    // Findings from the static sweep.
    let worst_nonfinite = sums.iter().map(|s| s.nonfinite).max().unwrap_or(0);
    if worst_nonfinite > 0 {
        finding("FATAL", "nonfinite", format!("{worst_nonfinite} NaN/Inf bins in a setting"));
    }
    for (i, s) in sums.iter().enumerate() {
        if s.peak_bin_gain_db > SPIKE_BIN_DB {
            finding("HIGH", "bin-spike",
                format!("{} gain {:.3}: a loud bin is {:.1} dB above any nearby input", settings[i].kind, settings[i].gain, s.peak_bin_gain_db));
            break;
        }
    }
    for (i, s) in sums.iter().enumerate() {
        if s.peak_out_db > SPIKE_PEAK_DB {
            finding("HIGH", "peak-spike",
                format!("{} gain {:.3}: output peak +{:.1} dB over input", settings[i].kind, settings[i].gain, s.peak_out_db));
            break;
        }
    }
    for range in [0..OFFSETS.len(), OFFSETS.len()..settings.len()] {
        let mut worst = (0.0f32, 0usize);
        for i in range.start + 1..range.end {
            let d = (sums[i].gain_db - sums[i - 1].gain_db).abs();
            if d > worst.0 { worst = (d, i); }
        }
        if worst.0 > CLIFF_DB {
            let i = worst.1;
            finding("MED", "cliff", format!("{} step gain {:.3} -> {:.3} jumps {:.1} dB",
                settings[i].kind, settings[i - 1].gain, settings[i].gain, worst.0));
        }
    }
    let span = |f: &dyn Fn(&Summary) -> f32| {
        let v: Vec<f32> = sums.iter().map(|s| f(s)).filter(|x| x.is_finite()).collect();
        v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
    };
    let gain_span = span(&|s| s.gain_db);
    let delta_span = span(&|s| s.delta_db.max(-120.0));
    let spread_span = span(&|s| s.spread_pct);
    let peak_span = span(&|s| s.peak_bin_gain_db);
    if declared_active && gain_span < DEAD_DB && delta_span < 1.0 && spread_span < 1.0 && peak_span < 0.5 {
        finding("LOW", "dead-curve", format!(
            "no measurable effect with other curves at default (gain span {gain_span:.2} dB, change span {delta_span:.2} dB)"));
    }
    if !declared_active && (gain_span > HIDDEN_DB || delta_span > 3.0 || peak_span > HIDDEN_DB) {
        finding("MED", "hidden-effect", format!(
            "curve is marked inactive for this mode but moves gain {gain_span:.1} dB / change {delta_span:.1} dB"));
    }

    // Live tweak: geometric ramp lowest→highest gain and back, one continuous instance.
    let gmin = settings.iter().map(|s| s.gain).fold(f32::MAX, f32::min).max(1e-3);
    let gmax = settings.iter().map(|s| s.gain).fold(f32::MIN, f32::max).max(gmin * 1.0001);
    let hops = stim.bins.len();
    let ramp = move |h: usize| -> f32 {
        let t = h as f32 / (hops - 1).max(1) as f32;
        let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
        gmin * (gmax / gmin).powf(tri)
    };
    let live = run_module(ty, mode, stim, &|h, cc| if cc == c { ramp(h) } else { 1.0 }, false);
    let static_peak = sums.iter().map(|s| s.peak_bin_gain_db).fold(f32::MIN, f32::max);
    let mut worst_motion = (f32::MIN, 0usize);
    let mut live_nonfinite_reported = false;
    let mut lrows = String::new();
    for (h, fr) in live.frames.iter().enumerate() {
        let s = summarise(std::slice::from_ref(fr));
        let _ = writeln!(lrows, "{mname},{modename},{c},{},{},{h},{:.5},{:.2},{:.2},{:.2},{:.1},{}",
            csv_esc(&label), stim.name, ramp(h), s.gain_db, s.delta_db, s.peak_bin_gain_db, s.spread_pct, s.nonfinite);
        if h >= measure_from && s.peak_bin_gain_db - static_peak > worst_motion.0 {
            worst_motion = (s.peak_bin_gain_db - static_peak, h);
        }
        if s.nonfinite > 0 && !live_nonfinite_reported {
            live_nonfinite_reported = true;
            finding("FATAL", "live-nonfinite", format!("NaN/Inf at hop {h} while sweeping (gain {:.3})", ramp(h)));
        }
    }
    if worst_motion.0 > MOTION_DB {
        finding("HIGH", "motion-spike", format!(
            "while sweeping, hop {} (gain {:.3}) reaches a bin gain {:.1} dB above any static setting",
            worst_motion.1, ramp(worst_motion.1), worst_motion.0));
    }
    out.live = lrows;
    out
}

// ── WAV ───────────────────────────────────────────────────────────────────

fn write_wav(path: &Path, frames: &[[Vec<Complex<f32>>; 2]]) {
    let mut planner = RealFftPlanner::<f32>::new();
    let inv = planner.plan_fft_inverse(FFT);
    let window: Vec<f32> = (0..FFT)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT - 1) as f32).cos()))
        .collect();
    let len = frames.len() * HOP + FFT;
    let mut pcm = [vec![0.0f32; len], vec![0.0f32; len]];
    let mut time = vec![0.0f32; FFT];
    for (h, fr) in frames.iter().enumerate() {
        for ch in 0..2 {
            let mut spec = fr[ch].clone();
            spec[0].im = 0.0;
            spec[NB - 1].im = 0.0;
            if inv.process(&mut spec, &mut time).is_err() { continue; }
            for i in 0..FFT { pcm[ch][h * HOP + i] += time[i] * window[i] * OLA_NORM; }
        }
    }
    let n = len as u32;
    let mut bytes = Vec::with_capacity(44 + len * 8);
    let data_len = n * 2 * 4;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&(SR as u32).to_le_bytes());
    bytes.extend_from_slice(&(SR as u32 * 8).to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..len {
        for ch in 0..2 { bytes.extend_from_slice(&pcm[ch][i].clamp(-4.0, 4.0).to_le_bytes()); }
    }
    let _ = std::fs::write(path, bytes);
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let threads: usize = arg("--threads").and_then(|v| v.parse().ok()).unwrap_or_else(|| {
        std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).clamp(1, 6)
    });
    let hops: usize = arg("--hops").and_then(|v| v.parse().ok()).unwrap_or(96);
    let only = arg("--only");
    let out_dir = PathBuf::from(arg("--out").unwrap_or_else(|| "target/bracket".into()));
    let wav_filter = arg("--wav");
    let measure_from = (hops / 6).max(4);
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let wav_dir = out_dir.join("wav");
    if wav_filter.is_some() { std::fs::create_dir_all(&wav_dir).expect("create wav dir"); }

    let t0 = std::time::Instant::now();
    let stims: Vec<Stimulus> = ["noise", "chord", "drums"].iter().map(|n| build_stimulus(n, hops)).collect();

    let mut jobs = Vec::new();
    for &ty in &MODULES {
        for mode in modes_for(ty) {
            let tag = format!("{:?}/{}", ty, mode.name());
            if let Some(o) = &only { if !tag.contains(o.as_str()) { continue; } }
            for curve in 0..module_spec(ty).num_curves {
                for stim in 0..stims.len() {
                    jobs.push(Job { ty, mode, curve, stim });
                }
            }
        }
    }
    eprintln!("bracket: {} jobs × {} settings + live sweep, {} hops, {} threads",
        jobs.len(), OFFSETS.len() + NODE_GAINS.len(), hops, threads);

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, JobResult)>> = Mutex::new(Vec::with_capacity(jobs.len()));
    let wav_arg = wav_filter.as_deref().map(|f| (f, wav_dir.as_path()));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= jobs.len() { break; }
                let r = run_job(&jobs[i], &stims, measure_from, wav_arg);
                let done = {
                    let mut g = results.lock().unwrap();
                    g.push((i, r));
                    g.len()
                };
                if done % 100 == 0 { eprintln!("  {done}/{} jobs", jobs.len()); }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);

    let mut settings = String::from("module,mode,curve,label,stimulus,kind,offset,gain,display,unit,gain_db,delta_db,peak_bin_gain_db,peak_out_db,spread_pct,spill_db,nonfinite\n");
    let mut live = String::from("module,mode,curve,label,stimulus,hop,gain,gain_db,delta_db,peak_bin_gain_db,spread_pct,nonfinite\n");
    let mut findings = String::from("severity,kind,module,mode,curve,label,stimulus,detail\n");
    let mut baselines = String::from("module,mode,stimulus,gain_db,delta_db,peak_bin_gain_db,peak_out_db,spread_pct,spill_db\n");
    let mut all_findings: Vec<String> = Vec::new();
    for (_, r) in &results {
        settings.push_str(&r.settings);
        live.push_str(&r.live);
        for f in &r.findings { findings.push_str(f); findings.push('\n'); all_findings.push(f.clone()); }
        if let Some((ty, mode, stim, s)) = &r.baseline {
            let _ = writeln!(baselines, "{ty:?},{mode},{stim},{:.2},{:.2},{:.2},{:.2},{:.1},{:.1}",
                s.gain_db, s.delta_db, s.peak_bin_gain_db, s.peak_out_db, s.spread_pct, s.spill_db);
            if s.gain_db.abs() > DEFAULT_GAIN_DB || s.spread_pct > DEFAULT_SPREAD_PCT {
                let f = format!("INFO,non-neutral-default,{ty:?},{mode},-,-,{stim},\"default curves change gain by {:.1} dB, {:.0}% of bins\"",
                    s.gain_db, s.spread_pct);
                findings.push_str(&f); findings.push('\n'); all_findings.push(f);
            }
        }
    }
    std::fs::write(out_dir.join("settings.csv"), settings).unwrap();
    std::fs::write(out_dir.join("live.csv"), live).unwrap();
    std::fs::write(out_dir.join("findings.csv"), findings).unwrap();
    std::fs::write(out_dir.join("baselines.csv"), baselines).unwrap();

    let count = |sev: &str| all_findings.iter().filter(|f| f.starts_with(sev)).count();
    eprintln!("done in {:.1} s → {}", t0.elapsed().as_secs_f32(), out_dir.display());
    eprintln!("findings: {} FATAL, {} HIGH, {} MED, {} LOW, {} INFO",
        count("FATAL"), count("HIGH"), count("MED"), count("LOW"), count("INFO"));
    for sev in ["FATAL", "HIGH"] {
        for f in all_findings.iter().filter(|f| f.starts_with(sev)).take(40) {
            eprintln!("  {f}");
        }
    }
}
