//! Single-window compositor for native rendering layers.

#![allow(clippy::large_enum_variant, clippy::chunks_exact_to_as_chunks)]

use std::num::NonZeroU32;
use std::sync::Arc;

use skia_safe::{Canvas, Color, Paint, PathBuilder};
use ugr_ui::UiRenderer;
use ugr_webgl::GlCommand;
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::window::Window;

pub enum Layer {
    Ui(UiRenderer),
    WebGl(Vec<GlCommand>),
    WebGpu(Vec<serde_json::Value>),
}

pub struct Compositor {
    title: String,
    /// CSS/layout viewport size in logical pixels.
    width: u32,
    height: u32,
    /// Native surface size in physical pixels.
    surface_width: u32,
    surface_height: u32,
    layers: Vec<Layer>,
    context: softbuffer::Context<OwnedDisplayHandle>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    redraw_pending: bool,
}

impl Compositor {
    pub fn new(
        title: String,
        width: u32,
        height: u32,
        layers: Vec<Layer>,
        display: OwnedDisplayHandle,
    ) -> Result<Self, String> {
        Ok(Self {
            title,
            width,
            height,
            surface_width: width.max(1),
            surface_height: height.max(1),
            layers,
            context: softbuffer::Context::new(display)
                .map_err(|e| format!("failed to create compositor display context: {e}"))?,
            window: None,
            surface: None,
            redraw_pending: false,
        })
    }
}

impl winit::application::ApplicationHandler for Compositor {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title(self.title.clone())
                .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("Compositor window creation failed: {error}");
                event_loop.exit();
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&self.context, window.clone()) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("Compositor surface creation failed: {error}");
                event_loop.exit();
                return;
            }
        };
        let initial_size = window.inner_size();
        self.surface_width = initial_size.width.max(1);
        self.surface_height = initial_size.height.max(1);
        let scale_factor = window.scale_factor() as f32;
        for layer in &mut self.layers {
            if let Layer::Ui(ui) = layer {
                if let Err(error) = ui.resize_for_scale(self.width, self.height, scale_factor) {
                    eprintln!("UI DPI initialization failed: {error}");
                }
            }
        }
        self.window = Some(window);
        self.surface = Some(surface);
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            winit::event::WindowEvent::Resized(size) => {
                self.surface_width = size.width.max(1);
                self.surface_height = size.height.max(1);
                let scale = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0)
                    .max(f64::EPSILON);
                self.width = ((self.surface_width as f64 / scale).round() as u32).max(1);
                self.height = ((self.surface_height as f64 / scale).round() as u32).max(1);
                for layer in &mut self.layers {
                    if let Layer::Ui(ui) = layer {
                        if let Err(error) =
                            ui.resize_for_scale(self.width, self.height, scale as f32)
                        {
                            eprintln!("UI resize failed: {error}");
                        }
                    }
                }
                self.request_redraw();
            }
            winit::event::WindowEvent::RedrawRequested if self.redraw_pending => {
                self.redraw_pending = false;
                self.draw_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.redraw_pending {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

impl Compositor {
    /// Mark the composed frame dirty. The actual work is deferred to winit's
    /// RedrawRequested event so idle windows do not continuously rasterize.
    pub fn request_redraw(&mut self) {
        self.redraw_pending = true;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn draw_frame(&mut self) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let (Some(width), Some(height)) = (
            NonZeroU32::new(self.surface_width.max(1)),
            NonZeroU32::new(self.surface_height.max(1)),
        ) else {
            return;
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let mut pixels =
            vec![0u8; (self.surface_width.max(1) * self.surface_height.max(1) * 4) as usize];
        // Start with an opaque page background. GPU layers may replace it via
        // clear commands, while transparent UI pixels leave the background
        // intact during alpha compositing.
        pixels
            .chunks_exact_mut(4)
            .for_each(|p| p.copy_from_slice(&[255, 255, 255, 255]));
        for layer in &mut self.layers {
            match layer {
                Layer::Ui(ui) => {
                    ui.draw();
                    blend_rgba(&mut pixels, &ui.rgba_pixels());
                }
                Layer::WebGl(commands) => raster_webgl(
                    &mut pixels,
                    self.surface_width,
                    self.surface_height,
                    commands,
                ),
                Layer::WebGpu(commands) => raster_webgpu(
                    &mut pixels,
                    self.surface_width,
                    self.surface_height,
                    commands,
                ),
            }
        }
        let mut buffer = match surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(_) => return,
        };
        let dst_width = buffer.width().get();
        let dst_height = buffer.height().get();
        copy_rgba_to_softbuffer(
            &mut buffer,
            &pixels,
            self.surface_width,
            self.surface_height,
            dst_width,
            dst_height,
        );
        let _ = buffer.present();
    }
}

/// Copy a tightly packed RGBA image into softbuffer's packed 0x00RRGGBB
/// surface. The destination dimensions are authoritative: on high-DPI
/// windows they can differ from the logical render size, and flattening the
/// source iterator directly would concatenate rows and produce tiled stripes.
fn copy_rgba_to_softbuffer(
    buffer: &mut [u32],
    pixels: &[u8],
    source_width: u32,
    source_height: u32,
    destination_width: u32,
    destination_height: u32,
) {
    if buffer.is_empty() || pixels.is_empty() {
        return;
    }
    let src_width = source_width.max(1) as usize;
    let src_height = source_height.max(1) as usize;
    let dst_width = destination_width.max(1) as usize;
    let dst_height = destination_height.max(1) as usize;
    if src_width == dst_width && src_height == dst_height {
        for (index, pixel) in buffer.iter_mut().enumerate() {
            let offset = index * 4;
            if offset + 2 >= pixels.len() {
                break;
            }
            *pixel = (u32::from(pixels[offset]) << 16)
                | (u32::from(pixels[offset + 1]) << 8)
                | u32::from(pixels[offset + 2]);
        }
        return;
    }
    for y in 0..dst_height {
        let sy = y.saturating_mul(src_height) / dst_height;
        for x in 0..dst_width {
            let sx = x.saturating_mul(src_width) / dst_width;
            let offset = (sy * src_width + sx) * 4;
            let index = y * dst_width + x;
            if offset + 2 >= pixels.len() || index >= buffer.len() {
                continue;
            }
            buffer[index] = (u32::from(pixels[offset]) << 16)
                | (u32::from(pixels[offset + 1]) << 8)
                | u32::from(pixels[offset + 2]);
        }
    }
}

fn blend_rgba(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let alpha = u16::from(s[3]);
        let inverse = 255 - alpha;
        d[0] = ((u16::from(s[0]) * alpha + u16::from(d[0]) * inverse) / 255) as u8;
        d[1] = ((u16::from(s[1]) * alpha + u16::from(d[1]) * inverse) / 255) as u8;
        d[2] = ((u16::from(s[2]) * alpha + u16::from(d[2]) * inverse) / 255) as u8;
        d[3] = 255;
    }
}

fn raster_webgl(pixels: &mut [u8], width: u32, height: u32, commands: &[GlCommand]) {
    let mut color = [8u8, 10, 18, 255];
    let mut draw = false;
    for command in commands {
        match *command {
            GlCommand::ClearColor(r, g, b, a) => {
                color = [
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    (a.clamp(0.0, 1.0) * 255.0) as u8,
                ]
            }
            GlCommand::Clear(_) => pixels
                .chunks_exact_mut(4)
                .for_each(|p| p.copy_from_slice(&color)),
            GlCommand::DrawArrays(_, _, _) | GlCommand::DrawElements(_, _, _, _) => draw = true,
            GlCommand::Viewport(_, _, _, _) => {}
        }
    }
    if draw {
        draw_triangle(
            pixels,
            width,
            height,
            [
                Color::from_rgb(255, 80, 80),
                Color::from_rgb(80, 220, 120),
                Color::from_rgb(80, 140, 255),
            ],
        );
    }
}

fn raster_webgpu(pixels: &mut [u8], width: u32, height: u32, commands: &[serde_json::Value]) {
    let mut draw = false;
    for command in commands {
        match command.get("op").and_then(serde_json::Value::as_str) {
            Some("beginRenderPass") => {
                if let Some(clear) = command
                    .get("descriptor")
                    .and_then(|d| d.get("colorAttachments"))
                    .and_then(|a| a.get(0))
                    .and_then(|a| a.get("clearValue"))
                {
                    let c = [
                        clear
                            .get("r")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.03) as f32,
                        clear
                            .get("g")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.04) as f32,
                        clear
                            .get("b")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.07) as f32,
                        1.0,
                    ];
                    let color = [
                        (c[0] * 255.0) as u8,
                        (c[1] * 255.0) as u8,
                        (c[2] * 255.0) as u8,
                        255,
                    ];
                    pixels
                        .chunks_exact_mut(4)
                        .for_each(|p| p.copy_from_slice(&color));
                }
            }
            Some("draw") | Some("drawIndexed") => draw = true,
            _ => {}
        }
    }
    if draw {
        draw_triangle(
            pixels,
            width,
            height,
            [
                Color::from_rgb(255, 80, 80),
                Color::from_rgb(80, 220, 120),
                Color::from_rgb(80, 140, 255),
            ],
        );
    }
}

fn draw_triangle(pixels: &mut [u8], width: u32, height: u32, colors: [Color; 3]) {
    let mut surface = match skia_safe::surfaces::raster_n32_premul((width as i32, height as i32)) {
        Some(surface) => surface,
        None => return,
    };
    let canvas: &Canvas = surface.canvas();
    let mut path = PathBuilder::new();
    path.move_to((width as f32 * 0.5, height as f32 * 0.15));
    path.line_to((width as f32 * 0.2, height as f32 * 0.8));
    path.line_to((width as f32 * 0.8, height as f32 * 0.8));
    path.close();
    let mut paint = Paint::default();
    paint.set_color(colors[0]);
    canvas.draw_path(&path.detach(), &paint);
    if let Some(map) = surface.peek_pixels() {
        let mut overlay = vec![0u8; pixels.len()];
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let c = map.get_color((x, y));
                let i = ((y as u32 * width + x as u32) * 4) as usize;
                overlay[i..i + 4].copy_from_slice(&[c.r(), c.g(), c.b(), c.a()]);
            }
        }
        blend_rgba(pixels, &overlay);
    }
}

#[cfg(test)]
mod tests {
    use super::{blend_rgba, copy_rgba_to_softbuffer};

    #[test]
    fn alpha_composition_preserves_background() {
        let mut dst = vec![10, 20, 30, 255];
        blend_rgba(&mut dst, &[110, 120, 130, 0]);
        assert_eq!(dst, vec![10, 20, 30, 255]);
    }

    #[test]
    fn alpha_composition_overlays_foreground() {
        let mut dst = vec![0, 0, 0, 255];
        blend_rgba(&mut dst, &[200, 100, 50, 255]);
        assert_eq!(dst, vec![200, 100, 50, 255]);
    }

    #[test]
    fn copies_rgba_by_rows_when_destination_is_wider() {
        let source = [255, 0, 0, 255, 0, 255, 0, 255];
        let mut destination = vec![0u32; 6];
        copy_rgba_to_softbuffer(&mut destination, &source, 2, 1, 6, 1);
        assert_eq!(
            destination,
            [0x00ff0000, 0x00ff0000, 0x00ff0000, 0x0000ff00, 0x0000ff00, 0x0000ff00]
        );
    }
}
