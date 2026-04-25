> **Status (2026-04-24): SUPERSEDED.** The Dynamics/Effects/Harmonic tab bar was removed when the per-slot module UI landed in plan D2. Do not follow this plan. Source of truth: the code + [../STATUS.md](../STATUS.md).

# GUI Tabs Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 3-tab structure to the editor (Dynamics / Effects / Harmonic) so every subsequent engine and feature has a designated home without reorganising the UI again.

**Architecture:** A persisted `active_tab` GUI-state field selects which of three tab panels is rendered in place of the curve area + control strip. Tab 0 is the existing Dynamics content (unchanged). Tabs 1 and 2 are empty placeholder panels that later plans fill in. Tab buttons live in the top bar between the curve selectors and the dB range controls, separated by a vertical divider.

**Tech Stack:** Rust, nih-plug, egui (nih_plug_egui), parking_lot Mutex

---

## File Structure

| File | Change |
|------|--------|
| `src/params.rs` | Add `active_tab: Arc<Mutex<u8>>` persisted GUI-state field |
| `src/editor_ui.rs` | Add tab button row; wrap existing body in tab-0 branch; add empty tab-1 and tab-2 panels |

No new files. No DSP changes.

---

### Task 1: Add `active_tab` to params

**Files:**
- Modify: `src/params.rs`

- [ ] **Step 1: Read `src/params.rs`**

Open the file. Find the block of three `Arc<Mutex<f32>>` GUI-state fields added recently (`graph_db_min`, `graph_db_max`, `peak_falloff_ms`). The new field goes immediately after `active_curve`.

- [ ] **Step 2: Add the field to the struct**

In `SpectralForgeParams`, after:
```rust
#[persist = "active_curve"]
pub active_curve: Arc<Mutex<u8>>,
```

Add:
```rust
#[persist = "active_tab"]
pub active_tab: Arc<Mutex<u8>>,   // 0 = Dynamics, 1 = Effects, 2 = Harmonic
```

- [ ] **Step 3: Initialize it in `Default`**

In `impl Default for SpectralForgeParams`, after `active_curve: Arc::new(Mutex::new(0)),` add:
```rust
active_tab: Arc::new(Mutex::new(0)),
```

- [ ] **Step 4: Verify compile**

```bash
cargo build 2>&1 | grep -E "^error"
```
Expected: no output (no errors).

- [ ] **Step 5: Commit**

```bash
git add src/params.rs
git commit -m "feat: add active_tab persisted GUI state param"
```

---

### Task 2: Add tab buttons to the top bar

**Files:**
- Modify: `src/editor_ui.rs`

The top bar currently has:
1. 7 curve selector buttons
2. Floor / Ceil / Falloff drag-values

We add 3 tab buttons between them, separated by a vertical rule drawn with `ui.separator()` (or a manual line).

- [ ] **Step 1: Read `src/editor_ui.rs`**

Locate the `ui.horizontal(|ui| { ... })` block that draws the top bar (around line 48). The curve selector loop ends at roughly line 67, followed by `ui.add_space(12.0)` then the Floor label.

- [ ] **Step 2: Read `active_tab` at the top of the frame closure**

At the start of the frame closure body (right after the `active_idx` / `sr` / `db_min` / `db_max` / `falloff` reads, around line 38), add:
```rust
let active_tab = *params.active_tab.lock() as usize;
```

- [ ] **Step 3: Insert tab buttons into the horizontal bar**

After the curve selector loop ends (after the closing `}` of the `for (i, label)` loop), and before the `ui.add_space(12.0)` that precedes the Floor label, insert:

```rust
// Vertical divider
ui.add_space(8.0);
ui.separator();
ui.add_space(4.0);

// Tab buttons
const TAB_LABELS: [&str; 3] = ["DYNAMICS", "EFFECTS", "HARMONIC"];
for (t, &tab_label) in TAB_LABELS.iter().enumerate() {
    let is_active = active_tab == t;
    let (fill, text_color) = if is_active {
        (th::BORDER, th::BG)
    } else {
        (th::BG, th::LABEL_DIM)
    };
    let btn = egui::Button::new(
        egui::RichText::new(tab_label).color(text_color).size(10.0),
    )
    .fill(fill)
    .stroke(egui::Stroke::new(th::STROKE_BORDER, th::BORDER));
    if ui.add(btn).clicked() {
        *params.active_tab.lock() = t as u8;
    }
}

ui.add_space(8.0);
ui.separator();
```

- [ ] **Step 4: Verify compile**

```bash
cargo build 2>&1 | grep -E "^error"
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: add DYNAMICS/EFFECTS/HARMONIC tab buttons to top bar"
```

---

### Task 3: Gate the curve area and control strip to Tab 0

**Files:**
- Modify: `src/editor_ui.rs`

Currently the entire body (curve area + control strip) renders unconditionally. It should only render when `active_tab == 0`.

- [ ] **Step 1: Locate the body block**

Find the horizontal divider line painted just after the top bar (around line 111):
```rust
ui.add_space(2.0);
{
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment( ... );
}
```
Everything from this divider to the end of the `show` closure is the body. It needs to be wrapped.

- [ ] **Step 2: Wrap everything after the divider in `if active_tab == 0 { ... }`**

The structure should be:

```rust
// horizontal divider (keep unconditional — always drawn)
ui.add_space(2.0);
{
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [r.left_top(), r.right_top()],
        egui::Stroke::new(th::STROKE_BORDER, th::BORDER),
    );
}

if active_tab == 0 {
    // ── Curve area ─────────────────────────────────────────────
    // ... (all existing curve area code unchanged) ...

    // ── Control strip ──────────────────────────────────────────
    // ... (all existing control strip code unchanged) ...
} else if active_tab == 1 {
    // Effects tab — placeholder
    let avail = ui.available_rect_before_wrap();
    ui.allocate_rect(avail, egui::Sense::hover());
    ui.painter().text(
        avail.center(),
        egui::Align2::CENTER_CENTER,
        "Effects — coming soon",
        egui::FontId::proportional(14.0),
        th::LABEL_DIM,
    );
} else {
    // Harmonic tab — placeholder
    let avail = ui.available_rect_before_wrap();
    ui.allocate_rect(avail, egui::Sense::hover());
    ui.painter().text(
        avail.center(),
        egui::Align2::CENTER_CENTER,
        "Harmonic — coming soon",
        egui::FontId::proportional(14.0),
        th::LABEL_DIM,
    );
}
```

**Important:** The existing code inside the `if active_tab == 0` branch is copied verbatim — do not change any of it. Only the surrounding structure changes.

- [ ] **Step 3: Build and confirm no errors**

```bash
cargo build 2>&1 | grep -E "^error"
```
Expected: no errors.

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -10
```
Expected: all tests pass (no DSP logic changed).

- [ ] **Step 5: Commit**

```bash
git add src/editor_ui.rs
git commit -m "feat: gate curve area to Dynamics tab; add Effects and Harmonic placeholders"
```

---

### Task 4: Push

- [ ] **Step 1: Push to GitHub**

```bash
git push origin master
```

---

## Self-Review Checklist

- [x] **Spec coverage:** active_tab param ✓, 3 tab buttons ✓, Dynamics tab shows existing content ✓, Effects/Harmonic show placeholders ✓
- [x] **No placeholders in code:** all code blocks are complete and literal
- [x] **Type consistency:** `active_tab` is `u8` in Mutex, cast to `usize` for indexing — consistent throughout
- [x] **No DSP changes:** pipeline, engines, bridge untouched — tests cannot regress
- [x] **Cursor tooltip guard:** the tooltip code inside `if active_tab == 0` already checks `curve_rect.contains(hover)` — no separate guard needed
