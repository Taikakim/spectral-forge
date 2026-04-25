> **Status (2026-04-24): DEFERRED.** Not yet implemented. Source of truth: [../STATUS.md](../STATUS.md).

# Matrix Amp Nodes — Design Spec

**Status:** Planned  
**Plan:** (to be written — Plan 2)

## What it is

Every routing-matrix send node becomes an "amp" — an active processing unit on the signal path between two slots. Currently each node is a scalar float (send amplitude). After this change, each node also carries an `AmpMode` that shapes how the signal passes through it.

Default mode is `Linear` (existing behavior: just multiply by the send amount). Clicking a node in the matrix UI opens a small panel where a different mode can be selected.

This is especially powerful with feedback routing: a `Vactrol` mode on a feedback send gives the feedback loop non-linear, photoresistor-style memory.

## AmpMode variants

| Mode | Behavior |
|---|---|
| `Linear` | Pass-through scalar multiply. Default. Zero overhead. |
| `Vactrol` | Fast attack, non-linear slow release (models a Buchla-style opto-isolator). State: one `[f32; MAX_NUM_BINS]` capacitor level per connection. |
| `Schmitt` | Hysteresis gate. Two thresholds: bin stays on until magnitude drops below `off_threshold`, stays off until it rises above `on_threshold`. State: one `[bool; MAX_NUM_BINS]` latch array. |
| `Slew` | Rate-limited output. Magnitude can only change at a maximum rate per hop. State: one `[f32; MAX_NUM_BINS]` current-value array. |
| `Stiction` | Dead-zone: change only propagates once accumulated delta exceeds a threshold. State: one `[f32; MAX_NUM_BINS]` accumulator. |

## Architecture

**RouteMatrix** (GUI side, cloned each block):
```rust
pub amp_mode: [[AmpMode; MAX_SLOTS]; MAX_MATRIX_ROWS]
```

**FxMatrix** (audio side, holds state):
```rust
amp_state: [[AmpNodeState; MAX_SLOTS]; MAX_MATRIX_ROWS]
```

Where `AmpNodeState` is a compact struct with optional state arrays — only allocated for non-Linear modes at the time the user selects the mode (one-time `permit_alloc`).

FxMatrix applies the amp mode to each contributing source signal *before* accumulating it into `mix_buf`. The quick-amount float still applies as normal (it's the send amplitude that feeds the amp).

## UI

- Matrix cell: shows send amount knob as now, plus a small colored indicator dot if mode ≠ Linear.
- Clicking the cell: opens a small inline panel (not a popup) showing the mode selector and mode-specific parameter(s).
- Mode-specific parameters are not curve-driven — they're simple float params stored in `RouteMatrix` alongside `amp_mode`.
