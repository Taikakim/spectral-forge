# Research Synthesis — 2026-04-26

**Purpose:** distill the six research-agent reports under `research/`
into the cross-cutting decisions, the dead ends, and the suggested
roadmap deltas. Each per-module audit file already carries its own
"Research findings (2026-04-26)" section with the module-local
takeaways; this file is the layer above that.

**Inputs:** `research/01-pvx-phase-and-pll.md` through
`research/06-specialized-topics.md`. Each report consolidated 1–4
prompts from `90-research-prompts.md` into an arxiv + GitHub +
literature digest.

**Status:** RESEARCH synthesis — not approved. Should inform the
next brainstorming + planning sessions, not pre-empt them.

---

## The single most important finding

**There are five shared infrastructure pieces that, if landed once,
unblock most of the deferred modules at near-zero per-module cost.**
They are individually small (each ≤ ~200 LOC of Rust), they share
the audio thread's existing budget, and they collapse what would
otherwise be N copies of the same per-bin loop into one.

| # | Shared piece | Owners (modules that consume it) | Source |
|---|---|---|---|
| 1 | **Unwrapped phase + peak set** in `ModuleContext` (the LDR/Roebel pass at Pipeline level) | Dynamics, PhaseSmear, Freeze, Modulate (PLL Tear), Past Stretch, Harmony (pitch), Kinetics Orbital | `research/01-pvx-phase-and-pll.md` cross-topic synthesis; `20-plpv-phase-cross-cutting.md` |
| 2 | **`if_offset[k]` cache** in `ModuleContext` (per-bin instantaneous-frequency offset, computed once after the analysis FFT) | Past Stretch, Future Predict, Punch fill, Modulate PLL Tear, Harmony Inharmonic | `05-time-manipulation.md` cross-topic synthesis |
| 3 | **`PhaseRotator` helper** (LUT-interpolated complex multiply for `Δφ = 2π · Δf · Δt / N` rotations) | Past Stretch, Future Predict, Punch fill, Modulate PLL Tear, Harmony Inharmonic | `05-time-manipulation.md` cross-topic synthesis |
| 4 | **`EnvelopeBank` SIMD primitive** (per-bin temporal smoothing indexed by sidechain or peak events) | Modulate Buchla envelopes, Punch healing curve, Kinetics Orbital phase-rate smoothing | `06-specialized-topics.md` cross-topic synthesis |
| 5 | **`circuit_kernels.rs` SoA helpers** (`tanh_levien_simd`, `lp_step_simd`, `spread_3tap_simd`, `slew_clip_simd`, `SimdRng`) on the `wide::f32x8` substrate | Circuit (all 8 sub-effects), Modulate Buchla, Life Capillary diffusion (potential reuse) | `04-simd-analog.md` cross-component synthesis |

Pieces 1+2+3 originate from the same hop — the unwrap pass already
yields per-bin IF, and the IF feeds the rotator. Built as one PR they
cost ~0.3 % of one core and unblock ≥ 6 modules each.

Piece 4 (`EnvelopeBank`) absorbs the Buchla two-cascade vactrol
model, the Punch healing curve, and the Orbital phase-rate smoother
into a single SIMD-vectorised inner loop; saving the per-module
duplication is the real win, not the SIMD speedup.

Piece 5 is the analog-character substrate. The `wide` crate is the
chosen SIMD library across all reports (stable Rust, MIT-licensed,
v1 path). `pulp` is reserved for v2 if runtime CPU dispatch becomes
necessary.

---

## Cross-cutting validated paths

These are the technical decisions the research validated. Each
points at the per-module audit file where the detail lives.

| Topic | Validated path | Detail |
|---|---|---|
| **Phase vocoder math** | Laroche-Dolson 1999 (peak-locking math) + Laroche-Dolson 1997 "About this Phasiness Business" (motivation / what to listen for) + Roebel 2003 transient COG. Drop the "PVX" naming — the technique is **PLPV (Peak-Locked Phase Vocoder) / LDR**. ProSoniq's PVX paper repackages the same math with worse documentation. | `20-plpv-phase-cross-cutting.md` |
| **Per-bin PLL** | 2nd-order PI loop per bin, `ωₙ ≈ 0.05 cyc/hop`, `ζ = 0.707`, hysteresis on lose-lock / re-lock thresholds. Track only peak bins via `track_mask` from shared peak set — cuts workload from 8193 to ~100 bins. | `16-modulate.md` |
| **Pitch/chord** | Classical IF + Klapuri-style harmonic summation + IF-refined HPCP chromagram + 60-template chord matcher. Defer all neural pitch trackers (CREPE, PESTO, BasicPitch, RMVPE, FCPE, SwiftF0) — opt-in only for monophonic edge cases. | `15-harmony.md` |
| **Cepstral envelope** | Naive cepstral liftering as default (cheap), promote Roebel True Envelope or WORLD CheapTrick to default if the bias is audible on Kim's typical material. | `15-harmony.md` |
| **Springs** | Velocity Verlet (Stoermer-Verlet) integrator, CFL bound `ω·dt < 2`, mandatory 1-pole curve smoothing to suppress Mathieu parametric instability, sparse harmonic links cap=8. | `12-kinetics.md` |
| **Diffusion** | FTCS finite-volume with harmonic-mean face flux, `D[k] ∈ [0, 0.45]` clamp, no Lattice-Boltzmann (sub-threshold quality difference, much higher cost). | `11-life.md` |
| **2D wave** | 128×64 Hilbert-mapped grid, leapfrog, `c ≤ 0.65` CFL ceiling, Mur first-order ABC boundary (PML overkill at this grid size), 16 KB Hilbert LUT shared. | `18-geometry.md` |
| **Orbital phase** | Linear `Δφ = α · S_K / d²` rotation per satellite, no Kepler orbits — the linear approximation is indistinguishable at the sub-bin rotation magnitudes we use. | `12-kinetics.md`, `06-specialized-topics.md` |
| **Future / Predict** | Linear-in-dB magnitude prediction + flux-and-IF-variance confidence gating + reused phase. Roughly 80 LOC, < 1 % of one core. | `14-future.md` |
| **Past Stretch** | In-house kernel using Puckette/lamination phase coherence (not full LDR rigid lock) for v1; LDR upgrade is a one-PR follow-up because the peak set is already shared infra. **No** Rubber Band, **no** Signalsmith — both pull licensing/architectural problems for marginal quality gain. | `13-past.md` |
| **Punch** | Amplitude fill default, exponential τ=150 ms healing curve, slow-drift-to-zero release, defer watch-range curve to v2. | `19-punch.md` |
| **Buchla envelope** | Two cascaded 1-pole filters (vactrol model from Parker 2013 LPG paper); ERB-band trigger detector; SIMD over `wide::f32x8` lanes via `EnvelopeBank`. | `16-modulate.md` |
| **Geometry persistence** | 1D peak-persistence (sthu O(n log n)) for v1 — could even replace the MQ peak picker in Harmony. Full 2D persistent homology deferred to v2 with worker-thread DSP. | `18-geometry.md` |

---

## Dead ends (do not pursue without new evidence)

Each came up in research; each was actively rejected. Documenting so
we don't re-explore them.

- **PGHI / RTPGHI** for ducking. It's a *spectrogram inversion*
  algorithm — irrelevant for the per-bin processing we do. The 1-hop
  variant is interesting for Freeze re-synthesis only.
- **Janssen 2.0** time-frequency AR audio inpainting. ~200× slower
  than real-time. The whole "fancy AR-in-STFT" class is offline-only.
- **Yule-Walker AR fitting.** Wharton tech note flags it as actively
  dangerous (poor conditioning, unstable poles for short windows).
  Use Burg if AR is ever needed.
- **Cubical Ripser at native resolution.** Too slow for hop-rate
  persistent homology. Worker-thread + downsampled grid is the only
  feasible path; ship in v2.
- **Neural pitch trackers as primary path.** CREPE / PESTO /
  BasicPitch / RMVPE / FCPE / SwiftF0 all duplicate work classical
  IF + Klapuri already does, are mostly monophonic, and add
  100 KB–100 MB of model data. Opt-in only.
- **Neural prediction for Future module.** No published codec is
  built for "frame N+1 from N" semantics; CNN inference at 8193 bins
  burns >30 % of a core.
- **AR-Burg per-bin for Future v1.** ~3 % CPU and added complexity
  for a quality gain users may not even hear. Defer.
- **Persistent Homology in Geometry v1.** See above; defer to v2.
- **Watch-range curve in Punch v1.** Adds UI complexity; ship the
  basic healing curve first.
- **Self-Punch.** Per `19-punch.md` audit.
- **Lookahead Duck in Future v1.** Pipeline reorder is too intrusive
  for the marginal benefit.
- **Crystal Ball as a sub-effect.** Better as a future `lookahead:
  bool` ModuleSpec flag than a Future sub-mode.
- **Volterra series for analog kernels.** Too expensive at hop rate.
  Stick with the polynomial / table-lookup approximations in `04-simd-analog.md`.
- **Jiles-Atherton hysteresis** for Transformer saturation. Defer
  until a dedicated Tape module exists; ship the cheap "tanh +
  one-pole memory" model in Circuit.
- **PVDR (Phase Vocoder Done Right) for v1 Past Stretch.** ~500 LOC
  of careful index arithmetic for a quality bar most users won't
  hear. Tag as "v2 quality upgrade if users complain about Stretch
  artifacts."
- **GPU / wgpu compute path.** Phase 7 only, and only if CPU-only
  modes prove insufficient. Easily a 6-month tar pit.
- **Kepler orbits** for Orbital. Linear approximation is enough.
- **Multi-mode-per-slot.** Several modules wanted it; all defer to
  v2 across the board (per existing audit consensus).

---

## Non-obvious correctness requirement

**Mathieu parametric instability under hop-rate curve modulation
applies to all three physics modules** (Kinetics, Life, Geometry).
Without 1-pole smoothing on per-bin parameter curves *between curve
read and integrator evaluation*, the integrators exhibit hop-rate-
aliased instabilities (parametric "ring up" for springs,
conservation drift for diffusion, parametric ringing for waves).

This is not a quality knob. It is a stability requirement that the
research surfaced as a single, easy-to-miss bug class that would
otherwise show up as "the module is unstable in production but works
in unit tests." See `03-physical-models.md` cross-topic synthesis
for the math and the empirical Mathieu sweep proposal.

The 1-pole smoothing helper is small enough to share across the
three modules — recommend exposing it in `dsp::utils` and consuming
it from any module that does per-hop integrator steps.

---

## Roadmap implications

`99-implementation-roadmap.md` was written before this research
landed. Three suggested deltas, framed as proposals for the
brainstorming session:

### Delta 1 — Phase 1 should bundle the cross-cutting infra

The current Phase 1 lists `ModuleContext` additions, `ModuleSpec`
hints, `heavy_cpu` flag, panel callback, reset-to-default, and the
modulation ring UI. None of those are blocking for the *modules* —
they are general housekeeping.

The **shared infra pieces (1)–(5) above** are blocking. Bundling
them into Phase 1 (or a new "Phase 1.5: shared kernels") means every
later phase consumes pre-built primitives instead of re-deriving the
same per-bin loop. Specifically:

- Phase 2 quick-win modules (Future, Punch, Modulate-light) all
  benefit from the shared `if_offset` cache + `PhaseRotator`. Today
  they would each carry their own copy.
- Phase 5 heavy physics modules (Life, Kinetics) need the 1-pole
  smoothing helper and the energy-tracker pattern.
- Phase 4 (PVX) becomes a thin Pipeline-level PR if pieces (1)+(2)
  are already there.

### Delta 2 — Phase 4 (PVX → LDR) should land before Phase 2 quick-wins

Today the order is: Phase 2 quick-wins → Phase 3 BinPhysics →
Phase 4 PVX. The research argues for: shared peak detection at
Pipeline level *first*, because:

- Punch (Phase 2) wants the shared peak set for its sidechain
  detection (`PeakDetector` in `06-specialized-topics.md`).
- Modulate PLL Tear (Phase 2-light or Phase 5b) wants the same peak
  set as a `track_mask`.
- Future Predict (Phase 2) benefits from the IF derived in the
  unwrap pass.

So: **PVX Phase 4.1 (unwrap + peak detection at Pipeline level)
becomes Phase 1.6, not Phase 4.** Module-level PVX integrations
(Phase 4.3a/b/c) stay in Phase 4 as they are today.

### Delta 3 — Add the 1-pole curve smoothing helper as a Phase 1 deliverable

Per the Mathieu correctness requirement above. Single small helper,
big stability payoff. Wiring it through every per-bin parameter
curve read in physics modules is the responsibility of the consumer
modules, but the helper itself ships in Phase 1 alongside the other
shared pieces.

---

## Open questions for the brainstorming session

The research closed many open questions but leaves these open for
Kim's judgment, not further research:

1. **Naming.** Drop "PVX" everywhere in favour of "PLPV" or "LDR"?
   The acronym shows up in `20-plpv-phase-cross-cutting.md`,
   `99-implementation-roadmap.md`, `00-README.md`, and in several
   per-module audits. Recommendation: rename to "PLPV" (descriptive)
   in the next spec round. **Resolved 2026-04-27:** renamed to PLPV
   across forward-looking design files; historical research outputs
   under `research/` and `90-research-prompts.md` keep their original
   wording for traceability.
2. **Should we ship Phase 4.1 (LDR peak set + unwrapped phase) as
   part of Phase 1?** Per Delta 2 above. Decision affects the
   release-cadence story for the v0.X.0 infra release.
3. **Past Stretch v1 phase quality.** Puckette/lamination is the
   recommended starting point. If Kim's listening tests prefer LDR
   rigid lock from day one, the upgrade path is a single follow-up
   PR (the peak set is already shared infra). Confirm direction.
4. **`needs_cepstrum: bool` lazy compute** for the cepstrum buffer.
   Only Lifter needs it; lazy compute keeps non-Harmony chains
   cheap. Confirm we want lazy-on-demand rather than always-on.
5. **`SimdRng` per-slot vs global.** The Circuit module wants
   per-slot RNG sub-streams (so the same patch is reproducible);
   adding a global one is cheaper but loses reproducibility.
   Recommend per-slot.
6. **Worker-thread DSP** as a global infra piece. Persistent
   Homology (v2 Geometry mode) is the first module to need it.
   Worth specifying once now even if no v1 module consumes it.

---

## Pointers

- Detailed module-level findings: each per-module audit file's
  "Research findings (2026-04-26)" section.
- Underlying research output: `research/01-pvx-phase-and-pll.md`
  through `research/06-specialized-topics.md`.
- Original prompts the research answered: `90-research-prompts.md`.
- Roadmap that the proposed deltas would amend:
  `99-implementation-roadmap.md`.

---

## Addendum (2026-04-27) — re-audit of `repos/pvx`

After the synthesis above was written, the `repos/pvx` checkout was
re-read end-to-end. **It is not the ProSoniq PVX paper that the
audit files reference — it is `pvx`, an MIT-licensed Python toolkit
by Colby Leider** (Copyright 2026 TheColby). The library is a
serious phase-vocoder + multichannel audio toolkit that the prior
synthesis effectively ignored.

This is a real research gap, and several prior conclusions need
adjustment.

### What `repos/pvx` actually is

- 6171-line `src/pvx/core/voc.py` — production phase vocoder.
- 4 `src/pvx/core/pvc_*.py` files (~1250 LOC total) — Paul Koonce
  PVC-style operators (functions, ops, harmony, resonators).
- Per-frame transient detection in `transients.py` (291 LOC) using
  spectral flux + HFC + broadbandness, robust-percentile normalised.
- Hybrid PV+WSOLA transient handling in `wsola.py` (122 LOC) +
  `--transient-mode hybrid|wsola` flag.
- Stereo helper in `stereo.py` is intentionally tiny (40 LOC for
  LR↔MS); the *coherence* work happens inside `voc.py`'s hop loop.
- 248-entry bibliography in `docs/references.bib`.
- MIT licence — algorithms are reusable as references; literal code
  isn't a great fit (Python + NumPy + CuPy, not real-time Rust).

### What was wrong in the synthesis above

1. **Naming.** "PVX" in `repos/pvx` is the toolkit's brand, not the
   ProSoniq paper. Our internal "PVX" feature name therefore
   *collides* with an actual MIT-licensed library Kim has on disk.
   Renaming to **PLPV** (descriptive of the Peak-Locked Phase
   Vocoder math) is now a stronger recommendation, not a stylistic
   one — it removes a real namespace clash.
2. **The cited paper count was incomplete.** The motivation paper
   for Past Stretch's quality concerns is **Laroche & Dolson 1997
   "About this Phasiness Business"** (line 417 of `references.bib`),
   distinct from the 1999 WASPAA peak-locking paper we already
   cited. Both should be in our reference set.
3. **Estimated implementation size for LDR identity locking was
   too high.** `voc.py:2509-2542` shows the entire algorithm in
   ~33 lines: `find_spectral_peaks` is a 3-point local-maximum
   scan (no magnitude threshold, no skirt-width math), and
   `apply_identity_phase_locking` does Voronoi (nearest-peak)
   skirt assignment plus the identity formula
   `locked = synth_phase[nearest_peak] + (analysis_phase −
   analysis_phase[nearest_peak])`. Our PLPV "Phase 1.6" PR is
   closer to **~50 LOC of Rust**, not ~200. Cheaper to ship; argues
   even more strongly for landing it before the Phase-2 quick-wins.

### What the synthesis missed (genuinely new findings)

1. **Low-energy bin phase damping.** Per
   `docs/PHASINESS_IMPLEMENTATION_PLAN.md` Phase 1: bins below a
   user-configurable noise floor get *damped* phase rather than
   peak-locked phase. This avoids "unstable phase diffusion in
   low-energy bins" — a quality concern none of our research
   prompts surfaced. Add as a third deliverable for the PLPV PR
   alongside unwrap and peak detection. CLI parameter shape:
   `--phase-noise-floor-db <dB>`.
2. **Adaptive per-frame coherence policy.** Per
   `PHASINESS_IMPLEMENTATION_PLAN.md` Phase 2: tonal frames use
   strong identity-lock; noisy/percussive frames use relaxed lock
   + transient protection. This is *more* than Roebel COG
   transient detection — Roebel gives the input signal, but the
   *policy switch* (which lock mode to use this frame) is a
   separate decision. Cheap to add (one boolean per frame), worth
   a `--phase-policy {static,adaptive}` parameter.
3. **Stereo M/S phase-coherence preservation.** This is a quality
   pillar for `pvx` and very likely a quality bug in Spectral
   Forge's shipped `StereoLink::MidSide` mode. Specifically:
   - Inter-channel phase delta `Δφᵢⱼ(k,t) = φᵢ(k,t) − φⱼ(k,t)`
     should be preserved across processing.
   - Phase-drift objective `J = Σ |Δφ_out − Δφ_in|` should
     stay near zero.
   - Lock decisions should apply *in the M/S domain* for stereo-
     preserving stretch.
   - **Action item:** verify whether our shipped MidSide preserves
     inter-channel phase. If not, add it as a calibration probe
     metric (`probe_interchannel_phase_drift_rad`) and treat
     drift > ε as a regression.
4. **Voronoi (nearest-peak) skirt assignment is the simplest
   workable approach.** The research synthesis presented
   magnitude-threshold and IF-defined skirts as the right answer.
   `pvx` ships Voronoi assignment in production. Grading:
   *Voronoi (3-point peaks, nearest-peak skirts) ≤ magnitude-
   threshold ≤ adaptive temporal-smoothed.* Voronoi is the v1
   default; the audit's "magnitude-defined skirt" is a v2 quality
   refinement.
5. **Hybrid PV+WSOLA transient handling** is a real, shipped
   technique. Our synthesis dismissed WSOLA implicitly. For Past
   Stretch v2 (if "tape-style stretch artifact is the feature"
   stops being the user-visible framing), hybrid PV+WSOLA is a
   well-trodden path.
6. **Multi-resolution phase vocoder** (different N per band) is in
   `pvx`'s algorithm list and we didn't mention it at all. Out of
   scope for our fixed-FFT plugin today, but file as a potential
   v2 feature if frequency-dependent time-frequency resolution
   ever becomes a user demand.
7. **Paul Koonce's PVC** is the canonical CCRMA reference. The
   `pvc_*.py` files (`pvc_functions`, `pvc_ops`, `pvc_harmony`,
   `pvc_resonators`) are direct ports of his operator vocabulary.
   `docs/PVC_LESSONS.md` is mostly UX lessons (composable tools,
   external control files, terminal-first), but the operator names
   (`filter`, `tvfilter`, `ringfilter`, `chordmapper`,
   `inharmonator`) are a useful reference vocabulary if/when we
   spec a Spectral Delay or Resonator module.

### What was confirmed by `repos/pvx`

1. **Identity-locking formula** matches ours exactly. Our research
   wasn't wrong about the math; just about the literature lineage
   and the implementation size.
2. **Classical pitch detection + opt-in neural** is also `pvx`'s
   strategy: it ships YIN, pYIN, RAPT, SWIPE, HPS, subharmonic
   summation, *and* CREPE side-by-side. This validates our
   recommendation to defer neural to opt-in.
3. **Transient-driven behaviour switching** (not just detection)
   is the right pattern. `pvx`'s `phase_locking="identity"` only
   activates when `phase_engine != "random"` and adapts per-frame.
4. **Phase coherence as the v1 quality concern under stretch** is
   `pvx`'s entire motivation, per `PHASINESS_IMPLEMENTATION_PLAN.md`.
   We were correct to centre our roadmap there.

### Recommended actions

1. **Rename "PVX" to "PLPV" everywhere** in this folder and in
   `docs/superpowers/specs/` before any spec lands. The collision
   with the unrelated `repos/pvx` library is going to confuse
   reviewers and future readers. (Affected files: `00-README.md`,
   `20-plpv-phase-cross-cutting.md`, `99-implementation-roadmap.md`,
   `15-harmony.md`, `13-past.md`, `16-modulate.md`, plus the
   spec under `docs/superpowers/specs/2026-04-21-pvx-*.md` if any.)
   **Done 2026-04-27** for the ideas/ folder.
2. **Update PLPV Phase 1.6 deliverables** to include low-energy
   bin damping (third deliverable alongside unwrap and peak
   detection). Estimated total still under 100 LOC of Rust.
3. **Add inter-channel phase-drift probe** for the shipped
   MidSide mode as a regression check. If the metric reveals
   drift today, that's a separate quality bug to file.
4. **Add Laroche-Dolson 1997** to the synthesis reference list.
5. **Defer adaptive per-frame coherence policy** to a v2 quality
   refinement on top of the basic PLPV PR — the static identity-
   lock is good enough for v1.
6. **No code lift from `repos/pvx`** is recommended. License is
   compatible (MIT) but the language and runtime are wrong
   (Python + CuPy vs real-time Rust). Use `voc.py` as a *reference
   implementation* for verifying our Rust port produces the same
   per-frame behaviour.

### Open question for Kim

`pvx` ships `--transient-mode {hybrid,wsola,reset}` as the user-
visible knob for transient handling. The Past audit's framing is
"tape-style stretch artifact is the feature" — i.e., transient
smearing is intended. **Should Past Stretch ship a `transient-mode`
parameter at all?** Two reasonable answers:

- **No, never:** the smear *is* the character; exposing transient
  modes invites users to "fix" what isn't broken.
- **Yes, with default = `smear`:** lets advanced users do
  transparent stretches when they need them, without growing a
  separate module.

Tag for the brainstorming session. Recommendation: ship without
the parameter for v1 (simpler model), revisit in v2 if user
feedback demands transparent-stretch support.
