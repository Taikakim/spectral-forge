# Matrix Amp Nodes — Audit

**Existing spec:** `docs/superpowers/specs/2026-04-21-matrix-amp-nodes.md`
**Status:** DEFERRED, not started.
**Source brainstorm:** scattered references in
`ideas_for_the_wonderful_future.txt`, especially the analog-modeled
section.

## What the spec says

Every send cell in the routing matrix becomes an "amp" — an active
processing unit instead of a scalar multiply. Modes: `Linear`, `Vactrol`,
`Schmitt`, `Slew`, `Stiction`. Each non-Linear mode allocates its own
per-bin state array (one-time `permit_alloc!`). State lives in
`FxMatrix.amp_state[row][col]`. UI: cell shows current send amount knob
+ a coloured indicator dot if mode ≠ Linear, click cell to inline-expand
the mode panel.

## What the spec gets right

- Default `Linear` mode = zero overhead. Existing matrix unchanged for
  users who don't touch this.
- State allocated only when a non-Linear mode is selected — keeps idle
  RAM low.
- Same amp modes mirror the Circuit module's analog-component palette,
  so users learn one vocabulary.
- Especially synergistic with **feedback** sends — `Vactrol` on a
  feedback edge in the matrix gives the loop non-linear photoresistor
  memory, which is way cooler than scalar feedback alone.

## What the spec glosses over

### a) Where exactly does the amp run?

Spec says "FxMatrix applies the amp mode to each contributing source
signal *before* accumulating it into `mix_buf`."

But what does "the amp" see — the magnitude only, or the complex bin?
A vactrol or stiction is purely magnitude-domain, but a Schmitt trigger
in complex space could plausibly latch on either magnitude or real-part
crossings. **Decision needed:** define the amp mode as
"magnitude-only — phase is preserved unchanged." Cleaner contract,
matches Circuit module practice.

### b) Sidechain to amp-node routing

The Matrix already supports off-diagonal sends. Could a sidechain bus
*be* a matrix row? That would let a sidechain-modulated Vactrol act as
a per-bin opto-isolator with the sidechain controlling the LED. Currently
sidechain assignment is per-slot, not per-matrix-cell.

This is a useful generalisation but a notable matrix UI complication —
defer to phase 2 of Matrix Amp Nodes work.

### c) Visualization of amp-node state

If a Vactrol cell is processing 8193 bins, what does the cell *show*
the user? The spec says "small coloured indicator dot." But for
debugging / artistic feedback, a tiny per-bin trace (one row of pixels
showing the vactrol's current cap level across the spectrum) inside the
cell would be huge. Costs ~8 ms per redraw at 24 fps for 9×9 = 81
cells × 64 visible pixels — trivial. **Recommendation:** add a "show
amp state" toggle in the matrix header.

### d) Reset semantics

What happens to amp-node state when:
- The user changes a send amplitude? — keep state.
- The user changes the mode? — clear state.
- The user changes FFT size? — clear state.
- A preset loads? — clear, then state warms up over a few hops.

Codify in the spec.

## Additional amp modes worth considering

Beyond the spec's `Linear, Vactrol, Schmitt, Slew, Stiction`:

| Mode | Behaviour | Cost |
|---|---|---|
| `BBD` | Single-stage bucket-brigade delay (1 hop, 1-pole LP per bin). Cheap version of the Circuit BBD. | 1 vec, 1 LP per bin. |
| `Crosstalk` | A small percentage of each bin bleeds into N±1. Cheap PCB-Crosstalk. | 1 vec scratch. |
| `Phase Print-Through` | Amp leaks 5% of the bin's value to a 1-hop delayed write. Cheap Tape Print-Through. | 1 vec ring. |
| `Companding` | Soft-knee compander (e.g. y = sign(x) * abs(x).powf(curve)) per bin. | 0 state. |
| `Ring Mod with Sidechain` | If the amp-node has a sidechain assigned, point-wise multiply with sidechain bin value. | 0 extra state (sc bus is already there). |

The first four are cheap downscale versions of full Circuit module
sub-effects. The advantage of having the same algorithm available *both*
as a slot module *and* as an amp mode is that:

- A slot module with all curves available gives full per-bin shaping
- An amp-node mode is for "I want a hint of vactrol on this *specific
  send*" without burning a whole slot

Most-needed are `BBD`, `Crosstalk`, and `Companding`.

## Architecture fit notes

### `RouteMatrix` size

Today `[[f32; MAX_SLOTS]; MAX_MATRIX_ROWS] = [[f32; 9]; 11]` (8 slots
+ Master + 2 virtual T/S rows). Adding `amp_mode: [[AmpMode; 9]; 11]` =
99 enum cells, ~200 bytes. Cloning per block is still cheap.

### Per-cell state

`AmpNodeState` is `Option<Vec<f32>>` per cell. 99 cells. Most are `None`.
Total RAM contribution if every cell uses one f32 array: ~3 MB. Realistic
worst case: 5-10 active amp nodes = ~300 KB. Fine.

### UI complications

- Matrix cell currently shows a small knob. Adding a status dot is fine.
- The "click to expand" inline panel is the awkward bit — the matrix
  is dense, and an inline panel will overlap neighbouring cells. Two
  options:
  1. **Floating popup** like the module-popup pattern (Kim has used this
     successfully for `module_popup.rs`).
  2. **Side panel** below the matrix, showing details for the currently-
     selected cell.
  Floating popup is more like the existing UX. Pick that.

## CPU class

Per-cell amp nodes are mostly **light**, with `BBD` borderline-medium
(extra LP filter per bin per cell).

## Calibration probe set

For each amp mode, expose:
- `probe_amount_pct` (the amp's effect strength as a percentage)
- `probe_state_at_k` (at the test bin, the amp's current internal state)
- `probe_release_ms` for time-domain modes (Vactrol, Slew)

Per-mode calibration round-trip tests in `tests/amp_nodes.rs`.

## Implementation sequencing

1. Land BinPhysics + Circuit module first. Use the same Vactrol /
   Schmitt / Slew kernels for amp nodes — code reuse.
2. Then Matrix Amp Nodes ships as "the same kernels, but per-edge."
3. UI for amp nodes can be a copy/adapt of the existing module popup.

This means amp nodes are work effort **2** rather than **1+1** — they
ride on top of the Circuit kernels.

## Open questions

1. Does an amp node accumulate state per-channel? Today the matrix
   processes both channels in lock-step (Linked) or two passes
   (Independent). If state is per-channel it doubles RAM per active
   amp; if shared it leaks one channel's history into the other.
   **Recommendation:** per-channel state for Independent / MidSide;
   shared for Linked.
2. Should the existing curve-driven send amplitude (the float in
   `RouteMatrix.send`) modulate the amp's *threshold* / *amount* in
   non-Linear modes, or always pre-multiply the input? Pre-multiplying
   is conservative; post-multiplying gives more interesting interactions.
   **Recommendation:** the existing `send` value is a *post*-amp
   amplitude (controls how much of the amp's output reaches the
   destination), and the amp has its own internal "amount" param.
3. Could amp modes be *automatable*? Today they're discrete enum
   cells. If a user wants to morph between Linear and Vactrol, they
   need either a dual-cell trick (two parallel cells, crossfade) or
   an amp-mode parameter that's continuous along a 1-D family. Skip
   for v1.
