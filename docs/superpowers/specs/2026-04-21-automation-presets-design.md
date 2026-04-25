> **Status (2026-04-24): IMPLEMENTED.** Source of truth: the code + [../STATUS.md](../STATUS.md).

# Automation, Tooltips & Preset System — Design Spec

**Status:** Approved, ready for plan
**Date:** 2026-04-21

## Context

Two foundational features are needed before more module work lands:

1. **Host automation** — Currently only the global knobs (IN/OUT/MIX/ATK/REL/FREQ) are real `FloatParam`s. The main control surface — graph nodes, per-curve tilt/offset, FX matrix sends — is stored in `Mutex`-protected state with `#[persist]`, so hosts cannot see or automate it. This needs to become part of the automatable param grid.
2. **Presets** — There is no preset system. Users designing a slot/curve/matrix setup can only rely on DAW project state.

### Why static names (with tooltips)

nih-plug's CLAP wrapper only calls `CLAP_PARAM_RESCAN_VALUES`, not `RESCAN_INFO`/`RESCAN_ALL`, so param *names* cannot be dynamically renamed per slot content without forking nih-plug. The compromise: static names like `S2 C0 N3 Y` (slot 2, curve 0, node 3, y-field), with in-plugin tooltips after a 1000ms hover delay showing the human-readable label ("Attack — Node 3 Gain"). Dynamic renaming is deferred — see §2.

## Parameter count

| Surface | Count |
|---|---|
| Graph nodes: 6 × (x, y, q) × 7 curves × 9 slots | 1134 |
| Curve tilt + offset: 2 × 7 × 9 | 126 |
| FX matrix sends: 9 × 9 | 81 |
| Existing globals (IN, OUT, MIX, ATK, REL, FREQ, SC GAIN, FFT size, stereo link, etc.) | ~20 |
| **Total** | **~1361** |

VST3 `ParamID` is `uint32`; CLAP has no hard limit. Bitwig/Reaper/Ableton handle thousands of params without strain.

Q **is** automated. 1361 is within practical host limits, and Q sweeps are musically useful (resonance morphing under modulation).

**Not automated (by design):**
- Curve selector buttons (GUI navigation only)
- Module type selector (changing module type at automation rate would be nonsensical; still persisted)
- `graph_db_min`, `graph_db_max`, `peak_falloff_ms` (GUI view state)

## §1 — Param-ification

### Data migration

The fields currently stored via `Mutex<[[[CurveNode; 6]; 7]; 9]>`, `Mutex<[[(f32, f32); 7]; 9]>` (tilt/offset), and the `RouteMatrix` inside `Mutex` become `FloatParam`s.

Param ID scheme (stable forever, never change):
- Graph node: `s{slot}c{curve}n{node}{field}` where field ∈ `{x, y, q}`. Example: `s0c0n0x`.
- Tilt / offset: `s{slot}c{curve}tilt`, `s{slot}c{curve}offset`.
- Matrix cell: `mr{row}c{col}` (send from slot col → row).

Param ranges match existing semantics:
- `x` ∈ [0, 1] (log-freq 20 Hz → 20 kHz)
- `y` ∈ [-1, 1] (mapped to ±2 parameter headroom via v2 display; unchanged)
- `q` ∈ [0, 1] (log-bandwidth 4 oct → 0.1 oct)
- `tilt` ∈ [-1, 1] normalized (multiplied by `TILT_MAX = 2.0` internally)
- `offset` ∈ [-1, 1] normalized (multiplied by `curve_offset_max(display_idx)`)
- Matrix cell ∈ [0, 1] linear amplitude

### Code generation

1341 repetitive fields by hand is error-prone. Use a `build.rs` that emits `$OUT_DIR/params_gen.rs` containing the field declarations, `Params` trait impl entries, and a `GraphNodeParams` accessor API (`get_node(slot, curve, node) -> (&FloatParam, &FloatParam, &FloatParam)`). Include via `include!(concat!(env!("OUT_DIR"), "/params_gen.rs"))` inside `impl SpectralForgeParams`.

Avoid a proc-macro crate — `build.rs` is simpler and has no cross-crate linkage cost.

### Curve editor integration

`src/editor/curve.rs` currently reads/writes `CurveNode { x, y, q }` from a `Mutex`. After this change:

- **Read** (drawing): `FloatParam.value()` — non-blocking, lock-free.
- **Write** (drag): `setter.begin_set_parameter(&param)` / `setter.set_parameter(&param, v)` / `setter.end_set_parameter(&param)` — standard nih-plug automation-recording path. Begin/end is needed to group a drag as a single automation edit in Bitwig.

The per-block curve recomputation stays on the GUI thread. Whenever any node param changes (detected via dirty-flag comparing cached vs. current values), the GUI recomputes the curve gains for that slot+curve and publishes to the audio thread via the existing `curve_tx[slot][curve]` triple-buffer. Audio-side code is unchanged.

### State migration

Existing Bitwig projects have state saved from the `#[persist]` fields. The refactor keeps those `#[persist]` fields in parallel for one release:

1. On state load, if `#[persist]` nodes/matrix have non-default values AND params are still at defaults, copy persist → params (one-shot migration).
2. Set a `#[persist] migrated_v1: bool = false` flag to track that migration ran.
3. Next release (after users have saved their projects once), remove the legacy persist fields.

### Perf note

1361 `FloatParam`s each hold a `Smoother<f32>`. At typical 512-sample buffers this is microseconds, but benchmark before shipping. If hot, disable smoothing on the graph-node params (they drive a per-block recompute anyway, not per-sample).

## §2 — Deferred: dynamic param naming

Write a separate short note at `docs/superpowers/specs/2026-04-21-dynamic-automation-naming.md` describing the deferred work: fork or patch nih-plug to send `CLAP_PARAM_RESCAN_INFO` on module-type changes, and expose a `rename_param(id, new_name)` API. Goal: slots show up in the host as `"Freeze Length"` instead of `"S2 C2"`. The note captures the why, the approximate scope, and the blocker (nih-plug upstream).

## §3 — Tooltips

Every automatable widget gets a delayed tooltip with the human-readable label.

**Delay:** 1000ms after cursor settles on the widget.

**Content:**
- Graph node: `"S2 C0 N3 — Attack · Freq"` (slot + curve name + node index + field name, period-separated)
- Tilt / Offset: `"S2 C0 — Attack · Tilt"`
- FX matrix cell: `"S2 → S5 send — 45%"`
- Global knobs: existing nih-plug param display (unchanged)

**Implementation:** helper `delayed_tooltip(ui, response, text)` in `src/editor/mod.rs`. Track hover-start timestamp in `egui::Memory` keyed by widget `Id`. When `elapsed > 1000ms`, call `response.on_hover_text_at_pointer(text)`. Reset the timer on any pointer motion beyond a small deadband.

## §4 — Preset system

A preset = serialized snapshot of every automatable param + GUI-only state (curve selector index per slot, module types, stereo link, FFT size).

### On-disk format

JSON, one file per preset, `.sfpreset` extension.

### Directory

Platform-conventional via the `directories` crate:
- Linux: `~/.config/spectral-forge/presets/`
- Windows: `%APPDATA%\Spectral Forge\presets\`
- Mac: `~/Library/Application Support/Spectral Forge/presets/`

Harder to accidentally delete than a local `./presets/` folder. Created on first launch if missing.

### Schema

```json
{
  "schema_version": 1,
  "plugin_version": "0.3.0",
  "name": "Big Punchy Drum Comp",
  "params": {
    "in_gain": 0.5,
    "s0c0n0x": 0.0,
    "s0c0n0y": 0.0,
    "s0c0n0q": 0.5,
    "mr0c1": 1.0
  },
  "gui": {
    "editing_slot": 2,
    "editing_curve": 1,
    "slot_module_types": [0, 2, 1, 0, 0, 0, 0, 0, 255],
    "stereo_link": 0,
    "fft_size": 2048
  }
}
```

### Versioning

A manual `PRESET_SCHEMA_VERSION: u32 = 1` constant in `src/preset.rs`. Increment only when incompatible changes are made (param removed, range changed semantically, meaning shifted). Loader filters: if `schema_version != PRESET_SCHEMA_VERSION`, the preset is **hidden from the menu** (not shown as an error). `plugin_version` is informational only.

### UI

**Placement:** top bar, first control before the Floor (`graph_db_min`) setting.

Layout:
```
[◂ Preset ▾ "Big Drum Comp" ▸] [Save] [Open folder…]   [Floor -100 dB] [Ceiling 0 dB] ...
```

- Pulldown: scrollable list of compatible presets from the directory, alphabetical.
- **Load**: triggers on pulldown selection — replaces current state.
- **Save**: opens a small text-input popup for the preset name; writes `<sanitized_name>.sfpreset` to the preset directory. Overwrite-existing is confirmed via a second popup.
- **Open folder**: launches the platform file manager (`xdg-open` / `explorer` / `open`) on the preset directory.

No delete-from-UI (user deletes via file manager). No nested folders. No factory presets (deferred — could ship files into user dir on first run later). No category tags.

### Save/load mechanics

- **Save:** iterate `params.param_map()`, read each param's `normalized_value()`, write into JSON.
- **Load:** for each key in JSON `params`, look up the `ParamPtr` by ID via `params.param_map()`, call `setter.set_parameter_normalized(ptr, v)`. Unknown IDs are silently ignored (forward-compatible). GUI-only state deserialized separately.

## Files affected

| File | Change |
|---|---|
| `src/params.rs` | Include generated file, add `GraphNodeParams` accessor, keep existing globals |
| `build.rs` (new) | Emit `params_gen.rs` with 1341 `FloatParam` fields |
| `src/editor/curve.rs` | Read/write node x/y/q via `FloatParam` + `ParamSetter`; remove `Mutex<CurveNode>` reads |
| `src/editor_ui.rs` | Add preset pulldown to top bar; apply `delayed_tooltip` to all automatable widgets |
| `src/editor/mod.rs` | `delayed_tooltip()` helper; export preset widget |
| `src/editor/preset_menu.rs` (new) | Preset pulldown + save dialog |
| `src/editor/theme.rs` | Tooltip colours, preset pulldown colours |
| `src/preset.rs` (new) | `Preset` struct (serde), save/load/scan; `PRESET_SCHEMA_VERSION` |
| `src/dsp/pipeline.rs` | No change (curves still arrive via triple-buffer) |
| `src/dsp/modules/mod.rs` | `RouteMatrix` reads params instead of Mutex |
| `docs/superpowers/specs/2026-04-21-dynamic-automation-naming.md` (new) | Deferred-work note |
| `Cargo.toml` | Add `directories`, `serde_json`, `opener` |

## Testing

- **Unit** (`tests/preset.rs` new): round-trip save → load preserves every normalized param value.
- **Unit**: schema-version mismatch filters out the preset.
- **Unit**: filename sanitization (invalid chars replaced).
- **Integration** (`tests/migration.rs` new): state saved with old `#[persist]` path loads into new params with correct values.
- **Manual**: in Bitwig, automate `s0c0n0y` and confirm the node moves under automation; hover 1000ms and confirm tooltip text.

## Audio-rate modulation robustness

Bitwig (and some other CLAP/VST3 hosts) support audio-rate modulation: any exposed param may receive a sample-rate signal such as white noise. Every newly-exposed param must tolerate this without producing NaN/Inf or degenerate output.

Requirements:

- **All params `.with_smoother(SmoothingStyle::Linear(…))`** at a small ms value (1–5 ms). Smoothing converts audio-rate noise on the param into bandlimited motion per sample, preventing per-sample discontinuities in the derived curves.
- **Graph-node curve recomputation** happens once per buffer on the GUI thread today. After this change, curves need recomputation driven by params, not only by drag edits. Approach: the GUI thread polls `param.smoothed.next()` snapshots — but the **audio thread already receives smoothed curves via `curve_tx`**, so the user-facing rule is: if you want fast modulation to actually change the sound, route it to the tilt/offset or matrix-send params which are re-read every block, not to individual graph-node params. Graph nodes are latency-bounded by one block. Document this.
- **Divide-by-zero and range guards**: `q` at 0 maps to 4-oct bandwidth and must not evaluate as `1.0 / q`. `x` at log-freq endpoints must clamp to `[20 Hz, sample_rate / 2]` before use. `offset * curve_offset_max` is safe since both factors are bounded, but verify no module does `gain.powf(offset)` without a `max(1e-6)` on the base.
- **Matrix send at 1.0 exactly** must not create a self-feedback loop — `RouteMatrix` diagonal cells are send-to-self and must remain 0 regardless of automation. Guard by ignoring `mr{i}c{i}` writes in the param→matrix rebuild.
- **Finite-value guard on curve outputs**: the existing `dsp::guard::sanitize()` runs before FFT; keep that as the backstop for any modulation-induced NaN that slips through.

Testing: a `tests/audio_rate_modulation.rs` test feeds random noise into every automatable param for 1 second at 48 kHz, asserts the output is finite and within ±24 dBFS. Run as part of CI.

## Risks

- **Param count perf**: 1361 `FloatParam`s, each with a `Smoother`. Measure — if per-block smoother tick cost is significant, switch graph-node params to a simpler `Linear(1ms)` smoother or a custom cheap one.
- **State migration**: must not lose user work. The persist→params migration is the biggest risk surface — needs test coverage with real saved project state.
- **Automation lane cache** in hosts: ParamIDs must never change. Once this spec ships, the ID scheme is frozen.
- **Param-scan noise**: exposing 1361 params may make Bitwig's parameter-browse UX slow. User accepts this on the assumption that real workflows use Bitwig's Remote Controls pages / visual assignment, not flat param-list browsing. If that assumption is wrong we may need to revisit grouping — not blocking for this plan.
- **Audio-rate modulation edge cases**: covered in the section above.
