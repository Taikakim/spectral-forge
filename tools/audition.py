"""Audition every Spectral Forge module × mode through the built VST3 (pedalboard).

Runs the real plugin — pipeline, routing, curve and offset parameters — with one
module in slot 0 routed to Master. For each module × mode it processes:

- the fresh-assignment default (what you hear right after picking the module);
- each active curve's Offset knob at −1 and +1, everything else at default.

Then it checks the effect's documented claim (e.g. Phase Smear dissolves
transients, Freeze holds after the input stops, Exciter adds new partials) on
material chosen to show it, flags broken behaviour (NaN, > +12 dB, silence),
and renders listening files plus a local HTML page.

    python tools/audition.py --plugin target/bundled/spectral_forge.vst3/Contents/x86_64-win/spectral_forge.vst3
        [--specs target/specs.json] [--out target/audition] [--only TEXT] [--input my.wav]

`target/specs.json` comes from `cargo run --release --example bracket -- --dump-specs target/specs.json`.
Limitations: pedalboard feeds no sidechain bus and no host transport, so
sidechain-driven modes and tempo-synced Rhythm modes are only partly covered
here (`examples/bracket.rs` drives both offline).
"""
from __future__ import annotations

import argparse
import copy
import html
import json
import re
import struct
import time
import warnings
from pathlib import Path

import numpy as np
import pedalboard
import soundfile as sf

SR = 48_000
SECONDS = 6.0
N = int(SR * SECONDS)
WARMUP = int(0.25 * SR)

# ── JUCE VST3 state wrapper ──────────────────────────────────────────────────
# pedalboard's raw_state is JUCE's "VC2!" + size + XML; the plugin's own state
# (nih-plug JSON) sits in <IComponent> as "<len>.<JUCE base64>" (custom
# alphabet, bits packed least-significant first).

JUCE_B64 = ".ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+"
B64_IDX = {c: i for i, c in enumerate(JUCE_B64)}


def juce_b64_decode(s: str) -> bytes:
    size_str, body = s.split(".", 1)
    n = int(size_str)
    out = bytearray()
    acc = nbits = 0
    for c in body:
        acc |= B64_IDX[c] << nbits
        nbits += 6
        while nbits >= 8:
            out.append(acc & 0xFF)
            acc >>= 8
            nbits -= 8
    if nbits and len(out) < n:
        out.append(acc & 0xFF)
    return bytes(out[:n])


def juce_b64_encode(data: bytes) -> str:
    chars = []
    acc = nbits = 0
    for b in data:
        acc |= b << nbits
        nbits += 8
        while nbits >= 6:
            chars.append(JUCE_B64[acc & 0x3F])
            acc >>= 6
            nbits -= 6
    if nbits:
        chars.append(JUCE_B64[acc & 0x3F])
    return f"{len(data)}." + "".join(chars)


class Forge:
    """The plugin plus its decoded default state."""

    def __init__(self, path: str):
        self.plugin = pedalboard.load_plugin(path)
        raw = self.plugin.raw_state
        size = struct.unpack("<I", raw[4:8])[0]
        self.trailer = raw[8 + size:]
        self.xml = raw[8:8 + size].decode("utf-8")
        blob = re.search(r"<IComponent>(.*?)</IComponent>", self.xml, re.S).group(1)
        self.default_state = json.loads(juce_b64_decode(blob.strip()))
        assert juce_b64_encode(juce_b64_decode(blob.strip())) == blob.strip(), "JUCE base64 round trip"

    def _encode(self, state: dict) -> bytes:
        data = json.dumps(state, separators=(",", ":")).encode()
        xml = re.sub(r"<IComponent>.*?</IComponent>",
                     lambda _: f"<IComponent>{juce_b64_encode(data)}</IComponent>",
                     self.xml, flags=re.S)
        body = xml.encode("utf-8")
        return b"VC2!" + struct.pack("<I", len(body)) + body + self.trailer

    def run(self, state: dict, audio: np.ndarray) -> np.ndarray:
        self.plugin.raw_state = self._encode(state)
        return self.plugin.process(audio, SR, reset=True)


# ── Test patch ───────────────────────────────────────────────────────────────

def set_param(state: dict, pid: str, value: float) -> None:
    state["params"][pid] = {"f32": float(value)}


def set_field(state: dict, key: str, value) -> None:
    state["fields"][key] = json.dumps(value)


def patch(base: dict, spec: dict, offsets: dict[int, float] | None = None,
          route: str = "main", arp_grid: bool = False) -> dict:
    """Slot 0 = the module under test, slots 1–7 empty, slot 0 → Master.

    `route` = "main" (slot 0's output), "v0" / "v1" (its first / second split
    stream, matrix sources 9 / 10). Curves start flat with the Offset defaults
    the module popup applies on assignment.
    """
    st = copy.deepcopy(base)
    types = json.loads(st["fields"]["slot_module_types"])
    types[0] = spec["module"]
    for s in range(1, 8):
        types[s] = "Empty"
    set_field(st, "slot_module_types", types)
    if spec["mode_field"]:
        modes = json.loads(st["fields"][spec["mode_field"]])
        modes[0] = spec["mode"]
        set_field(st, spec["mode_field"], modes)
    if arp_grid:
        grids = json.loads(st["fields"]["slot_arp_grid"])
        grids[0] = {"steps": [1 << v for v in range(8)]}  # one voice per step
        set_field(st, "slot_arp_grid", grids)
    # Matrix params mr{dst}c{src}: clear everything, then route the chosen source.
    for key in list(st["params"]):
        if re.fullmatch(r"mr\d+c\d+", key):
            set_param(st, key, 0.0)
    src = {"main": 0, "v0": 9, "v1": 10}[route]
    set_param(st, f"mr8c{src}", 1.0)
    # Flat curves (all node gains 0) and assignment-default transforms.
    for c in range(7):
        for n in range(6):
            set_param(st, f"s0c{c}n{n}y", 0.0)
        set_param(st, f"s0c{c}tilt", 0.0)
        set_param(st, f"s0c{c}curv", 0.0)
        default = spec["curves"][c]["offset_default"] if c < len(spec["curves"]) else 0.0
        set_param(st, f"s0c{c}offset", default)
    for c, o in (offsets or {}).items():
        set_param(st, f"s0c{c}offset", o)
    return st


# ── Stimuli ──────────────────────────────────────────────────────────────────

def _rng(seed):
    return np.random.default_rng(seed)


def chord(n):
    t = np.arange(n) / SR
    out = np.zeros(n)
    for f0 in (110.0, 138.59, 164.81):
        for k in range(1, 21):
            if f0 * k > SR * 0.45:
                break
            out += np.sin(2 * np.pi * f0 * k * t) / k
    return out * 0.5 / np.abs(out).max()


def drums(n, seed=0x5EED):
    r = _rng(seed)
    beat = int(SR * 0.5)
    out = np.zeros(n)
    i = np.arange(n)
    pos = i % (beat * 4)
    b = pos // beat
    t = (pos % beat) / SR
    kick = (b == 0) | (b == 2)
    f = 50 + 90 * np.exp(-t * 30)
    out += np.where(kick, np.sin(2 * np.pi * f * t) * np.exp(-t * 9) * 0.8, 0)
    out += np.where(~kick, r.uniform(-1, 1, n) * np.exp(-t * 25) * 0.4, 0)
    th = (i % (beat // 2)) / SR
    out += r.uniform(-1, 1, n) * np.exp(-th * 120) * 0.15
    return out


def gate(x, start, stop, fade=0.01):
    t = np.arange(len(x)) / SR
    env = np.clip(np.minimum((t - start) / fade, (stop - t) / fade), 0, 1)
    return x * env


def tone(n):
    t = np.arange(n) / SR
    x = sum(a * np.sin(2 * np.pi * 220.0 * h * t) for h, a in ((1, 1.0), (2, 0.5), (3, 0.33)))
    return x * 0.5 / np.abs(x).max()


def build_stimuli():
    c = chord(N + 11)
    d = drums(N + 7)
    left = c[11:] * 0.6 + d[7:] * 0.6
    right = c[:N] * 0.48 + d[:N] * 0.54
    pad = gate(chord(N), 0.75, 3.75)
    tn = tone(N)
    return {
        "mix":   np.stack([left, right]).astype(np.float32),
        "pad":   np.stack([pad, pad * 0.9]).astype(np.float32),
        "drums": np.stack([d[7:], d[:N] * 0.9]).astype(np.float32),
        "tone":  np.stack([tn, tn]).astype(np.float32),
    }


# ── Metrics ──────────────────────────────────────────────────────────────────

def db(x):
    return 10 * np.log10(max(float(x), 1e-30))


def stft(x, n_fft, hop):
    w = np.hanning(n_fft).astype(np.float32)
    frames = np.lib.stride_tricks.sliding_window_view(x, n_fft)[::hop]
    return np.fft.rfft(frames * w, axis=-1)


def seg(x, a, b):
    return x[..., int(a * SR):int(b * SR)]


def env_db(x, block=480):
    m = (x[0] ** 2 + x[1] ** 2) / 2
    m = m[: len(m) // block * block].reshape(-1, block).mean(1)
    return 10 * np.log10(np.maximum(m, 1e-12))


def metrics(name: str, x: np.ndarray, y: np.ndarray) -> dict:
    """Compare input x and output y (both (2, N)) for one stimulus."""
    out = {}
    if not np.all(np.isfinite(y)):
        out["nonfinite"] = 1
        y = np.nan_to_num(y, nan=0.0, posinf=0.0, neginf=0.0)
    xs, ys = x[:, WARMUP:], y[:, WARMUP:]
    ex = float((xs ** 2).sum())
    out["level_db"] = db((ys ** 2).sum() / ex)
    out["change_db"] = db(((ys - xs) ** 2).sum() / ex)
    out["peak_db"] = 20 * np.log10(max(np.abs(ys).max(), 1e-9) / np.abs(xs).max())

    X = stft(xs[0], 2048, 512)
    Y = stft(ys[0], 2048, 512)
    ax, ay = np.abs(X), np.abs(Y)
    active = ax > ax.max(axis=1, keepdims=True) * 1e-3
    d = 20 * np.log10(np.maximum(ay, 1e-9) / np.maximum(ax, 1e-9))
    d = np.where(active, d, np.nan)
    resid = d - np.nanmedian(d, axis=1, keepdims=True)
    out["shape_db"] = float(np.nanmean(np.sqrt(np.nanmean(resid ** 2, axis=1))))
    out["spread_pct"] = float(np.nanmean(np.abs(d) > 1.0) * 100)
    out["mag_change_db"] = db(((ay - ax) ** 2).sum() / (ax ** 2).sum())

    if name == "mix":
        def width(z):
            m, s = (z[0] + z[1]) / 2, (z[0] - z[1]) / 2
            return db((s ** 2).sum() / max((m ** 2).sum(), 1e-30))
        out["width_db"] = width(ys) - width(xs)
        out["pan_db"] = (db((ys[0] ** 2).sum() / max((ys[1] ** 2).sum(), 1e-30))
                         - db((xs[0] ** 2).sum() / max((xs[1] ** 2).sum(), 1e-30)))
    if name == "pad":
        on_x, on_y = seg(x, 1.0, 3.7), seg(y, 1.0, 3.7)
        e_on = max(float((on_y ** 2).sum()) / on_y.shape[-1], 1e-30)
        tail = seg(y, 3.95, 5.5)
        pre = seg(y, 0.2, 0.7)
        out["tail_db"] = db((tail ** 2).sum() / tail.shape[-1] / e_on)
        out["pre_db"] = db((pre ** 2).sum() / pre.shape[-1] / e_on)
        out["trem_db"] = float(np.std(env_db(on_y)) - np.std(env_db(on_x)))
        Xo, Yo = np.abs(stft(on_x[0], 2048, 512)), np.abs(stft(on_y[0], 2048, 512))
        act = Xo > Xo.max(axis=1, keepdims=True) * 1e-2
        g = 20 * np.log10(np.maximum(Yo, 1e-9) / np.maximum(Xo, 1e-9))
        g = np.where(act, g, np.nan)
        out["jitter_db"] = float(np.nanmedian(np.nanstd(g, axis=0)))
        lx = np.where(act, 20 * np.log10(np.maximum(Xo, 1e-9)), np.nan)
        ly = np.where(act, 20 * np.log10(np.maximum(Yo, 1e-9)), np.nan)
        out["peaky_db"] = float(np.nanmean(np.nanstd(ly, axis=1)) - np.nanmean(np.nanstd(lx, axis=1)))
    if name == "drums":
        ex_, ey_ = env_db(xs), env_db(ys)
        crest = lambda e: float(np.percentile(e, 99) - np.median(e))
        out["crest_db"] = crest(ey_) - crest(ex_)
    if name == "tone":
        Xt = np.abs(stft(xs[0], 8192, 2048)) ** 2
        Yt = np.abs(stft(ys[0], 8192, 2048)) ** 2
        fresh = Xt < Xt.max(axis=1, keepdims=True) * 1e-8  # > 80 dB below the frame peak
        tot = max(float(Yt.sum()), 1e-30)
        out["new_db"] = db((Yt * fresh).sum() / tot)
        lo = int(160 / (SR / 8192))
        out["below_db"] = db(Yt[:, :lo].sum() / tot)
    return out


# ── Claims (from the help text) ──────────────────────────────────────────────
# (stimulus, metric, op, threshold, claim). `None` = nothing audible expected.

SC = "needs a sidechain (not available through pedalboard; covered by examples/bracket.rs)"
CLAIMS = {
    "Dynamics/-": ("drums", "crest_db", "<", -1.0, "compresses: transient peaks come down"),
    "Freeze/-": ("pad", "tail_db", ">", -30.0, "holds the spectrum after the input stops"),
    "PhaseSmear/-": ("drums", "crest_db", "<", -2.0, "dissolves transients into smear"),
    "Contrast/Spatial": ("pad", "peaky_db", ">", 1.0, "sharpens spectral peaks, deepens valleys"),
    "Contrast/Temporal": ("drums", "crest_db", ">", 1.0, "expands changes over time"),
    "Contrast/Tilt": ("mix", "shape_db", ">", 1.0, "tilts spectral balance"),
    "Gain/Add": ("mix", "level_db", "abs>", 3.0, "GAIN curve changes level"),
    "Gain/Subtract": ("mix", "level_db", "abs>", 3.0, "GAIN curve carves level"),
    "Gain/Pull": ("mix", "shape_db", ">", 1.0, "pulls the spectrum toward the curve"),
    "Gain/Match": ("mix", "shape_db", ">", 1.0, "shapes input toward the curve"),
    "MidSide/-": ("mix", "width_or_pan", ">", 3.0, "changes stereo width / pan"),
    "TransientSustainedSplit/-": ("drums", "split_crest", ">", 3.0, "T stream punchier than S stream"),
    "Harmonic/HpSplit": ("mix", "split_crest", "<", -3.0, "P stream punchier than H stream"),
    "Harmonic/PartialEq": ("tone", "shape_db", ">", 1.0, "re-weights partials"),
    "Harmonic/Exciter": ("tone", "new_db", ">", -40.0, "adds harmonics that were not there"),
    "Future/PrintThrough": ("pad", "pre_db", ">", -50.0, "faint copy before the sound"),
    "Future/PreEcho": ("pad", "pre_db", ">", -50.0, "echo before the sound"),
    "Punch/Direct": None, "Punch/Inverse": None,
    "Punch/SelfPunch": ("pad", "shape_db", ">", 1.0, "carves at its own peaks, fills neighbours"),
    "Rhythm/Euclidean": ("pad", "trem_db", ">", 3.0, "rhythmic gating (needs host tempo)"),
    "Rhythm/Arpeggiator": ("pad", "trem_db", ">", 3.0, "step-sequenced voice gating (needs host tempo)"),
    "Rhythm/PhaseReset": ("pad", "change_db", ">", -15.0, "phase snaps at step edges (needs host tempo)"),
    "Geometry/Chladni": ("mix", "shape_db", ">", 1.5, "carves along nodal-line frequencies"),
    "Geometry/Helmholtz": ("mix", "shape_db", ">", 1.0, "resonant cavity band"),
    "Modulate/PhasePhaser": ("pad", "change_db", ">", -15.0, "rotates phase over time"),
    "Modulate/BinSwapper": ("pad", "shape_db", ">", 1.0, "swaps energy between bins"),
    "Modulate/RmFmMatrix": None, "Modulate/DiodeRm": None,
    "Modulate/GroundLoop": ("tone", "below_db", ">", -50.0, "injects mains hum"),
    "Modulate/GravityPhaser": ("pad", "change_db", ">", -15.0, "pulls phases toward a well"),
    "Modulate/PllTear": ("pad", "change_db", ">", -15.0, "lock-loss artefacts"),
    "Modulate/FmNetwork": ("tone", "new_db", ">", -45.0, "FM sidebands between partials"),
    "Modulate/SlewLag": ("pad", "tail_db", ">", -40.0, "level changes slowly: releases hang"),
    "Circuit/BbdBins": ("pad", "change_db", ">", -15.0, "bucket-brigade smoothing + dither"),
    "Circuit/SpectralSchmitt": ("mix", "shape_db", ">", 1.0, "latches quiet bins down"),
    "Circuit/CrossoverDistortion": ("mix", "shape_db", ">", 1.0, "silences bins below a deadzone"),
    "Circuit/Vactrol": ("pad", "change_db", ">", -15.0, "opto-coupler soft saturation"),
    "Circuit/TransformerSaturation": ("pad", "peaky_db", "<", -1.0, "saturates loud bins"),
    "Circuit/PowerSag": ("mix", "level_db", "<", -1.0, "loud material sags the level"),
    "Circuit/ComponentDrift": ("pad", "jitter_db", ">", 0.5, "random per-bin gain wander"),
    "Circuit/PcbCrosstalk": ("pad", "peaky_db", "<", -0.5, "leaks energy into neighbours"),
    "Circuit/SlewDistortion": ("drums", "crest_db", "<", -1.0, "slows transients"),
    "Circuit/BiasFuzz": ("mix", "shape_db", ">", 1.0, "asymmetric clipping"),
    "Life/Viscosity": ("pad", "peaky_db", "<", -1.0, "smooths across bins"),
    "Life/SurfaceTension": ("pad", "peaky_db", ">", 1.0, "peaks absorb neighbours"),
    "Life/Crystallization": ("pad", "change_db", ">", -15.0, "phase-locks sustained bins"),
    "Life/Archimedes": ("mix", "level_db", "<", -1.0, "ducks when total level exceeds capacity"),
    "Life/NonNewtonian": ("drums", "crest_db", "<", -1.0, "clamps fast changes"),
    "Life/Stiction": ("pad", "change_db", ">", -15.0, "holds slow-moving bins"),
    "Life/Yield": ("mix", "change_db", ">", -15.0, "tears loud bins"),
    "Life/Capillary": ("tone", "new_db", ">", -50.0, "wicks energy to harmonics above"),
    "Life/Sandpaper": ("tone", "new_db", ">", -50.0, "sparks of noise above"),
    "Life/Brownian": ("pad", "jitter_db", ">", 0.5, "random-walk magnitude jitter"),
    "Past/Granular": ("pad", "change_db", ">", -10.0, "replays grains from history"),
    "Past/DecaySorter": ("pad", "change_db", ">", -10.0, "re-orders bins by a history feature"),
    "Past/Convolution": ("drums", "crest_db", "<", -1.0, "convolves with history (smears)"),
    "Past/Reverse": ("drums", "change_db", ">", -10.0, "plays history backwards"),
    "Past/Stretch": ("pad", "change_db", ">", -10.0, "time-stretches history"),
    "Kinetics/Hooke": ("pad", "shape_db", ">", 1.0, "springs couple neighbouring bins"),
    "Kinetics/GravityWell": ("pad", "shape_db", ">", 1.0, "bins fall toward a well"),
    "Kinetics/InertialMass": ("pad", "none", "", 0.0, "writes mass for later slots only (no sound alone)"),
    "Kinetics/OrbitalPhase": ("pad", "change_db", ">", -15.0, "rotates satellite phases"),
    "Kinetics/Ferromagnetism": ("pad", "change_db", ">", -15.0, "aligns neighbour phases to peaks"),
    "Kinetics/ThermalExpansion": ("pad", "change_db", ">", -15.0, "loud bins heat up and detune"),
    "Kinetics/TuningFork": ("pad", "shape_db", ">", 1.0, "sympathetic resonance near peaks"),
    "Kinetics/Diamagnet": ("pad", "shape_db", ">", 1.0, "carves and redistributes energy"),
    "Harmony/Chordification": ("mix", "shape_db", ">", 1.0, "snaps toward a chord"),
    "Harmony/Undertone": ("tone", "below_db", ">", -50.0, "adds sub-octave partials"),
    "Harmony/Companding": ("tone", "shape_db", ">", 1.0, "attenuates overtones vs fundamentals"),
    "Harmony/FormantRotation": ("tone", "new_db", ">", -50.0, "shifts harmonics"),
    "Harmony/Lifter": ("mix", "shape_db", ">", 1.0, "cepstral envelope/pitch shaping"),
    "Harmony/Inharmonic": ("tone", "new_db", ">", -50.0, "detunes partials off the series"),
    "Harmony/HarmonicGenerator": ("tone", "new_db", ">", -50.0, "synthesises extra harmonics"),
    "Harmony/Shuffler": ("pad", "jitter_db", ">", 1.0, "randomly swaps bins each hop"),
}


def claim_value(metric: str, m: dict, extra: dict) -> float | None:
    if metric == "width_or_pan":
        return max(abs(m.get("width_db", 0)), abs(m.get("pan_db", 0)))
    if metric == "split_crest":
        return extra.get("split_crest")
    return m.get(metric)


def meets(op: str, v: float | None, thr: float) -> bool:
    if v is None or not np.isfinite(v):
        return False
    return {"<": v < thr, ">": v > thr, "abs>": abs(v) > thr}[op]


def strength(op: str, v: float | None) -> float:
    if v is None or not np.isfinite(v):
        return -1e9
    return {"<": -v, ">": v, "abs>": abs(v)}[op]


# ── Driver ───────────────────────────────────────────────────────────────────

def main():
    warnings.filterwarnings("ignore", category=RuntimeWarning)
    ap = argparse.ArgumentParser()
    ap.add_argument("--plugin", required=True)
    ap.add_argument("--specs", default="target/specs.json")
    ap.add_argument("--out", default="target/audition")
    ap.add_argument("--only")
    ap.add_argument("--input", help="your own WAV for the listening renders")
    args = ap.parse_args()

    out = Path(args.out)
    (out / "wav").mkdir(parents=True, exist_ok=True)
    specs = json.loads(Path(args.specs).read_text())
    forge = Forge(args.plugin)
    stims = build_stimuli()
    listen = stims["mix"]
    if args.input:
        a, sr = sf.read(args.input, dtype="float32", always_2d=True)
        assert sr == SR, f"--input must be {SR} Hz"
        a = a.T if a.shape[1] > 1 else np.repeat(a.T, 2, 0)
        listen = a[:2, :N] if a.shape[1] >= N else np.pad(a[:2], ((0, 0), (0, N - a.shape[1])))
    for k, v in stims.items():
        sf.write(out / "wav" / f"_dry_{k}.wav", v.T, SR, subtype="PCM_16")
    sf.write(out / "wav" / "_dry_listen.wav", listen.T, SR, subtype="PCM_16")

    # Latency check: an all-Empty patch must give back the input.
    empty = patch(forge.default_state, {"module": "Empty", "mode_field": None, "mode": "-", "curves": []})
    y = forge.run(empty, stims["mix"])
    null = metrics("mix", stims["mix"], y)
    print(f"empty-slot null test: level {null['level_db']:+.2f} dB, residual {null['change_db']:.1f} dB")

    rows = []
    t0 = time.time()
    for spec in specs:
        key = f"{spec['module']}/{spec['mode']}"
        if args.only and args.only not in key:
            continue
        claim = CLAIMS.get(key, ("mix", "shape_db", ">", 1.0, "audible change"))
        variants = [("", False)]
        if key == "Rhythm/Arpeggiator":
            variants.append((" +grid", True))
        for suffix, arp in variants:
            rows.append(audition(forge, spec, key + suffix, claim, stims, listen, out, arp))
            r = rows[-1]
            print(f"[{time.time() - t0:6.0f}s] {r['key']:40s} {r['verdict']:10s} {r['note']}")
    (out / "results.json").write_text(json.dumps(rows, indent=1, default=float))
    write_html(out, rows, null)
    print(f"done -> {out / 'index.html'}")


def settings_for(spec):
    sets = [("default", {})]
    for c in spec["curves"]:
        if not c["active"]:
            continue
        for o in (-1.0, 1.0):
            if o != c["offset_default"]:
                sets.append((f"{c['label']} offset {o:+.0f}", {c["index"]: o}))
    return sets


def display_of(spec, offsets):
    parts = []
    for ci, o in offsets.items():
        c = spec["curves"][ci]
        d = next((x["display"] for x in c["offsets"] if x["offset"] == o), None)
        parts.append(f"{c['label']} = {d:.3g} {c['unit']}".strip() if d is not None else c["label"])
    return ", ".join(parts)


def audition(forge, spec, key, claim, stims, listen, out, arp):
    res = {"key": key, "module": spec["module"], "mode": spec["mode"],
           "overview": spec.get("overview") or "", "settings": []}
    if claim is None:
        res.update(verdict="SKIP", note=SC, claim=SC)
        stim, metric, op, thr, text = "mix", "shape_db", ">", 1.0, "audible change"
    else:
        stim, metric, op, thr, text = claim
        res["claim"] = text
    best = None
    flags = set()
    for label, offs in settings_for(spec):
        st = patch(forge.default_state, spec, offs, arp_grid=arp)
        m = {"mix": metrics("mix", stims["mix"], forge.run(st, stims["mix"]))}
        if stim != "mix":
            m[stim] = metrics(stim, stims[stim], forge.run(st, stims[stim]))
        extra = {}
        if metric == "split_crest":
            crest = []
            for route in ("v0", "v1"):
                sv = patch(forge.default_state, spec, offs, route=route)
                crest.append(metrics("drums", stims["drums"], forge.run(sv, stims["drums"]))["crest_db"])
            extra["split_crest"] = crest[0] - crest[1]
        v = claim_value(metric, m[stim], extra) if metric != "none" else None
        mm = m["mix"]
        if any(x.get("nonfinite") for x in m.values()):
            flags.add("NaN/Inf")
        if mm["level_db"] > 12 or mm["peak_db"] > 12:
            flags.add(f"loud ({label}: {mm['level_db']:+.0f} dB)")
        if label == "default" and mm["level_db"] < -40:
            flags.add("silent at default")
        s = {"label": label, "display": display_of(spec, offs), "offsets": offs,
             "claim_value": v, "mix": mm, stim: m[stim]}
        res["settings"].append(s)
        if best is None or strength(op, v) > strength(op, best["claim_value"]):
            best = s
    dflt = res["settings"][0]
    res["default_audible"] = dflt["mix"]["change_db"] > -20
    res["best"] = best["label"]
    res["flags"] = sorted(flags)
    if res.get("verdict") != "SKIP":
        if metric == "none":
            loud = max(s["mix"]["change_db"] for s in res["settings"])
            res["verdict"] = "OK" if loud < -40 else "CHECK"
            res["note"] = f"max change {loud:.0f} dB (expected silent alone)"
        else:
            ok = meets(op, best["claim_value"], thr)
            # Audibly changes the signal, just not in the way the claim metric measures.
            most = max((s["mix"]["change_db"] for s in res["settings"] if s["mix"]["level_db"] > -40),
                       default=-300.0)
            res["verdict"] = "OK" if ok else ("OTHER EFFECT" if most > -20 else "NO EFFECT")
            bv = best["claim_value"]
            res["note"] = (f"{metric} {bv:+.1f} at '{best['label']}' (needs {op}{thr:g})"
                           if bv is not None else f"{metric} unavailable")
            if not ok and most > -20:
                res["note"] += f"; but changes the mix by up to {most:.0f} dB"
        if flags:
            res["verdict"] = "BROKEN" if ("NaN/Inf" in flags or "silent at default" in flags) else res["verdict"] + "*"
    # Renders: default and the claim's strongest setting, on the listening material.
    safe = key.replace("/", "_").replace(" ", "").replace("+", "")
    for tag, lbl in (("default", "default"), ("best", best["label"])):
        offs = next(s["offsets"] for s in res["settings"] if s["label"] == lbl)
        y = forge.run(patch(forge.default_state, spec, offs, arp_grid=arp), listen)
        y = np.nan_to_num(y)
        clip = int((np.abs(y) > 1).sum())
        sf.write(out / "wav" / f"{safe}_{tag}.wav", np.clip(y, -1, 1).T, SR, subtype="PCM_16")
        res[f"wav_{tag}"] = f"wav/{safe}_{tag}.wav"
        res[f"clip_{tag}"] = clip
        if tag == "best" and stim != "mix":
            y2 = np.nan_to_num(forge.run(patch(forge.default_state, spec, offs, arp_grid=arp), stims[stim]))
            sf.write(out / "wav" / f"{safe}_best_{stim}.wav", np.clip(y2, -1, 1).T, SR, subtype="PCM_16")
            res["wav_best_stim"] = f"wav/{safe}_best_{stim}.wav"
            res["stim"] = stim
    return res


# ── HTML ─────────────────────────────────────────────────────────────────────

def write_html(out: Path, rows: list, null: dict):
    e = html.escape
    colors = {"OK": "#2e7d32", "NO EFFECT": "#c62828", "BROKEN": "#b71c1c", "SKIP": "#757575", "CHECK": "#ef6c00",
              "OTHER EFFECT": "#1565c0"}
    counts = {}
    for r in rows:
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
    body = []
    for r in rows:
        col = colors.get(r["verdict"].rstrip("*"), "#ef6c00")
        d = r["settings"][0]["mix"]
        set_rows = ""
        for s in r["settings"]:
            cv = "" if s["claim_value"] is None else f"{s['claim_value']:+.2f}"
            set_rows += (f"<tr><td>{e(s['label'])}</td><td>{e(s['display'])}</td>"
                         f"<td>{s['mix']['level_db']:+.1f}</td><td>{s['mix']['change_db']:.1f}</td>"
                         f"<td>{s['mix']['spread_pct']:.0f}%</td><td>{cv}</td></tr>")
        audio = (f"<div class=a><span>default</span><audio controls preload=none src='{r['wav_default']}'></audio></div>"
                 f"<div class=a><span>strongest ({e(r['best'])})</span><audio controls preload=none src='{r['wav_best']}'></audio></div>")
        if r.get("wav_best_stim"):
            audio += (f"<div class=a><span>strongest on {r['stim']}</span>"
                      f"<audio controls preload=none src='{r['wav_best_stim']}'></audio></div>")
        body.append(f"""
<section>
 <h3>{e(r['key'])} <b style="background:{col}">{e(r['verdict'])}</b></h3>
 <p class=o>{e(r['overview'])}</p>
 <p><i>Claim:</i> {e(r.get('claim', ''))}. <i>Result:</i> {e(r['note'])}
 {'<br><i>Flags:</i> ' + e(', '.join(r['flags'])) if r['flags'] else ''}
 <br><i>Default:</i> level {d['level_db']:+.1f} dB, change {d['change_db']:.1f} dB, {d['spread_pct']:.0f}% of bins
 {'(audible)' if r['default_audible'] else '(inaudible)'}; clipped samples {r['clip_default']} / {r['clip_best']}</p>
 {audio}
 <details><summary>all settings</summary><table><tr><th>setting</th><th>display</th><th>level dB</th>
 <th>change dB</th><th>spread</th><th>claim metric</th></tr>{set_rows}</table></details>
</section>""")
    summary = ", ".join(f"{v} {k}" for k, v in sorted(counts.items()))
    page = f"""<!doctype html><html lang=en><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1"><title>Spectral Forge audition</title>
<style>
body{{font:14px/1.45 system-ui,sans-serif;margin:0 auto;max-width:980px;padding:16px;background:#fafafa;color:#222}}
section{{background:#fff;border:1px solid #ddd;border-radius:6px;padding:10px 14px;margin:10px 0}}
h3{{margin:2px 0}} h3 b{{color:#fff;font-size:12px;padding:2px 6px;border-radius:3px;margin-left:6px}}
.o{{color:#555;margin:4px 0}} .a{{display:inline-flex;flex-direction:column;margin:4px 10px 4px 0;font-size:12px}}
audio{{height:30px;max-width:300px}} table{{border-collapse:collapse;font-size:12px}} td,th{{border-bottom:1px solid #eee;padding:2px 8px;text-align:left}}
</style></head><body>
<h1>Spectral Forge audition</h1>
<p>Real VST3 through pedalboard: one module in slot 0 → Master, fresh-assignment defaults, then each active
curve's Offset at −1 / +1. Empty-slot null test: level {null['level_db']:+.2f} dB, residual {null['change_db']:.1f} dB.
Verdicts: {e(summary)}. No sidechain or host tempo is available here.</p>
<div class=a><span>dry listening material</span><audio controls preload=none src='wav/_dry_listen.wav'></audio></div>
<div class=a><span>dry pad</span><audio controls preload=none src='wav/_dry_pad.wav'></audio></div>
<div class=a><span>dry drums</span><audio controls preload=none src='wav/_dry_drums.wav'></audio></div>
<div class=a><span>dry tone</span><audio controls preload=none src='wav/_dry_tone.wav'></audio></div>
{''.join(body)}
</body></html>"""
    (out / "index.html").write_text(page, encoding="utf-8")


if __name__ == "__main__":
    main()
