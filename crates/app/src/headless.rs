//! Off-screen rendering for visual checks: `SCRAWL_SHOT=out.png [SCRAWL_SHOT_SCENE=draw|graph|tablet|typing]`.
//! Uses Slint's software renderer into memory, so no window, display server or compositor is touched.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Rgb8Pixel};
use std::rc::Rc;

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    start: std::time::Instant,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> std::time::Duration {
        self.start.elapsed()
    }
}

/// Install the in-memory platform. Must run before any window is created.
pub fn install(w: u32, h: u32, scale: f32) -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(w, h));
    if (scale - 1.0).abs() > f32::EPSILON {
        window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale });
    }
    let _ = slint::platform::set_platform(Box::new(Headless { window: window.clone(), start: std::time::Instant::now() }));
    window
}

/// Pump timers/animations briefly, render one frame and save it as a PNG.
pub fn capture(window: &Rc<MinimalSoftwareWindow>, ui: &impl ComponentHandle, path: &str, (w, h): (u32, u32), settle_ms: u64) -> Result<(), String> {
    ui.show().map_err(|e| e.to_string())?;
    let end = std::time::Instant::now() + std::time::Duration::from_millis(settle_ms);
    while std::time::Instant::now() < end {
        slint::platform::update_timers_and_animations();
        window.draw_if_needed(|_| {});
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut buf = vec![Rgb8Pixel::default(); (w * h) as usize];
    window.request_redraw();
    let mut drawn = false;
    window.draw_if_needed(|r| {
        r.render(&mut buf, w as usize);
        drawn = true;
    });
    if !drawn {
        return Err("nothing was rendered".into());
    }
    let mut rgba = Vec::with_capacity(buf.len() * 4);
    for p in &buf {
        rgba.extend_from_slice(&[p.r, p.g, p.b, 255]);
    }
    let pm = tiny_skia::Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(w, h).unwrap()).ok_or("bad buffer")?;
    pm.save_png(path).map_err(|e| e.to_string())
}
