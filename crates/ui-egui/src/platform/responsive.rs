//! Viewport breakpoints, touch-device detection, dynamic theme token overrides and stationary
//! long-press wake scheduling for mobile and tablet layouts.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft::theme::{AppSizes, SizeValue};

/// Minimum window width where desktop multi-panel layout stays unaltered (matches `MIN_WINDOW_SIZE[0]`).
const DESKTOP_MIN_WIDTH: f32 = 480.0;
/// Tablet width below which dock panels switch to single-panel-at-a-time / compact mode on touch.
const TOUCH_COMPACT_WIDTH: f32 = 768.0;
/// Phone width below which the floating quick toolbar auto-hides behind a toggle button on touch.
const TOUCH_PHONE_TOOLBAR_WIDTH: f32 = 560.0;
/// Width below which left `Tools` and a `RightPanel` are mutually exclusive on touch devices.
const TOUCH_SINGLE_PANEL_WIDTH: f32 = 960.0;
/// Shortest-side threshold below which touch input switches controls to enlarged touch targets,
/// keeping wide desktop harnesses with synthetic touch events at desktop metrics.
const TOUCH_SIZING_MIN_SIDE: f32 = 860.0;
/// Stationary hold time after which egui synthesizes a secondary context-click on touch.
const LONG_PRESS_DURATION_SECS: f32 = 0.5;
const LONG_PRESS_WAKE_DELAY: Duration = Duration::from_millis(520);
const TOUCH_CLICK_SLOP: f32 = 12.0;
const DESKTOP_CLICK_SLOP: f32 = 6.0;
const DESKTOP_CLICK_DURATION: f32 = 0.6;

/// Quantized responsive layout profile driving cached [`AppSizes`] overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResponsiveProfile {
    touch_sizing: bool,
    page_inset_tier: u8,
    dialog_max_q8: u16,
    dock_col_q8: u16,
}

impl ResponsiveProfile {
    /// Derives the quantized sizing profile for `viewport` under the current pointer state.
    pub(crate) fn compute(viewport: egui::Vec2, coarse_pointer: bool, touch_input: bool) -> Self {
        let width = if viewport.x.is_finite() && viewport.x > 0.0 { viewport.x } else { 1280.0 };
        let height = if viewport.y.is_finite() && viewport.y > 0.0 { viewport.y } else { 800.0 };
        let touch_mode = coarse_pointer || touch_input;
        let touch_sizing = coarse_pointer || width < DESKTOP_MIN_WIDTH || (touch_input && width.min(height) < TOUCH_SIZING_MIN_SIDE);
        let page_inset_tier = if width < DESKTOP_MIN_WIDTH || (touch_mode && width < TOUCH_PHONE_TOOLBAR_WIDTH) {
            2
        } else if touch_mode && width < 900.0 {
            1
        } else {
            0
        };
        let dialog_max_q8 = if width < 500.0 { quantize_8((width - 24.0).clamp(120.0, 440.0)) } else { 0 };
        let dock_col_q8 =
            if width < DESKTOP_MIN_WIDTH || (touch_mode && width < 900.0) { quantize_8((width * 0.62).clamp(180.0, 340.0)) } else { 0 };
        Self { touch_sizing, page_inset_tier, dialog_max_q8, dock_col_q8 }
    }

    /// Builds [`AppSizes`] overrides strictly within craft-theme's `sizes::leaves` bounds.
    pub(crate) fn build_sizes(self) -> AppSizes {
        let mut sizes = AppSizes::default();
        sizes.all.insert("typography.display.size".into(), SizeValue::Number(24.0));
        sizes.all.insert("typography.display.line_height".into(), SizeValue::Number(29.0));
        if self.touch_sizing {
            apply_touch_target_sizes(&mut sizes);
        }
        apply_page_inset_tier(&mut sizes, self.page_inset_tier);
        if self.dialog_max_q8 > 0 {
            let max = f32::from(self.dialog_max_q8);
            sizes.all.insert("size.dialog.least".into(), SizeValue::Number(max.min(380.0)));
            sizes.all.insert("size.dialog.wrap".into(), SizeValue::Number(max.min(440.0)));
            if max < 260.0 {
                sizes.all.insert("size.popup_min".into(), SizeValue::Number(max.clamp(64.0, 220.0)));
            }
        }
        if self.dock_col_q8 > 0 {
            let col = f32::from(self.dock_col_q8).clamp(120.0, 640.0);
            sizes.all.insert("size.dock.column".into(), SizeValue::Number(col));
        }
        sizes
    }
}

fn quantize_8(value: f32) -> u16 {
    let steps = (value / 8.0).floor().clamp(0.0, 4096.0) as u16;
    steps.saturating_mul(8)
}

fn apply_touch_target_sizes(sizes: &mut AppSizes) {
    const TOUCH_TOKENS: &[(&str, f32)] = &[
        ("size.control.sm", 32.0),
        ("size.control.md", 38.0),
        ("size.icon.md", 20.0),
        ("size.bar.title", 44.0),
        ("size.bar.tab", 38.0),
        ("size.bar.options", 46.0),
        ("size.bar.options_narrow", 44.0),
        ("size.panel_header", 40.0),
        ("size.rail.width", 52.0),
        ("size.rail.button", 40.0),
        ("size.handle.sm", 10.0),
        ("size.handle.md", 14.0),
        ("size.handle.ring", 28.0),
        ("size.hit.slop", 8.0),
        ("size.hit.target", 28.0),
        ("size.hit.edge", 16.0),
    ];
    for &(key, value) in TOUCH_TOKENS {
        sizes.all.insert(key.into(), SizeValue::Number(value));
    }
}

fn apply_page_inset_tier(sizes: &mut AppSizes, tier: u8) {
    match tier {
        1 => {
            sizes.all.insert("space.inset.page_side".into(), SizeValue::Number(20.0));
            sizes.all.insert("space.inset.page_margin".into(), SizeValue::Number(14.0));
            sizes.all.insert("space.inset.screen".into(), SizeValue::Number(16.0));
        }
        2 => {
            sizes.all.insert("space.inset.page_side".into(), SizeValue::Number(8.0));
            sizes.all.insert("space.inset.page_margin".into(), SizeValue::Number(8.0));
            sizes.all.insert("space.inset.page_gap".into(), SizeValue::Number(8.0));
            sizes.all.insert("space.inset.screen".into(), SizeValue::Number(12.0));
        }
        _ => {}
    }
}

/// Detects whether the browser matches `(pointer: coarse)` at startup on WebAssembly.
#[cfg(target_arch = "wasm32")]
fn detect_coarse_pointer() -> bool {
    web_sys::window().and_then(|w| w.match_media("(pointer: coarse)").ok().flatten()).is_some_and(|m| m.matches())
}

#[cfg(not(target_arch = "wasm32"))]
fn detect_coarse_pointer() -> bool {
    false
}

/// Tracks active touch/mouse mode, responsive panel auto-collapse transitions and cached theme sizes.
pub(crate) struct ResponsiveState {
    coarse_pointer: bool,
    touch_input: bool,
    touch_options_applied: bool,
    active_touches: HashSet<egui::TouchId>,
    pub(crate) compact_applied: Option<bool>,
    pub(crate) tools_auto_collapsed: bool,
    pub(crate) phone_toolbar_applied: Option<bool>,
    pub(crate) toolbar_auto_hidden: bool,
    pub(crate) tools_compact_width: Option<f32>,
    pub(crate) last_viewport_width: f32,
    cached_sizes: Mutex<Option<(ResponsiveProfile, Arc<AppSizes>)>>,
}

impl Default for ResponsiveState {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponsiveState {
    /// Creates initial responsive state from the host environment.
    pub(crate) fn new() -> Self {
        let coarse = detect_coarse_pointer();
        Self {
            coarse_pointer: coarse,
            touch_input: coarse,
            touch_options_applied: false,
            active_touches: HashSet::new(),
            compact_applied: None,
            tools_auto_collapsed: false,
            phone_toolbar_applied: None,
            toolbar_auto_hidden: false,
            tools_compact_width: None,
            last_viewport_width: 1280.0,
            cached_sizes: Mutex::new(None),
        }
    }

    /// Inspects raw input events to track touch vs mouse mode and schedule stationary long-press repaints.
    pub(crate) fn on_raw_input(&mut self, ctx: &egui::Context, raw: &egui::RawInput) {
        let (mut saw_touch, mut saw_touch_start, mut saw_mouse) = (false, false, false);
        for event in &raw.events {
            match event {
                egui::Event::Touch { id, phase, .. } => {
                    saw_touch = true;
                    match phase {
                        egui::TouchPhase::Start => {
                            saw_touch_start = true;
                            self.active_touches.insert(*id);
                        }
                        egui::TouchPhase::Move => {
                            self.active_touches.insert(*id);
                        }
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                            self.active_touches.remove(id);
                        }
                    }
                }
                egui::Event::WindowFocused(false) => self.active_touches.clear(),
                egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. } => saw_mouse = true,
                _ => {}
            }
        }
        if saw_touch {
            self.touch_input = true;
        } else if saw_mouse && self.active_touches.is_empty() {
            self.touch_input = false;
        }
        if self.touch_input != self.touch_options_applied {
            self.touch_options_applied = self.touch_input;
            ctx.options_mut(|o| {
                if self.touch_input {
                    o.input_options.max_click_dist = TOUCH_CLICK_SLOP;
                    o.input_options.max_click_duration = LONG_PRESS_DURATION_SECS;
                } else {
                    o.input_options.max_click_dist = DESKTOP_CLICK_SLOP;
                    o.input_options.max_click_duration = DESKTOP_CLICK_DURATION;
                }
            });
        }
        if saw_touch_start && self.active_touches.len() == 1 {
            // Wake egui even when the finger stays completely still so long-press context clicks fire on time.
            ctx.request_repaint_after(LONG_PRESS_WAKE_DELAY);
        }
    }

    /// True when the latest pointer interaction used touch (or the device started with a coarse pointer).
    pub(crate) fn touch_input(&self) -> bool {
        self.touch_input
    }

    /// True on touch-primary devices or when touch input is currently active.
    pub(crate) fn touch_mode(&self) -> bool {
        self.coarse_pointer || self.touch_input
    }

    /// True when the Tools pane should default to collapsed to preserve reading space.
    pub(crate) fn compact_panels(&self, width: f32) -> bool {
        width.is_finite() && (width < DESKTOP_MIN_WIDTH || (self.touch_mode() && width < TOUCH_COMPACT_WIDTH))
    }

    /// True when the floating quick toolbar should auto-hide on phone widths.
    pub(crate) fn phone_toolbar(&self, width: f32) -> bool {
        width.is_finite() && (width < DESKTOP_MIN_WIDTH || (self.touch_mode() && width < TOUCH_PHONE_TOOLBAR_WIDTH))
    }

    /// True when opening Tools and a RightPanel simultaneously would crush the document canvas.
    pub(crate) fn single_side_panel(&self, width: f32) -> bool {
        width.is_finite() && (width < DESKTOP_MIN_WIDTH || (self.touch_mode() && width < TOUCH_SINGLE_PANEL_WIDTH))
    }

    /// Returns cached [`AppSizes`] overrides for the current viewport and pointer profile.
    pub(crate) fn app_sizes(&self, viewport: egui::Vec2) -> Arc<AppSizes> {
        let profile = ResponsiveProfile::compute(viewport, self.coarse_pointer, self.touch_input);
        if let Ok(mut slot) = self.cached_sizes.lock() {
            if let Some((cached, sizes)) = slot.as_ref()
                && *cached == profile
            {
                return Arc::clone(sizes);
            }
            let sizes = Arc::new(profile.build_sizes());
            *slot = Some((profile, Arc::clone(&sizes)));
            return sizes;
        }
        Arc::new(profile.build_sizes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_event(id: u64, phase: egui::TouchPhase) -> egui::Event {
        egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: egui::pos2(100.0, 200.0), force: None }
    }

    #[test]
    fn generated_app_sizes_pass_theme_validation_for_all_viewports() {
        let viewports = [
            egui::vec2(320.0, 568.0),
            egui::vec2(375.0, 667.0),
            egui::vec2(390.0, 844.0),
            egui::vec2(480.0, 520.0),
            egui::vec2(768.0, 1024.0),
            egui::vec2(1024.0, 768.0),
            egui::vec2(1400.0, 900.0),
            egui::vec2(f32::NAN, 0.0),
        ];
        for vp in viewports {
            for coarse in [false, true] {
                for touch in [false, true] {
                    let profile = ResponsiveProfile::compute(vp, coarse, touch);
                    let sizes = profile.build_sizes();
                    let json = sizes.to_json();
                    AppSizes::parse(json.as_bytes()).unwrap_or_else(|e| panic!("invalid AppSizes for vp={vp:?} coarse={coarse} touch={touch}: {e}"));
                }
            }
        }
    }

    #[test]
    fn desktop_mouse_and_mixed_desktop_harness_invariants_hold() {
        let mut state = ResponsiveState::new();
        // Desktop at MIN_WINDOW_SIZE (480x520) with mouse keeps desktop margins and multi-panel rules.
        let min_desktop = ResponsiveProfile::compute(egui::vec2(480.0, 520.0), false, false);
        assert!(!min_desktop.touch_sizing);
        assert_eq!(min_desktop.page_inset_tier, 0);
        assert_eq!(min_desktop.dock_col_q8, 0);
        assert!(!state.compact_panels(480.0));
        assert!(!state.single_side_panel(480.0));

        // 1400x900 desktop harness sending a touch tap keeps desktop control sizes (no mid-frame height jump).
        let mixed = ResponsiveProfile::compute(egui::vec2(1400.0, 900.0), false, true);
        assert!(!mixed.touch_sizing);
        assert_eq!(mixed.page_inset_tier, 0);

        // Phone and coarse tablet get enlarged touch controls and reduced page margins.
        let phone = ResponsiveProfile::compute(egui::vec2(375.0, 812.0), true, true);
        assert!(phone.touch_sizing);
        assert_eq!(phone.page_inset_tier, 2);
        assert!(phone.dialog_max_q8 > 0 && phone.dialog_max_q8 < 380);
        assert!(phone.dock_col_q8 >= 180 && phone.dock_col_q8 < 300);

        state.touch_input = true;
        assert!(state.compact_panels(390.0));
        assert!(state.phone_toolbar(390.0));
        assert!(state.single_side_panel(768.0));
    }

    #[test]
    fn raw_input_switches_between_touch_and_mouse_cleanly() {
        let ctx = egui::Context::default();
        let mut state = ResponsiveState::new();
        assert!(!state.touch_input());

        let mut start = egui::RawInput::default();
        start.events.push(touch_event(1, egui::TouchPhase::Start));
        state.on_raw_input(&ctx, &start);
        assert!(state.touch_input());
        assert_eq!(state.active_touches.len(), 1);

        // A synthesized mouse move while finger is still down must not drop touch mode.
        let mut moved = egui::RawInput::default();
        moved.events.push(egui::Event::PointerMoved(egui::pos2(100.0, 200.0)));
        state.on_raw_input(&ctx, &moved);
        assert!(state.touch_input());

        let mut end = egui::RawInput::default();
        end.events.push(touch_event(1, egui::TouchPhase::End));
        state.on_raw_input(&ctx, &end);
        assert!(state.touch_input(), "touch mode persists during post-release flick glide");
        assert!(state.active_touches.is_empty());

        // Real mouse motion after touch ended restores mouse mode.
        state.on_raw_input(&ctx, &moved);
        assert!(!state.touch_input());
    }
}
