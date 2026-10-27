//! Fitting a saved window frame to what a display can show before a
//! window is asked for it. A frame saved on a larger setup can be
//! bigger than any attached display and bigger than the GPU will make a
//! surface; wgpu rejects such a surface with an error that, unhandled,
//! panics inside an `AppKit` callback and aborts the app. Pure: the
//! displays come in as data.

use super::model::WindowFrame;

/// The largest surface side, in device pixels, any Metal device makes.
/// Every Apple GPU family supports at least this, so it needs no device.
pub const MAX_SURFACE_PIXELS: i32 = 8192;

/// The title bar's height in native points. A frame keeps the outer
/// origin and the inner size, so a window that fills a visible frame is
/// one title bar shorter inside than the frame is tall. Measured on
/// macOS 26 as the visible height less the inner height macOS settled a
/// full-height window at.
pub const TITLE_BAR: i32 = 32;

/// A rectangle in whole native points, origin at the primary display's
/// top-left, y down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl Area {
    fn contains(&self, x: i32, y: i32) -> bool {
        self.x <= x && x <= self.x + self.w && self.y <= y && y <= self.y + self.h
    }
}

/// One attached display: its name, its whole frame, the part the menu
/// bar and Dock leave free, and how many device pixels make a point.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// The name saved on a frame to say which display it was on.
    pub name: String,
    /// The whole display.
    pub frame: Area,
    /// The part the menu bar and Dock leave free.
    pub visible: Area,
    /// Device pixels per point.
    pub scale: f64,
}

/// `frame` cut to `MAX_SURFACE_PIXELS / scale` points a side and to the
/// visible frame of its display, and moved inside that frame. The
/// display is the one the frame names, else the one holding its centre,
/// else the first. With no display only the size is capped, at scale 1.
/// Fitting a fitted frame changes nothing.
#[must_use]
pub fn fit(frame: &WindowFrame, displays: &[Display]) -> WindowFrame {
    let (cx, cy) = (frame.x + frame.w / 2, frame.y + frame.h / 2);
    let display = displays
        .iter()
        .find(|d| !frame.monitor.is_empty() && d.name == frame.monitor)
        .or_else(|| displays.iter().find(|d| d.frame.contains(cx, cy)))
        .or_else(|| displays.first());
    let cap = surface_cap(display.map_or(1.0, |d| d.scale));
    let mut fitted = WindowFrame {
        w: frame.w.min(cap).max(1),
        h: frame.h.min(cap).max(1),
        ..frame.clone()
    };
    if let Some(d) = display {
        let v = d.visible;
        fitted.w = fitted.w.min(v.w).max(1);
        fitted.h = fitted.h.min(v.h - TITLE_BAR).max(1);
        // `max` before `min` keeps the left and top edges on screen when
        // the window is still wider or taller than the visible frame.
        fitted.x = fitted.x.min(v.x + v.w - fitted.w).max(v.x);
        fitted.y = fitted.y.min(v.y + v.h - TITLE_BAR - fitted.h).max(v.y);
    }
    fitted
}

/// The frame a window opens at with nothing saved: the primary display's
/// visible frame, as a size the app chose (and so can bound) rather than
/// the system's zoomed state. `None` when no display is listed.
#[must_use]
pub fn first_launch(displays: &[Display]) -> Option<WindowFrame> {
    let primary = displays.first()?;
    let v = primary.visible;
    let frame = WindowFrame {
        x: v.x,
        y: v.y,
        w: v.w,
        h: v.h - TITLE_BAR,
        monitor: primary.name.clone(),
    };
    Some(fit(&frame, displays))
}

/// Points a side that stay under the surface limit at `scale`; scale 1
/// when the scale is not a usable number.
#[allow(clippy::cast_possible_truncation)]
fn surface_cap(scale: f64) -> i32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (f64::from(MAX_SURFACE_PIXELS) / scale).floor() as i32
}
