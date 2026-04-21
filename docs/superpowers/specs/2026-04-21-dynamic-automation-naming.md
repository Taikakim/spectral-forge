# Deferred: Dynamic Automation Parameter Naming

**Status:** Deferred — tracked as future work
**Date:** 2026-04-21
**Related:** `2026-04-21-automation-presets-design.md` §2

## The problem

Because slot contents change at user-edit time (user swaps slot 2 from Dynamics → Freeze), the host-visible parameter names are static placeholders (`S2 C0 N3 Y`) rather than the meaningful labels the user sees in the plugin GUI (`Freeze Length · Node 3 · Gain`).

The short-term mitigation is the 1000ms-hover tooltip inside the plugin (shipped in the main plan). That works while the user is looking at the plugin UI, but it does not help when:
- The host shows a parameter-browse dialog (Bitwig's Remote Controls / Modulator target list)
- Automation lanes are labelled in the arrangement view
- The user is reading a preset's automation data outside the plugin

## Why it is not in the main plan

nih-plug's CLAP wrapper only calls `CLAP_PARAM_RESCAN_VALUES` when parameters change — never `CLAP_PARAM_RESCAN_INFO` or `CLAP_PARAM_RESCAN_ALL`, which are the flags that cause the host to re-query the parameter's `name` / `display_text` fields.

Reference: `src/wrapper/clap/wrapper.rs:418` in nih-plug (commit 28b149e). Hard-coded to `VALUES` only.

To make dynamic names work we would need one of:

1. **Fork or patch nih-plug** to emit `RESCAN_INFO` when instructed by the plugin. Expose a `context.rescan_param_info(&[ParamPtr])` API. This is the clean fix.
2. **Bypass nih-plug's CLAP layer** and talk to `clap_host_params` directly via `ClapPlugin::ext_raw_host_params()` — probably not exposed, would need unsafe FFI inside the plugin.
3. **Live with static names** forever and rely entirely on the tooltip (status quo of the main plan).

## Estimated scope for option 1

- Patch `Wrapper<P>` in `wrapper/clap/wrapper.rs` to expose `fn request_rescan_info(&self, flags: u32)`.
- Add a `rescan_param_info(&mut self, ids: &[&'static str])` method on the CLAP `InitContext` / `ProcessContext` trait surface.
- Connect to slot-type change events in the plugin: on `slot_module_types[i]` write, collect the param IDs for that slot and request rescan.
- Add a `fn display_name(&self, ctx: &DisplayContext) -> String` trait method on params (or similar) so the wrapper can ask the plugin for a new string at the moment of `clap_plugin_params_get_info()`.

Rough effort: ~2–4 days including upstream PR work, review, and compatibility testing across Bitwig/Reaper/Ableton.

## Why not upstream first

nih-plug maintainer acceptance of such a feature is uncertain — dynamic param info is a rare request because most plugins have static layouts. Prototype in a fork, confirm Bitwig actually honours `RESCAN_INFO` mid-session (some hosts ignore it), then propose upstream with real validation.

## VST3 considerations

VST3 has `IComponentHandler2::restartComponent(kParamTitlesChanged)` — same concept, different flag. If the CLAP fork path works, extending to VST3 is straightforward: patch the VST3 wrapper at the equivalent point. Reaper is known to support `kParamTitlesChanged`; Bitwig's VST3 host does too.

## Not blocking anything

This feature's absence does not block any DSP or UI work. The 1000ms tooltip in the main plan gives users enough in-plugin feedback to build muscle memory for the static-name scheme. When a user has 10+ plugin instances and needs to sort automation lanes in a song, that is when they will feel the pain — at which point revisit.
