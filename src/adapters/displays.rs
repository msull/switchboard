//! The attached displays with their backing scale, for fitting window
//! frames. The geometry comes from promptbox's screen list; the scale
//! from `NSScreen`, read in the same order.
//!
//! Empty off macOS and off the main thread, where the fit falls back to
//! scale 1 and the surface limit alone.

use promptbox::adapters::screens::{Screen, screens};

use crate::core::window_fit::{Area, Display};

/// Every attached display, primary first. Call from the main thread.
#[must_use]
pub fn displays() -> Vec<Display> {
    #[cfg(target_os = "macos")]
    let scales = macos::scales();
    #[cfg(not(target_os = "macos"))]
    let scales = Vec::new();
    with_scales(screens(), &scales)
}

/// Each screen with the scale at its index when the two lists line up;
/// scale 1 for all when they do not (a display came or went between the
/// two reads).
fn with_scales(screens: Vec<Screen>, scales: &[f64]) -> Vec<Display> {
    let matched = screens.len() == scales.len();
    screens
        .into_iter()
        .enumerate()
        .map(|(i, s)| Display {
            name: s.name,
            frame: area(s.frame),
            visible: area(s.visible),
            scale: if matched { scales[i] } else { 1.0 },
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
fn area(r: egui::Rect) -> Area {
    Area {
        x: r.min.x.round() as i32,
        y: r.min.y.round() as i32,
        w: r.width().round() as i32,
        h: r.height().round() as i32,
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    /// Backing scale of each screen, in `NSScreen::screens` order, which
    /// is the order promptbox lists them in.
    pub fn scales() -> Vec<f64> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Vec::new();
        };
        NSScreen::screens(mtm)
            .iter()
            .map(|s| s.backingScaleFactor())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};

    fn screen(name: &str) -> Screen {
        let frame = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1512.4, 982.0));
        Screen {
            name: name.into(),
            frame,
            visible: frame,
        }
    }

    #[test]
    fn scales_pair_up_by_index_only_when_the_lists_match() {
        let both = with_scales(vec![screen("a"), screen("b")], &[2.0, 1.0]);
        assert!((both[0].scale - 2.0).abs() < f64::EPSILON);
        assert!((both[1].scale - 1.0).abs() < f64::EPSILON);
        assert_eq!(both[0].frame.w, 1512, "rounded to whole points");
        let short = with_scales(vec![screen("a"), screen("b")], &[2.0]);
        assert!(short.iter().all(|d| (d.scale - 1.0).abs() < f64::EPSILON));
        assert!(with_scales(vec![], &[2.0]).is_empty());
    }
}
