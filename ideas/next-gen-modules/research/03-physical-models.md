# Research: Physical Models — Springs, Diffusion, 2D Wave

**Source prompts:** 8, 9, 12 in `90-research-prompts.md`
**Status:** Findings as of 2026-04-26
**Plugin context:** Spectral CLAP, Rust, real-time, 8193 FFT bins, hop=128–1024,
slot update rate 43–344 Hz at 44.1 kHz.

## Refined research questions

The original prompts asked open questions. Three concrete sub-problems fell out
once the literature was inventoried:

1. **What is the actual stability bound for a coupled-spring chain at our hop
   rate, expressed in terms of "max stiffness" rather than abstract `omega*dt`?**
   Spring chains have eigenvalues that scale with both stiffness and connection
   topology (the chain Laplacian); a single-oscillator bound is not enough.

2. **For a "diffuse loud bins into neighbours" Life sub-effect, do we need a
   conservative scheme (finite volume, Lattice Boltzmann) or is a plain
   Laplacian smoothing of magnitude already energy-conservative enough that the
   audible difference is sub-threshold?** Audio listeners care about RMS/loudness
   over windows of ~10–50 ms, not per-bin instantaneous conservation.

3. **For a 2D wave equation on a Hilbert-mapped grid, what is the practical
   useful wave-speed range and what does the effective neighbourhood look like
   when the bin-bin coupling implied by Hilbert non-locality is unrolled?**
   The intuition that "Hilbert preserves locality" hides a long tail of
   non-neighbour pairs that the wave equation will couple.

These three sub-problems guide the rest of the document.

---

## Topic A — Stiff spring/mass at audio hop rate

### Integrator comparison

| Scheme | Order | Stability bound (linear) | Energy behaviour | CPU/step | SIMD-friendly | Notes for our use |
|---|---|---|---|---|---|---|
| **Forward Euler** | 1 | unconditionally **un**stable for undamped oscillators (spirals out) | gains energy each step | 1 mul + 1 add per state | trivial | useless — would explode immediately |
| **Backward Euler (implicit)** | 1 | A-stable (handles any timestep) | strongly damped — eats energy, kills the ringing we want | per-step linear solve (sparse Newton) | hard (Jacobian assembly) | over-damps; defeats the "spring" character |
| **Symplectic Euler** (= Euler-Cromer = NSV = "semi-implicit Euler") | 1 | conditionally stable: `omega*dt < 2` | bounded (energy oscillates around mean, not strictly conserved) | same as Forward Euler | trivial | best cheap option |
| **Velocity Verlet** (= Stoermer-Verlet) | 2 | conditionally stable: `omega*dt < 2` | same as Symplectic Euler but lower truncation error | 2 force evals per step (or one with cached acceleration) | trivial | same stability bound, better accuracy |
| **Position Verlet** (= "leapfrog") | 2 | `omega*dt < 2` | same | 1 force eval per step | trivial | mathematically equivalent to Velocity Verlet, differs only in I/O |
| **Implicit Midpoint** | 2 | unconditionally stable; conserves quadratic invariants | strict energy conservation for quadratic Hamiltonians, but **explodes for nonlinear / time-varying systems** | per-step linear solve | hard | unsuitable here — Dinev et al. show it explodes when modulated |
| **IMEX (Newmark/SDIRK split)** | 2 | implicit on stiff modes, explicit on soft modes | depends on splitting | mixed; one tridiagonal-class solve per step | partial | overkill for our update rate |
| **XPBD** (Macklin) | 1 | unconditionally stable per-iteration (constraint-based) | controlled by compliance and damping params | 1 Gauss-Seidel sweep per constraint | tricky for spring chains (depends on order) | promising for one-shot stability but Gauss-Seidel is sequential |
| **Dinev/Liu 2018 "Stabilizing Integrators"** | mixed | tracks energy, blends midpoint with backward Euler when energy grows | minimal artificial damping | per-step local/global solve | poor (SOA-unfriendly Newton) | designed for graphics scenes, too heavy here |

**Key result for our spec:** symplectic Euler and Velocity Verlet share the same
stability bound `omega*dt < 2` for the linear harmonic oscillator. Verlet wins
on accuracy (order 2 vs 1) at no real CPU cost. Below the stability boundary
both methods conserve a *shadow Hamiltonian* — a slight perturbation of the true
energy that bounds the long-time energy error to `O(dt^2)` (rather than growing
secularly as forward Euler does). This is the textbook reason Verlet is the
default choice for molecular dynamics.

### Key references

- **Stefan Bilbao, *Numerical Sound Synthesis: Finite Difference Schemes and
  Simulation in Musical Acoustics*** (Wiley 2009).
  [Online TOC](https://ccrma.stanford.edu/~bilbao/booktop/booktop.html). The
  oscillator chapter (nodes 30–55) covers symplectic schemes for the SHO,
  energy analysis (node 41), accuracy (node 40), and computational cost
  (node 42). The 1D wave equation chapter (nodes 86–115) gives von Neumann
  stability analysis (node 99) and bounds on solution size (node 103). This is
  the canonical reference for "audio-rate finite-difference physical models."

- **Bilbao & Smith (2005), "Energy-conserving finite difference schemes for
  nonlinear strings"**, Acta Acustica united with Acustica 91:299–311.
  [Semantic Scholar](https://www.semanticscholar.org/paper/Energy-conserving-finite-difference-schemes-for-Bilbao-Smith/b8629742004b4373f6d637b4ada6fadf887e3780).
  Discrete energy conservation as a proof-of-stability technique. This is the
  "energy method" that lets you prove a scheme stable for nonlinear PDEs where
  von Neumann analysis would fail. Important for us because user-driven curve
  modulation is effectively a time-varying nonlinearity.

- **Dinev, Liu, Kavan (2018), "Stabilizing Integrators for Real-Time Physics"**,
  ACM TOG 37(1).
  [Project page](https://users.cs.utah.edu/~ladislav/dinev18stabilizing/dinev18stabilizing.html).
  Blends implicit midpoint with backward/forward Euler based on energy tracking;
  shows that pure implicit midpoint explodes on stiff systems and pure backward
  Euler kills the dynamics. The blending idea is good but the per-step Newton
  solve is too expensive for 8193 bins at 86–344 Hz.

- **Liu, Bargteil, O'Brien, Kavan (2013), "Fast Simulation of Mass-Spring
  Systems"**, ACM TOG.
  [PDF](http://graphics.berkeley.edu/papers/Liu-FSM-2013-11/Liu-FSM-2013-11.pdf).
  The "local/global" projective dynamics approach: implicit Euler reformulated
  as a fixed-point iteration that decouples each spring. Each iteration is
  embarrassingly parallel. Worth borrowing for sympathetic harmonic springs.

- **Macklin, Mueller, Chentanez (2016), "XPBD: Position-Based Simulation of
  Compliant Constrained Dynamics"**.
  [PDF](https://matthias-research.github.io/pages/publications/XPBD.pdf).
  Adds a compliance parameter `alpha = compliance / dt^2` so stiffness becomes
  time-step-and-iteration-count independent. PBD is normally Gauss-Seidel
  sequential, which kills SIMD. XPBD with Jacobi iteration (single pass per
  hop) is more useful for us.

- **Macklin (2019), "Small Steps in Physics Simulation"** (mmacklin.com).
  Argues that one big step with N solver iterations is *worse* than N small
  steps with one iteration each. Relevant counter-argument to our "no
  substepping" goal. Within a hop, however, we have ~512 audio samples between
  parameter changes, so we *could* substep cheaply if we wanted to — but only
  for stiff slots, gated by a per-slot CFL-style check.

- **Hairer, Lubich, Wanner (2003), "Geometric Numerical Integration
  Illustrated by the Stoermer-Verlet Method"**, Acta Numerica.
  [PDF](https://www.math.kit.edu/ianm3/lehre/geonumint2009s/media/gni_by_stoermer-verlet.pdf).
  The canonical mathematical treatment. Section on "shadow Hamiltonian" gives
  the rigorous statement of why Verlet's energy doesn't drift.

- **Toxvaerd (2018), "Shadow Hamiltonian in classical NVE molecular dynamics
  simulations"**.
  [ResearchGate](https://www.researchgate.net/publication/338523186).
  Practical numerical observation that Verlet's stability boundary is
  *exactly* `dt = 2/omega` for the linear SHO; beyond it, the shadow
  Hamiltonian becomes complex (eigenvalues leave the unit circle).

### Synthesis

For our problem the textbook picture and the physical-modelling-of-audio
literature converge on the same answer: **start with Velocity Verlet
(equivalently leapfrog or symplectic Euler), add per-bin viscous damping, and
police the stability boundary at parameter-change time rather than per-hop.**

The stability bound `omega*dt < 2` is the hard constraint. With our hop
durations:

- hop = 128 samples at 44.1 kHz → dt = 2.90 ms → max omega = 689 rad/s →
  max f = 109 Hz. This is the **upper safe spring resonance** at the fastest
  hop rate for raw Verlet. A spring tuned to 110 Hz oscillation would already
  ring marginally.
- hop = 256 → dt = 5.80 ms → max f = 54.6 Hz.
- hop = 512 → dt = 11.6 ms → max f = 27.4 Hz.
- hop = 1024 → dt = 23.2 ms → max f = 13.7 Hz.

This is *much* lower than the user might naively expect when they crank a
"stiffness" knob. Two consequences:

1. We must clamp the user-facing stiffness curve to the per-hop CFL bound, or
   the simulation explodes silently. The clamp is hop-rate-dependent, which is
   awkward UX (the same knob does different things at different FFT settings).
   The cleanest approach: expose stiffness as an **angular frequency** (the
   ringing frequency of an isolated spring with mass=1) rather than as a raw
   spring constant. Then the clamp at `omega < 2/dt` is a single value the
   user sees as a frequency ceiling that scales with FFT setting.

2. Above the CFL bound, the spring will ring at the Nyquist rate of the hop
   stream rather than at its physical frequency. That is a sub-octave noise
   that *might* be musically useful but is more likely to be ugly. A safer
   move is to substep stiff slots: for any spring above `omega = 1/dt`, do
   `ceil(omega*dt)` substeps per hop. CPU stays bounded as long as users
   don't crank stiffness across the whole spectrum.

For coupled chains the stability bound tightens further. A linear chain of
identical masses with identical springs has its largest eigenvalue at
`omega_max = 2*sqrt(k/m)`, twice the single-oscillator natural frequency.
**For a chain network we therefore need `omega_max * dt < 2`, i.e.
`dt < 1/sqrt(k/m)`.** The chain effectively halves the safe stiffness. A
network with sympathetic harmonic springs (bin K connected to 2K, 3K, 4K) has
even larger spectral radius — every additional connection per node pushes the
top eigenvalue up. A safe rule of thumb from FDTD: estimate the spectral
radius as `omega_max ≈ 2 * sqrt(k_max * (degree_max + 1) / m_min)` and use
that in the CFL check.

### Stiffness limits and parametric amplification

User-driven curve modulation between hops *is* parametric forcing. When
`stiffness(t)` changes at the hop rate, the system is exactly Mathieu-like
(the time-varying-stiffness oscillator) with the modulation frequency equal to
the hop rate.

Mathieu's equation has well-known instability tongues centred on
`omega_drive = 2*omega_natural / n` for integer n. With our hop rate as the
"drive" and the spring's natural frequency below the CFL bound, the dangerous
tongues are at hop-rate-multiples of the spring frequency. The most dangerous
case is `omega_natural ≈ omega_drive / 2` — i.e. the spring's frequency is
half the hop rate. At hop=512 (dt=11.6 ms, hop rate 86 Hz), springs around
43 Hz can be parametrically pumped if their stiffness is modulated at the hop
rate. Audible result: rather than tracking the user's curve smoothly, the
spring chains "ring up" at half the hop rate, producing artifacts with a
spectral centre that drifts with hop setting.

References:
- **Berge, Pomeau, Vidal, *Order Within Chaos*** for Mathieu instability tongues
- [Damping by parametric stiffness excitation](https://link.springer.com/article/10.1007/s11071-007-9325-z) — averaging method shows
  the boundary tongue at `Omega = 2*omega_0` is the dominant one.

**Mitigation strategies:**
1. **Smooth user curves at the audio rate** with a 1-pole follower with time
   constant ≥ 4*dt before they hit the integrator. This converts the discrete
   per-hop parameter steps into a continuous trajectory and pushes the
   modulation spectrum below the dangerous regions.
2. **Add unconditional viscous damping** with damping ratio ≥ 0.05. Mathieu
   tongues shrink rapidly with damping; ζ=0.05 cuts the tongue width by
   roughly the factor we need to make hop-rate-aliased modulation safe across
   the spectrum.
3. **Energy clamp** as a last-resort safety net: track per-bin kinetic energy,
   if it grows by more than 6 dB in two hops, scale velocity by sqrt(0.5).
   This is Dinev/Liu's idea applied cheaply per-bin without their Newton
   machinery. Catches numerical blowups due to user knob slamming without
   killing legitimate transients.

### Sparse harmonic-spring access pattern

Sympathetic harmonic springs connect bin K to 2K, 3K, 4K, ... K_max/K. The
access pattern is non-stride-1 and reads grow more spread as K shrinks: at
K=1 the spring touches *every* bin in the spectrum; at K=4096 it touches
only one (8192). This is bad for SIMD and bad for cache.

Three implementation options:

**Option A — CSR-style sparse matrix, evaluated as SpMV per hop.**
Pros: standard infrastructure, handles arbitrary topologies.
Cons: SpMV with this row-density distribution is a known hard case for SIMD
([SC09 SpMV throughput](https://www.nvidia.com/docs/io/77944/sc09-spmv-throughput.pdf)).
Per-row SIMD is poor when row lengths vary by 4000x.

**Option B — Per-bin small fixed array of harmonic neighbours, hand-rolled
gather loops.**
Pros: cache-friendly per-bin update; vectorises across bins (each bin's
harmonics can be summed with AVX-256 reading from gather addresses).
Cons: degree varies per bin (K=1 has 8192 harmonics, K=4096 has 1). Have to
cap maximum harmonic count, e.g. only first 8 harmonics. With cap=8, the
inner loop becomes 8 fixed gathers per bin; that's 1 AVX-512 gather
instruction per harmonic.

**Option C — Two-pass scatter/gather with a precomputed harmonic-index LUT.**
Forward pass: for each bin K, scatter K's velocity/displacement into a
"harmonic accumulator" buffer at indices 2K, 3K, 4K... up to cap. Backward
pass: read the harmonic accumulator and apply spring force. Trade an extra
buffer-pass for stride-1 reads.
Pros: stride-1 SIMD, low branch.
Cons: 2x memory traffic.

**Recommendation:** Option B with cap=8 harmonics per bin. AVX-512 gather is
mature on the target hardware (Bilbao's NESS work uses similar patterns on
GPUs but the data shape is the same). Cap=8 is enough for "bin K is connected
to its first 8 overtones." Sparser-than-8 connections get NaN-padded entries
that map to the bin itself (self-loop with zero weight — valid and free).

### Implementation candidates

- **fundsp (Sami Perttu)** — [github.com/SamiPerttu/fundsp](https://github.com/SamiPerttu/fundsp) — Rust DSP library with FFT support. Doesn't ship spring-mass primitives but the SoA pattern they use in oscillator banks is identical to what we need.
- **DASP** — [github.com/RustAudio/dasp](https://github.com/RustAudio/dasp) — pure Rust, low-level traits. No physical models per se.
- **Faust physmodels.lib** — [grame-cncm/faustlibraries](https://github.com/grame-cncm/faustlibraries/blob/master/physmodels.lib) — plenty of mass-spring primitives in Faust. Not directly usable but is a cheat sheet for which models are simple to implement.
- **rasp (buosseph/rasp)** — [github.com/buosseph/rasp](https://github.com/buosseph/rasp) — Rust port of STK. Has Karplus-Strong and waveguide primitives that share structure with what we want.
- **NESS Project** — [ness.music.ed.ac.uk](https://www.ness.music.ed.ac.uk/project) — Bilbao group's GPU implementations of large-scale physical models. Not real-time on CPU but has the most relevant architecture pattern (per-grid-point parallel update with hop-rate parameter modulation).

### Recommendations

1. **Use Velocity Verlet with per-bin viscous damping** (`damp ≥ 0.05`), SoA
   layout, AVX-512-friendly stride-1 inner loops over `displacement[]`,
   `velocity[]`, `mass[]`, `stiffness[]`.
2. **Express user-facing stiffness as an angular frequency**, not a raw spring
   constant. Clamp to `omega_max < 1.5/dt` (50% safety margin from the CFL
   bound). The clamp value scales with FFT-size choice, displayed to the user
   as a per-FFT-size "max spring frequency."
3. **Smooth all per-bin parameter curves at hop boundaries** with a 1-pole
   IIR (time constant 4*dt). This removes the dominant Mathieu instability
   tongue centred on the hop rate.
4. **Sympathetic harmonic springs: cap at 8 harmonics, gather pattern.**
   Document the cap as a v1 limitation.
5. **Per-bin energy-rise clamp** as runtime safety: scale velocities by
   sqrt(0.5) on any bin where kinetic+potential energy doubles in 2 hops.
6. **Avoid implicit Euler / midpoint / Newton-style integrators.** Their
   per-step solver cost dominates and they over-damp the dynamics that make
   springs interesting. The XPBD compliance pattern is interesting but
   Gauss-Seidel sequencing kills SIMD; XPBD with Jacobi iteration is the only
   variant worth considering.

---

## Topic B — Energy-conserving spectral diffusion

### Key references

- **Bilbao chapter on diffusion in 1D** (in *Numerical Sound Synthesis*, the
  bar-and-string chapters discuss damping/diffusion as a Laplacian of velocity).
- **Crank-Nicolson method**, [Wikipedia](https://en.wikipedia.org/wiki/Crank%E2%80%93Nicolson_method) — unconditionally stable second-order scheme for 1D heat equation.
  Tridiagonal solve per step via Thomas algorithm in O(N).
- **FTCS scheme**, [Wikipedia](https://en.wikipedia.org/wiki/FTCS_scheme) — explicit forward-Euler for 1D heat. Stable iff `D*dt/dx^2 ≤ 0.5`.
- **Dellar (2001), "A lattice Boltzmann equation for diffusion"**, [J. Stat. Phys.](https://link.springer.com/article/10.1007/BF02181215) — the simplest LBM diffusion (D1Q2) reduces to FTCS at relaxation parameter omega=1.
- **Dubois & Lallemand (2013), "Construction and Analysis of Lattice Boltzmann
  Methods Applied to a 1D Convection-Diffusion Equation"**, Acta Applicandae
  Mathematicae. [Springer link](https://link.springer.com/article/10.1007/s10440-013-9850-3).
- **HESS conservative finite-volume Saint-Venant**, [Article](https://hess.copernicus.org/articles/23/1281/2019/) — example of finite-volume conservation for shallow-water 1D. Same machinery applies to magnitude redistribution.
- **MITgcm advection schemes overview**, [docs](https://mitgcm.readthedocs.io/en/latest/algorithm/adv-schemes.html) — practical comparison of upwind / central / flux-limited TVD / Superbee.

### Heat equation vs Lattice Boltzmann vs flux-limited

For our use case (smooth loud-bin magnitudes into neighbours, conserve total
energy) all three approaches converge in the small-`D*dt/dx^2` regime:

- **Plain Laplacian (FTCS) on |X|^2 (power):**
  ```
  P[k]_new = P[k] + D * (P[k-1] - 2*P[k] + P[k+1])
  ```
  Conservation: this scheme has a discrete conservation law
  `sum(P[k]_new) = sum(P[k])` (interior), with boundary leaks of `P[0]` and
  `P[N-1]` if you don't reflect. Stability: `D ≤ 0.5`. SIMD-friendliness:
  perfect — it's a 3-tap stride-1 stencil.

- **D1Q2 Lattice Boltzmann diffusion:**
  Two distributions f_+, f_- per bin (rightward, leftward populations).
  Streaming: `f_+[k]_new = f_-[k-1]`, `f_-[k]_new = f_+[k+1]`. Collision:
  toward equilibrium `f_+ = f_- = P/2`. Mass `P = f_+ + f_-` is exactly
  conserved by construction. Reduces to FTCS at relaxation omega=1.
  CPU cost: 2x state, ~3x ops. SIMD: same 3-tap stencil twice.

- **Flux-limited TVD / Superbee:**
  Compute fluxes between cell faces; limit the flux based on slope ratios to
  avoid overshooting in steep gradients. Total variation diminishing — no
  spurious oscillations. CPU cost: 2x ops vs FTCS plus branchy
  `min/max/sign` for the limiter. SIMD: workable with AVX `min/max` but not
  as clean.

For *audio-perceptual* purposes the difference is sub-threshold for our
operation. We're computing `D` from a user curve in the range [0, ~0.4]; we
already need to cap `D ≤ 0.5` for FTCS stability; the audible "spread" is the
same Gaussian convolution in all three schemes within that range. The
*conservation* of FTCS at that range is to within `1e-7` per hop relative
error (interior; boundaries leak unless explicitly reflected). LBM gives
machine-precision conservation but it does so by *doubling* memory and ops.
TVD/flux-limited matters only when you have extreme gradients and you mind
overshoot — neither of which is true for diffusion of a positive magnitude
spectrum.

**Verdict: plain FTCS is the right answer for audio diffusion.** The
"conservation visible to the listener" is `sum(P)` over a perceptual window of
~30 ms — perhaps 8 hops at fast hop rate, 1 hop at slow. Even with `1e-3`
per-hop conservation error (worst case, including reflective boundaries), an
8-hop window has `1e-2` cumulative drift, well below loudness JND of ~0.5 dB
≈ 12% power.

### Stability under varying viscosity

If the user's per-bin diffusion curve sets `D[k]` rather than a global `D`,
the FTCS update becomes:
```
P[k]_new = P[k] + D[k] * (P[k-1] - 2*P[k] + P[k+1])
```

Two failure modes:

1. **Negative diffusion (anti-diffusion) where user curve dips below zero.**
   This is exactly the unstable backwards heat equation. Either clamp `D[k]
   ≥ 0` (recommended) or cap negative values to a small `epsilon` and accept
   that anti-diffusion will inject high-frequency noise where active.
   Recommend: clamp to `[0, 0.45]`.
2. **Conservation breaks when D varies sharply.** If `D[k] = 0.4` and
   `D[k+1] = 0.05`, the standard FTCS no longer conserves total power — the
   asymmetric flux causes drift. Fix by switching to *finite-volume form*:
   ```
   J[k+1/2] = D_face * (P[k+1] - P[k])  // flux at face k+1/2
   D_face = harmonic_mean(D[k], D[k+1])
   P[k]_new = P[k] + (J[k+1/2] - J[k-1/2])  // since flux into k = -flux out
   ```
   With the same flux on both sides of an interior face, conservation is
   exact. Cost: same as FTCS plus one harmonic-mean per face. SIMD-friendly.

The flux-limiter question (Superbee, Van Leer, etc.) is irrelevant here: it
addresses overshoot in *advection*, not diffusion. The recommended scheme
above doesn't overshoot because it's a discrete elliptic operator with
non-negative weights when `D[k] ≥ 0`.

### Implementation candidates

- **WebGPU reaction-diffusion examples** like
  [robert-leitl/webgpu-reaction-diffusion](https://github.com/robert-leitl/webgpu-reaction-diffusion)
  show the GPU compute-shader layout. CPU SoA layout is similar, just
  vectorised with AVX-256.
- **Faust** has a `de.delay` plus a `fi.smooth` that together form a 1-pole
  diffusion analogue per-bin; this is similar to per-bin first-order
  smoothing without spatial coupling.

### Recommendations

1. **Plain finite-volume FTCS on `|X|^2` (power) is sufficient.** Use the
   harmonic-mean face flux to handle per-bin viscosity:
   ```rust
   let d_face_left  = 2.0 * d[k] * d[k-1] / (d[k] + d[k-1] + EPS);
   let d_face_right = 2.0 * d[k] * d[k+1] / (d[k] + d[k+1] + EPS);
   p_new[k] = p[k] + d_face_right * (p[k+1] - p[k])
                   - d_face_left  * (p[k]   - p[k-1]);
   ```
   This is 6 muls + 4 adds + 2 divs per bin, vectorisable across bins.
2. **Clamp `D[k] ∈ [0, 0.45]`** for stability across all hop rates. The 0.45
   ceiling gives a 10% safety margin on the FTCS bound.
3. **Operate on power, output magnitude.** Read `mag = |bin|`, compute
   `power = mag^2`, run diffusion, `mag_new = sqrt(power_new)`, scale the
   complex bin by `mag_new / mag`. Phase preserved. Conservation in `power`
   is what matches perceptual loudness conservation.
4. **Reflective boundaries by default.** First and last bin: clamp the flux
   into the boundary to zero (`J[-1/2] = J[N-1/2] = 0`). This is exact
   conservation including boundary, as opposed to "ghost cell" tricks.
5. **Skip Lattice Boltzmann.** No audible benefit; double the state and
   complexity.
6. **For per-bin "reach" parameter**, iterate the diffusion N times per hop
   (cheap — each iteration is O(num_bins)). Total reach ≈ `sqrt(N * D)` bins.
   So a "reach=8 bins" with `D=0.4` needs `N ≈ 160 / 0.4 = 400` iterations,
   which at 8193 bins is 3.3M flops per hop — fits in 1 ms easily.

---

## Topic C — 2D wave equation on Hilbert-mapped spectrum

### Key references

- **Bilbao 2D wave / membrane / plate chapters** in *Numerical Sound Synthesis*
  (nodes 158–185). Discrete CFL analysis, energy methods, anisotropic
  behaviour.
- **Hamilton & Bilbao (2014), "Finite difference schemes on hexagonal grids
  for thin linear plates"**, [DAFx14](https://dafx14.fau.de/papers/dafx14_brian_hamilton_finite_difference_schemes.pdf).
  Compares stencils for plate equations with stability analysis.
- **Bilbao, Hamilton, Botts, Savioja (2017), "FDTD Methods for 3-D Room
  Acoustics Simulation With High-Order Accuracy in Space and Time"**, IEEE
  TASLP. Higher-order FDTD reduces dispersion.
- **Mur (1981), "Absorbing Boundary Conditions for the Finite-Difference
  Approximation of the Time-Domain Electromagnetic-Field Equations"**, IEEE
  TEMC 23(4). The original first-order ABC paper.
- **Johnson (2008), "Notes on Perfectly Matched Layers (PMLs)"**, MIT,
  [PDF](https://math.mit.edu/~stevenj/18.369/spring09/pml.pdf). Cleaner ABC
  alternative; quadratic absorption profile.
- **Hilbert curve locality** — [Hilbert Curve Reordering](https://www.emergentmind.com/topics/hilbert-curve-reordering)
  notes >90% of points have below-average index jumps for Hilbert vs raster
  or Morton. [Lossless compression of medical images using Hilbert space-
  filling curves](https://www.sciencedirect.com/science/article/abs/pii/S089561110700167X)
  measures locality concretely for image domains.
- **NESS Project** for the GPU-scale precedent.
- **AudioGroupCologne/wavefront** [GitHub](https://github.com/AudioGroupCologne/wavefront) — Rust 2D acoustic
  simulation using TLM. Same stencil shape, useful as a reference codebase.
- **bsxfun/pffdtd** [GitHub](https://github.com/bsxfun/pffdtd) — modern
  CPU/GPU FDTD for room acoustics. Concrete numbers on grid sizes and
  performance.

### CFL bounds at our hop rates and grid size

For the standard 2D leapfrog wave-equation update (5-point stencil) the CFL
condition is `c * dt / dx ≤ 1/sqrt(2) ≈ 0.7071`. For a 91x91 grid with one
finite-difference timestep per hop:

- The "physical timestep" for the wave equation is the hop duration: `dt =
  hop / fs`.
- The "physical grid spacing" `dx` is unitless (we work on a normalised grid).
  Take `dx = 1` for simplicity.
- Then `c ≤ 1/(sqrt(2) * dt)` in units where `dx=1` and time is hop-units.
  Equivalently, `c ≤ 0.707` in those units.

That means the wave can travel at most ~0.7 grid cells per hop. At 91x91 the
diagonal is ~129 cells. Sound takes ~183 hops (~93 ms at hop=512) to cross
the diagonal. For perceptual ringing patterns, 100–500 ms decay is musically
useful, so this is in the right ballpark — a single hop is one "ripple step,"
and a transient injection rings for several hundred hops.

**To make this useful as an effect, expose `c` as the user knob.** The user's
"WAVE_SPEED" curve maps to `c ∈ [0.05, 0.7]`. Lower c = slow ripples (long
ring time), higher c = fast ripples (short ring time, more dispersion). The
upper bound is the CFL ceiling.

**Numerical dispersion** is severe at the CFL bound (the discrete dispersion
relation deviates from the continuous one). For musical use this is fine — we
*want* the dispersion because it makes the simulation sound like a real
plate, not a perfect waveguide. But the user should know that "c near 0.7"
gives bright, phasey ringing while "c near 0.1" gives smoother, lower-pitched
ringing.

### Boundary condition tradeoffs

Three contender boundary conditions, all at the spatial edges of the 91x91
grid (which after Hilbert un-mapping correspond to *somewhere* in the
spectrum — we'll get to that):

1. **Neumann (reflective) — `du/dn = 0`.** Implementation: ghost cell
   `u[-1] = u[1]`. Simplest. Energy is exactly conserved (no leakage). Sounds
   like a closed plate; rings forever (until viscous damping kills it).
   Musically: best for "tonal" patches that should sustain.
2. **Periodic (toroidal) — `u[-1] = u[N-1]`.** Energy conserved, but
   wave-travelling-around-the-torus creates beats and modal structure
   different from the flat plate. The Hilbert-projected meaning is "spectrum
   wraps around" which is musically weird. Not recommended.
3. **Mur first-order ABC — `u_t + c*u_n = 0` at boundary.** Energy leaks out.
   Sounds like a plate with absorbent edges. Cheaper than PML. Imperfect
   absorption (some reflection at oblique angles) but fine for our case
   because a Hilbert-mapped grid has no meaningful "angle of incidence."
   Musically: best for "transient" patches that should decay.
4. **PML (perfectly matched layer).** A layer of artificial damping with a
   quadratic profile near the boundary. Best absorption but adds memory
   (the layer thickness — 4–8 cells minimum) and CPU. Overkill for our use
   (we don't need >40 dB absorption).

**Per-bin boundary curve (the spec's Wavefield "BOUNDARY" curve).** The user
draws a curve mapping bin index → boundary reflectivity ∈ [0, 1]. We mix
Neumann and Mur per-cell:
```
u_boundary = lerp(u_mur, u_neumann, reflectivity[k])
```
This works smoothly. The Hilbert-mapped meaning: bins with high
`reflectivity` resonate; bins with low `reflectivity` damp out. UX: the user
draws "where the spectrum holds vibration vs where it leaks" — meaningful as
a spectral colouration.

### Hilbert curve locality preservation under wave dynamics

This is the question most worth scrutinising. The intuition "Hilbert curve
preserves locality so wave-equation neighbours in 2D ≈ neighbours in 1D" is
*partially* true but with significant tail behaviour.

The 2D 5-point stencil at grid cell `(x, y)` reads cells `(x±1, y)` and
`(x, y±1)`. After Hilbert un-mapping each of these four neighbours is some
1D bin index. The relevant question is: **what is the distribution of the
bin-index distances `|hilbert_inv(x+1,y) - hilbert_inv(x,y)|`?**

The classical result (Moon, Jagadish, Faloutsos & Saltz, "Analysis of the
Clustering Properties of the Hilbert Space-Filling Curve", IEEE TKDE 2001):
the average distance between 2D-adjacent cells under Hilbert mapping grows
like `sqrt(N)` where N is total cells. For N=8192, that's ~90 — which means
the *average* 2D-neighbour bin-distance is about 90 bins (about 4% of
spectrum). The *median* distance is about 1 (because most Hilbert sub-quadrant
moves are stride-1) but a long tail of jumps cross sub-quadrant boundaries
and can be hundreds of bins apart.

**Implication for our wave equation:** at every hop, every bin's value mixes
into 4 neighbour bins. *Most* of the time those neighbours are within ±3
bins (within the Hilbert sub-quadrant). *Some* of the time they're 100–1000
bins away. The audible result: the wave equation, mapped through Hilbert,
acts as a *mostly-local-but-occasionally-leaping* spectral redistribution.

This is actually musically interesting (more interesting than row-major
mapping which has a more uniform "neighbour 96 bins away" distribution and
thus produces more boring spectral cross-talk). But it does mean that "the
wave equation respects spectral locality" is overstated — there's a long
tail. The Hilbert curve gives us better-than-row-major locality but not
true locality.

Mitigation if needed: replace 4-neighbour Laplacian with 8-neighbour or 9-
neighbour stencils (Bilbao, Hamilton 2014 hexagonal grid argument). With more
neighbours, we average across more Hilbert tail-jumps and the cross-spectrum
leakage smooths out. Cost: 8/4 = 2x ops per cell.

### SIMD strategy for 91×91 grid

`8193 / 91 = 90.0`. Actually `91 * 90 = 8190`, so we have 3 leftover bins. A
clean Hilbert mapping needs a power-of-2 grid. Use `128 x 64 = 8192` (one
spare bin) or `256 x 32 = 8192`, with the 1 leftover bin handled as a virtual
cell. Or: pad the grid to `128 x 128 = 16384` and only use the first 8193
indices in the un-map.

For SIMD, **pad the grid to a multiple of the SIMD width.** AVX-256 = 8
floats; pad to 96x96 with ghost cells on the edges. Each row is then 12
SIMD lanes of 8 floats, processed as 12 vector loads. AVX-512 = 16 floats; pad
to 96x96 = 6 lanes per row.

The two-buffer scheme:
- `u_curr[96 * 96]` = current displacement field
- `u_prev[96 * 96]` = previous

Update kernel (pseudocode):
```rust
for y in 1..GRID-1 {
    for x in (1..GRID-1).step_by(8) {  // AVX-256
        let center = simd_load(&u_curr[y*GRID + x]);
        let left   = simd_loadu(&u_curr[y*GRID + x - 1]);
        let right  = simd_loadu(&u_curr[y*GRID + x + 1]);
        let up     = simd_load(&u_curr[(y-1)*GRID + x]);
        let down   = simd_load(&u_curr[(y+1)*GRID + x]);
        let lap = left + right + up + down - 4.0 * center;
        let prev = simd_load(&u_prev[y*GRID + x]);
        let new  = 2.0 * center - prev + c2_dt2 * lap;
        simd_store(&u_next[y*GRID + x], new);
    }
}
swap(u_prev, u_curr);  // ping-pong
swap(u_curr, u_next);
```

Cost: 5 muls + 6 adds per cell + 5 loads + 1 store. At 8192 cells per channel
per hop, that's ~150k flops per channel — sub-100 microseconds at 100 GFLOPS,
fits real-time easily.

**Boundary handling.** Don't branch in the inner loop. Initialise `u_curr[0,
*]`, `u_curr[GRID-1, *]`, `u_curr[*, 0]`, `u_curr[*, GRID-1]` to ghost values
*before* the inner loop. For Neumann, copy from interior neighbours
(reflective). For Mur first-order:
```
u_next[0,y] = u_curr[1,y] + (c*dt - 1)/(c*dt + 1) * (u_next[1,y] - u_curr[0,y])
```
This branchless boundary update is cheap — 4 separate boundary loops outside
the main vectorised core.

**Hilbert mapping LUT.** Precompute at `reset()`:
- `bin_to_xy[bin]: (u8, u8)` — 8193 entries × 2 bytes = 16 KB.
- `xy_to_bin[y*GRID + x]: u16` — 9216 entries × 2 bytes = 18 KB.

Both fit in L1. Read pattern at the un-map back to 1D is stride-1 over
`bin_to_xy[]` then gather from `u_curr` at the implied (x, y). This is one
gather per bin — AVX-512 has gather support, AVX-256 less so. For AVX-2-only
hardware, fall back to scalar un-map (still cheap).

### Implementation candidates

- **AudioGroupCologne/wavefront** ([github](https://github.com/AudioGroupCologne/wavefront)) — Rust, TLM. Read for the boundary handling and the buffer
  ping-pong pattern.
- **trentfridey/rust-waves** ([github](https://github.com/trentfridey/rust-waves)) — Rust+WASM 2D wave equation simulator. Small reference codebase.
- **MEisebitt/WaveSim** ([github](https://github.com/MEisebitt/WaveSim)) — Rust 2D wave simulation (druid GUI). Tiny.
- **bsxfun/pffdtd** ([github](https://github.com/bsxfun/pffdtd)) — production-grade FDTD for 3D room acoustics, CPU+GPU. The CPU SIMD path is the
  reference implementation we should mirror.
- **nantonel/AcFDTD.jl** ([github](https://github.com/nantonel/AcFDTD.jl)) —
  Julia, 3D. Useful for stability analysis cross-checking.
- **ovcharenkoo/CUDA_FDTD_2D_acoustic_wave_propagation** ([github](https://github.com/ovcharenkoo/CUDA_FDTD_2D_acoustic_wave_propagation)) — GPU
  reference. Stencil code is identical to what CPU SIMD wants.
- **Hilbert curve LUTs:**
  - **morton-encoding** crate ([docs.rs](https://docs.rs/morton-encoding)) for Z-order
  - **spacecurve** ([github](https://github.com/cortesi/spacecurve)) for Hilbert in Rust
  - **eisenwave/hilbert-curves-cpp** ([github](https://github.com/Eisenwave/hilbert-curves-cpp))
    has a clean reference implementation (header-only C++17, 3D — adapt to
    2D).

### Recommendations

1. **Pad the grid to 128x64** (= 8192 cells, one virtual cell at end). Use
   AVX-256 inner loop (8 floats per vector). Total 4 vectors per row × 64
   rows = 256 SIMD ops per hop per channel. Trivially real-time.
2. **Standard 4-neighbour stencil** for v1. If audible "uneven cross-talk" is
   complained about, upgrade to 8/9-neighbour to smooth Hilbert tail jumps.
3. **Per-bin boundary mix between Neumann and Mur first-order ABC.** No PML —
   too much memory and CPU for marginal benefit.
4. **CFL ceiling: `c ≤ 0.65`** in normalized units. 92% of the 1/√2 limit.
   This gives audible high-frequency dispersion without crashing.
5. **Hilbert LUT precomputed at reset.** 16 KB read-only LUT. Use it both for
   energy injection (1D bin → grid cell) and energy extraction (grid cell →
   1D bin). Same LUT, both directions.
6. **Boundary curve mixed Neumann/Mur.** A `[0,1]` per-bin curve where 0 =
   Mur (absorbent), 1 = Neumann (reflective) gives the user clean control
   over "where the spectrum rings vs where it dies."
7. **Tail-handling: if the user wants periodicity, expose it as a separate
   discrete boundary mode (Toroidal), not as a curve value.** Periodic +
   Mur don't mix cleanly.

---

## Cross-topic synthesis

All three topics are instances of the same abstract pattern: **per-hop
explicit time-stepping of a finite-difference operator on the spectrum, with
energy as a sanity invariant.** The differences are which operator, which
state per cell, and which CFL-style bound governs stability.

| Topic | State per cell | Operator | CFL condition | Conservation invariant |
|---|---|---|---|---|
| Springs | (displacement, velocity) | tridiagonal stiffness (chain) plus optional sparse harmonic links | `omega_max * dt < 2` | Hamiltonian (sum of K + V) |
| Diffusion | power | Laplacian (second derivative) | `D * dt / dx^2 < 0.5` | total power |
| 2D wave | (u_curr, u_prev) on 2D grid | 2D Laplacian + leapfrog | `c * dt / dx < 1/sqrt(2)` | discrete energy (½v² + ½(grad u)²) |

**Shared infrastructure opportunities:**

1. **Per-bin energy tracker.** Every operator has an "energy" we can sum.
   A single `[8193]f32` energy buffer plus a single `compute_energy()` per
   slot is enough to apply Dinev-style emergency damping (scale states by
   sqrt(0.5) on suspicious doubling) across all three modules.
2. **Per-bin curve smoothing.** All three need 1-pole hop-rate smoothing on
   per-bin parameter curves to suppress Mathieu instabilities. Single shared
   helper.
3. **Two-buffer ping-pong.** Springs need `(d_curr, v_curr)` and
   `(d_next, v_next)`. Diffusion needs `(p_curr, p_next)`. Wave needs
   `(u_curr, u_prev, u_next)`. The same `Vec::swap` pattern works for all.
4. **CFL clamping helper.** Each integrator needs to clamp its parameter
   curve against a hop-rate-dependent ceiling. Centralise as
   `clamp_for_cfl(value: f32, ceiling: f32) -> f32`.
5. **Hilbert LUT.** Used by 2D wave, potentially by Chladni Plate
   eigenmode patches. Build once per FFT-size, share across slots.

**Single shared kernel pattern.** All three updates fit:
```rust
fn update<S: SoaState, F: FnMut(&mut S, &S, usize)>(
    state_curr: &mut S,
    state_prev: &S,
    update: F,
) {
    for i in 0..NUM_BINS {  // SIMD-vectorised by compiler
        update(state_curr, state_prev, i);
    }
}
```
Where `S` is `(d, v)` for springs, `p` for diffusion, `u` for waves. The
compile-time monomorphisation gives identical SIMD assembly to hand-rolled
versions.

**One critical shared concern: interaction with the existing curve modulation
pipeline.** All three modules will see per-hop parameter changes via the
existing `curve_rx[slot][curve].read()` mechanism. The 1-pole smoothing
between curve-read and integrator-evaluation is the single most important
stability gate. If skipped, *all three modules will exhibit hop-rate-aliased
instabilities* (Mathieu tongues for springs, conservation drift for diffusion,
parametric ringing for waves). It must be done.

**Patent safety check.** None of these methods (Verlet, leapfrog, FTCS, FDTD,
Mur ABC, Hilbert curves) are patent-encumbered. They're textbook
finite-difference / molecular-dynamics / PDE-numerics methods from the 1960s
through 1990s. No oeksound-Hilbert-transform overlap. No spectral-modeling
patent overlap.

---

## Open questions

1. **Mathieu instability empirical sweep.** The Mathieu prediction is well-
   founded but the actual artefact magnitude depends on damping ratio and
   curve modulation depth. We should run a synthetic test: hold a sustained
   sine, sweep stiffness modulation depth from 0 to 100% at a hop-rate
   harmonic, listen for the parametric "ring up." Document the user-facing
   safe modulation depth.

2. **Hilbert tail-jump audibility.** The theoretical "long-tail neighbours
   100s of bins apart" prediction needs to be verified perceptually. If the
   tail is audible as "spectral leaping artefacts," 8-neighbour stencil is
   needed. If sub-threshold, 4-neighbour saves CPU.

3. **Diffusion vs Lattice Boltzmann blind A/B.** I claim FTCS is enough.
   Worth a 30-minute listening test on a sustained chord with diffusion
   active to confirm the LBM/finite-volume difference is really sub-threshold.
   If it isn't, reconsider.

4. **XPBD with Jacobi sweep for stiff springs.** XPBD's compliance trick is
   genuinely time-step-independent. The standard XPBD is Gauss-Seidel
   (sequential, kills SIMD). Jacobi-style XPBD is rarely discussed in
   literature — would have to derive convergence behaviour ourselves. If we
   ever want springs *stiffer than the CFL bound allows for Verlet*, this is
   the only path. Not needed for v1.

5. **Sparse harmonic-springs cap=8 audibility.** If a user wants bin 1
   connected to *all* 8192 of its harmonics, cap=8 obviously truncates. At
   what audible threshold does the truncation matter? Suspect inaudibly past
   the 5th harmonic for most material. Confirm in listening.

6. **Energy clamp side effects.** The "scale by sqrt(0.5) on suspicious
   energy doubling" safety net is a clean stability guarantee but it's also
   audible if it triggers on legitimate transients. Need a hysteresis: only
   trigger if energy doubles in 2 consecutive hops, not 1. And only on
   springs/wave modules, not diffusion (which can't blow up if D is clamped).

7. **Two-step substepping for "ringing" stiffness band.** The CFL bound
   limits the user's "max spring frequency" to ~50 Hz at hop=512. Many
   physical springs producers want are above that (drum-skin springs at
   200 Hz). Substepping just the springs module by 8x (cost: 8x O(N) work in
   that one module = still cheap at 8193 bins) lifts the bound to 400 Hz at
   the cost of some CPU. v2 feature; v1 ship with hop-locked CFL ceiling and
   document.

8. **Are diffusion and wave equation the same module under the hood?** The
   diffusion update IS the wave update with `u_prev = u_curr` — i.e., second
   time derivative replaced by first. Could implement both as one
   `FdtdScheme` enum and share the inner loop. Architectural cleanness
   question.
