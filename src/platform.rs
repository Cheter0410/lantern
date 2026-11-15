//! Keeping the output window above everything on macOS (full-screen spaces
//! would otherwise hide it or show the menu bar).

/// `frame` is the display the output belongs on: position and size in points,
/// top-left origin, as winit reports monitors (relative to the main display).
#[cfg(target_os = "macos")]
#[allow(unsafe_code, unused_unsafe)]
pub fn raise_output_windows(prefix: &str, frame: [f32; 4]) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSScreen, NSWindowCollectionBehavior};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: AppKit calls on the main thread with live window objects.
    unsafe {
        // AppKit counts from the bottom-left corner of the main display.
        let Some(main) = NSScreen::screens(mtm).firstObject() else {
            return;
        };
        let [x, y, w, h] = frame.map(f64::from);
        let want = NSRect::new(
            NSPoint::new(x, main.frame().size.height - y - h),
            NSSize::new(w, h),
        );
        let app = NSApplication::sharedApplication(mtm);
        for win in app.windows().iter() {
            if !win.title().to_string().starts_with(prefix) {
                continue;
            }
            if win.level() < 1000 {
                win.setLevel(1000);
                win.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::Stationary
                        | NSWindowCollectionBehavior::IgnoresCycle,
                );
                win.setHidesOnDeactivate(false);
                win.setHasShadow(false);
            }
            // When the window is shown, AppKit keeps it below the menu bar: it
            // comes out shifted down and shortened, leaving a strip of desktop
            // at the top. Setting the frame directly is not constrained.
            let f = win.frame();
            let off = (f.origin.x - want.origin.x).abs()
                + (f.origin.y - want.origin.y).abs()
                + (f.size.width - want.size.width).abs()
                + (f.size.height - want.size.height).abs();
            if off > 0.5 {
                win.setFrame_display(want, true);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn raise_output_windows(_prefix: &str, _frame: [f32; 4]) {}
