//! Raylib off-screen schematic renderer (spec D3 and 9.2).
//!
//! Following Meshcraft, Raylib runs on its own thread with a hidden window and
//! draws into a `RenderTexture2D`. The pixels are read back and handed to the
//! UI, which uploads them into an egui texture. The UI describes what to draw
//! as a [`DisplayList`] in physical pixels; this crate owns no editing logic
//! and no hit testing.
//!
//! Lines are anti-aliased by drawing at twice the size and downsampling on
//! the GPU with bilinear filtering (2×2 supersampling) before the read-back.

use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// RGBA colour, not premultiplied.
pub type Rgba = [u8; 4];
/// A point in physical pixels.
pub type Px = [f32; 2];

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A drawing command in physical pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        a: Px,
        b: Px,
        width: f32,
        color: Rgba,
    },
    Polyline {
        points: Vec<Px>,
        width: f32,
        color: Rgba,
        closed: bool,
    },
    /// Filled polygon (convex, or star-shaped about its first point).
    Polygon { points: Vec<Px>, color: Rgba },
    Circle {
        c: Px,
        r: f32,
        color: Rgba,
        fill: bool,
        width: f32,
    },
    Rect {
        min: Px,
        max: Px,
        color: Rgba,
        fill: bool,
        width: f32,
    },
    /// Radial glow from `inner` at the centre to transparent at `r`.
    Glow { c: Px, r: f32, inner: Rgba },
    /// Text; `at` is the left/centre/right point of the vertical middle.
    Text {
        at: Px,
        text: String,
        size: f32,
        color: Rgba,
        align: Align,
    },
}

/// Everything needed to draw one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    pub width: u32,
    pub height: u32,
    pub clear: Rgba,
    pub shapes: Vec<Shape>,
    /// Caller's tag, returned with the frame.
    pub revision: u64,
}

/// A rendered frame, RGBA rows top to bottom.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub revision: u64,
    /// Time spent drawing and reading back, in milliseconds.
    pub render_ms: f32,
    /// Time spent on the read-back alone, in milliseconds.
    pub readback_ms: f32,
}

/// Renderer thread state.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Starting,
    Ready,
    /// Raylib could not start (no OpenGL); the caller should draw itself.
    Failed(String),
}

/// Handle to the off-screen renderer thread.
pub struct Renderer {
    tx: Sender<Option<DisplayList>>,
    frame: Arc<Mutex<Option<Frame>>>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
}

/// Characters preloaded into the font atlas besides printable ASCII.
const EXTRA_GLYPHS: &str = "ΩµμπΔ°±→←↑↓·–—…×≈≤≥√∞";

impl Renderer {
    /// Start the renderer with a TrueType font (embedded bytes).
    pub fn spawn(font: &'static [u8]) -> Self {
        let (tx, rx) = bounded::<Option<DisplayList>>(4);
        let frame = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(Status::Starting));
        let (f2, s2) = (frame.clone(), status.clone());
        let thread = std::thread::Builder::new()
            .name("electrocraft-raylib-offscreen".into())
            .spawn(move || {
                let s3 = s2.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    run(rx, f2, s2, font)
                }));
                if let Err(e) = result {
                    let msg = e
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "raylib renderer stopped".into());
                    if let Ok(mut s) = s3.lock() {
                        *s = Status::Failed(msg);
                    }
                }
            })
            .expect("start raylib renderer thread");
        Self {
            tx,
            frame,
            status,
            thread: Some(thread),
        }
    }

    /// Current status.
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Status::Failed("poisoned".into()))
    }

    /// Queue a display list. Older queued lists are superseded.
    pub fn submit(&self, list: DisplayList) {
        let _ = self.tx.try_send(Some(list));
    }

    /// Take the newest finished frame, if any.
    pub fn take_frame(&self) -> Option<Frame> {
        self.frame.lock().ok().and_then(|mut f| f.take())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.tx.send(None);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn color(c: Rgba) -> raylib::ffi::Color {
    raylib::ffi::Color {
        r: c[0],
        g: c[1],
        b: c[2],
        a: c[3],
    }
}

fn v2(p: Px, s: f32) -> raylib::ffi::Vector2 {
    raylib::ffi::Vector2 {
        x: p[0] * s,
        y: p[1] * s,
    }
}

fn run(
    rx: Receiver<Option<DisplayList>>,
    out: Arc<Mutex<Option<Frame>>>,
    status: Arc<Mutex<Status>>,
    font_bytes: &'static [u8],
) {
    use raylib::prelude::*;
    let (mut rl, thread) = raylib::init()
        .log_level(TraceLogLevel::LOG_WARNING)
        .size(320, 240)
        .title("Electrocraft offscreen renderer")
        .hidden()
        .build();
    let mut glyphs: String = (32u8..127).map(char::from).collect();
    glyphs.push_str(EXTRA_GLYPHS);
    const FONT_PX: i32 = 48;
    let font = rl
        .load_font_from_memory(&thread, ".ttf", font_bytes, FONT_PX, Some(&glyphs))
        .ok();
    if let Some(f) = &font {
        unsafe {
            raylib::ffi::SetTextureFilter(
                AsRef::<raylib::ffi::Font>::as_ref(f).texture,
                raylib::ffi::TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
            )
        };
    }
    let font_raw = font
        .as_ref()
        .map(|f| {
            let raw: &raylib::ffi::Font = f.as_ref();
            *raw
        })
        .unwrap_or_else(|| unsafe { raylib::ffi::GetFontDefault() });
    if let Ok(mut s) = status.lock() {
        *s = Status::Ready;
    }
    let mut size = (0u32, 0u32);
    let mut big: Option<RenderTexture2D> = None;
    let mut small: Option<RenderTexture2D> = None;
    const SS: f32 = 2.0;
    while let Ok(msg) = rx.recv() {
        let Some(mut list) = msg else { return };
        // Skip to the newest queued list.
        while let Ok(next) = rx.try_recv() {
            match next {
                Some(l) => list = l,
                None => return,
            }
        }
        let start = Instant::now();
        let (w, h) = (list.width.max(1), list.height.max(1));
        if size != (w, h) || big.is_none() {
            big = rl
                .load_render_texture(&thread, (w as f32 * SS) as u32, (h as f32 * SS) as u32)
                .ok();
            small = rl.load_render_texture(&thread, w, h).ok();
            if let Some(b) = &big {
                unsafe {
                    raylib::ffi::SetTextureFilter(
                        b.texture,
                        raylib::ffi::TextureFilter::TEXTURE_FILTER_BILINEAR as i32,
                    )
                };
            }
            size = (w, h);
        }
        let (Some(bt), Some(st)) = (big.as_mut(), small.as_mut()) else {
            if let Ok(mut s) = status.lock() {
                *s = Status::Failed("cannot create render texture".into());
            }
            return;
        };
        unsafe {
            use raylib::ffi::*;
            BeginTextureMode(**bt);
            ClearBackground(color(list.clear));
            for shape in &list.shapes {
                draw_shape(shape, SS, font_raw, FONT_PX as f32);
            }
            EndTextureMode();
            // Downsample 2x2 with bilinear filtering. Render textures are stored
            // upside down, so the source rectangle has a negative height.
            BeginTextureMode(**st);
            ClearBackground(color(list.clear));
            let src = Rectangle {
                x: 0.0,
                y: 0.0,
                width: bt.texture.width as f32,
                height: -(bt.texture.height as f32),
            };
            let dst = Rectangle {
                x: 0.0,
                y: 0.0,
                width: w as f32,
                height: h as f32,
            };
            DrawTexturePro(
                bt.texture,
                src,
                dst,
                Vector2 { x: 0.0, y: 0.0 },
                0.0,
                color([255, 255, 255, 255]),
            );
            EndTextureMode();
        }
        let read_start = Instant::now();
        let rgba = unsafe {
            let img = raylib::ffi::LoadImageFromTexture(st.texture);
            let bytes = (img.width * img.height * 4) as usize;
            let data = std::slice::from_raw_parts(img.data as *const u8, bytes);
            // Flip rows: GL textures are bottom-up.
            let row = img.width as usize * 4;
            let mut out = vec![0u8; bytes];
            for y in 0..img.height as usize {
                let src = (img.height as usize - 1 - y) * row;
                out[y * row..(y + 1) * row].copy_from_slice(&data[src..src + row]);
            }
            raylib::ffi::UnloadImage(img);
            out
        };
        let frame = Frame {
            width: w,
            height: h,
            rgba,
            revision: list.revision,
            render_ms: start.elapsed().as_secs_f32() * 1000.0,
            readback_ms: read_start.elapsed().as_secs_f32() * 1000.0,
        };
        if let Ok(mut f) = out.lock() {
            *f = Some(frame);
        }
    }
}

/// Draw one shape with the raw raylib API, scaled by `s`.
unsafe fn draw_shape(shape: &Shape, s: f32, font: raylib::ffi::Font, font_px: f32) {
    use raylib::ffi::*;
    unsafe {
        match shape {
            Shape::Line {
                a,
                b,
                width,
                color: c,
            } => {
                DrawLineEx(v2(*a, s), v2(*b, s), width * s, color(*c));
            }
            Shape::Polyline {
                points,
                width,
                color: c,
                closed,
            } => {
                let n = points.len();
                let segs = if *closed { n } else { n.saturating_sub(1) };
                for i in 0..segs {
                    DrawLineEx(
                        v2(points[i], s),
                        v2(points[(i + 1) % n], s),
                        width * s,
                        color(*c),
                    );
                }
                if *width * s > 2.0 {
                    // Round joins so thick corners have no notches.
                    for p in points
                        .iter()
                        .skip(if *closed { 0 } else { 1 })
                        .take(if *closed { n } else { n.saturating_sub(2) })
                    {
                        DrawCircleV(v2(*p, s), width * s / 2.0, color(*c));
                    }
                }
            }
            Shape::Polygon { points, color: c } => {
                if points.len() >= 3 {
                    let o = v2(points[0], s);
                    for i in 1..points.len() - 1 {
                        let (p, q) = (v2(points[i], s), v2(points[i + 1], s));
                        // One of the two windings survives back-face culling.
                        DrawTriangle(o, p, q, color(*c));
                        DrawTriangle(o, q, p, color(*c));
                    }
                }
            }
            Shape::Circle {
                c: centre,
                r,
                color: c,
                fill,
                width,
            } => {
                let segs = ((r * s) as i32 / 2).clamp(16, 96);
                if *fill {
                    DrawCircleSector(v2(*centre, s), r * s, 0.0, 360.0, segs, color(*c));
                } else {
                    DrawRing(
                        v2(*centre, s),
                        (r - width / 2.0) * s,
                        (r + width / 2.0) * s,
                        0.0,
                        360.0,
                        segs,
                        color(*c),
                    );
                }
            }
            Shape::Rect {
                min,
                max,
                color: c,
                fill,
                width,
            } => {
                let rect = Rectangle {
                    x: min[0] * s,
                    y: min[1] * s,
                    width: (max[0] - min[0]) * s,
                    height: (max[1] - min[1]) * s,
                };
                if *fill {
                    DrawRectangleRec(rect, color(*c));
                } else {
                    DrawRectangleLinesEx(rect, width * s, color(*c));
                }
            }
            Shape::Glow {
                c: centre,
                r,
                inner,
            } => {
                let mut outer = *inner;
                outer[3] = 0;
                DrawCircleGradient(v2(*centre, s), r * s, color(*inner), color(outer));
            }
            Shape::Text {
                at,
                text,
                size,
                color: c,
                align,
            } => {
                let Ok(ctext) = std::ffi::CString::new(text.as_str()) else {
                    return;
                };
                let px = size * s;
                let spacing = 0.0;
                let m = MeasureTextEx(font, ctext.as_ptr(), px, spacing);
                let x = match align {
                    Align::Left => at[0] * s,
                    Align::Center => at[0] * s - m.x / 2.0,
                    Align::Right => at[0] * s - m.x,
                };
                let y = at[1] * s - m.y / 2.0;
                let _ = font_px;
                DrawTextEx(
                    font,
                    ctext.as_ptr(),
                    Vector2 { x, y },
                    px,
                    spacing,
                    color(*c),
                );
            }
        }
    }
}
