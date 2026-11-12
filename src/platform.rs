//! Keeping the output window above everything on macOS (full-screen spaces
//! would otherwise hide it or show the menu bar).

#[cfg(target_os = "macos")]
#[allow(unsafe_code, unused_unsafe)]
pub fn raise_output_windows(prefix: &str) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindowCollectionBehavior};
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: AppKit calls on the main thread with live window objects.
    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        for w in app.windows().iter() {
            if !w.title().to_string().starts_with(prefix) {
                continue;
            }
            if w.level() < 1000 {
                w.setLevel(1000);
                w.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::Stationary
                        | NSWindowCollectionBehavior::IgnoresCycle,
                );
                w.setHidesOnDeactivate(false);
                w.setHasShadow(false);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn raise_output_windows(_prefix: &str) {}
