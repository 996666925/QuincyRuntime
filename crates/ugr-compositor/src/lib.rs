//! Single-window compositor for native rendering layers.

#![allow(clippy::large_enum_variant, clippy::chunks_exact_to_as_chunks)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use skia_safe::{Canvas, Color, Paint, PathBuilder};
use ugr_ui::{UiEventTarget, UiRenderer};
use ugr_webgl::GlCommand;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

mod event;

pub use event::{Modifiers, UiEvent, UiEventHandler, UiEventResult};

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
    window: Option<Arc<Window>>,
    gpu: Option<GpuCompositor>,
    redraw_pending: bool,
    ui_resize_pending: bool,
    ui_redraw_pending: bool,
    last_resize: Instant,
    ui_pixels: Vec<u8>,
    ui_pixel_width: u32,
    ui_pixel_height: u32,
    composed_pixels: Vec<u8>,
    cursor_position: (f32, f32),
    mouse_left_down: bool,
    clipboard: String,
    modifiers: winit::keyboard::ModifiersState,
    ui_event_handler: Option<UiEventHandler>,
    mouse_down_target: Option<UiEventTarget>,
    hovered_target: Option<UiEventTarget>,
}

impl Compositor {
    pub fn new(title: String, width: u32, height: u32, layers: Vec<Layer>) -> Result<Self, String> {
        Ok(Self {
            title,
            width,
            height,
            surface_width: width.max(1),
            surface_height: height.max(1),
            layers,
            window: None,
            gpu: None,
            redraw_pending: false,
            ui_resize_pending: false,
            ui_redraw_pending: true,
            last_resize: Instant::now(),
            ui_pixels: Vec::new(),
            ui_pixel_width: width.max(1),
            ui_pixel_height: height.max(1),
            composed_pixels: Vec::new(),
            cursor_position: (0.0, 0.0),
            mouse_left_down: false,
            clipboard: String::new(),
            modifiers: winit::keyboard::ModifiersState::empty(),
            ui_event_handler: None,
            mouse_down_target: None,
            hovered_target: None,
        })
    }

    /// Connect native pointer events to the JavaScript DOM runtime.
    pub fn set_ui_event_handler(&mut self, handler: UiEventHandler) {
        self.ui_event_handler = Some(handler);
    }

    fn dispatch_ui_event(&mut self, event: UiEvent) -> UiEventResult {
        let Some(handler) = self.ui_event_handler.as_mut() else {
            return UiEventResult::default();
        };
        let result = match handler(event) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("UI event dispatch failed: {error}");
                return UiEventResult::default();
            }
        };
        if let Some(markup) = result.markup.as_deref() {
            for layer in &mut self.layers {
                if let Layer::Ui(ui) = layer {
                    if let Err(error) = ui.update_from_html(markup) {
                        eprintln!("UI event update failed: {error}");
                    }
                }
            }
            self.ui_redraw_pending = true;
            self.request_redraw();
        }
        result
    }
}

impl winit::application::ApplicationHandler for Compositor {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
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
        let size = window.inner_size();
        let mut gpu = match pollster::block_on(GpuCompositor::new(window.clone())) {
            Ok(gpu) => gpu,
            Err(error) => {
                eprintln!("Compositor GPU initialization failed: {error}");
                event_loop.exit();
                return;
            }
        };
        gpu.resize(size.width.max(1), size.height.max(1));
        window.set_ime_allowed(true);
        self.surface_width = size.width.max(1);
        self.surface_height = size.height.max(1);
        let scale_factor = window.scale_factor() as f32;
        for layer in &mut self.layers {
            if let Layer::Ui(ui) = layer {
                if let Err(error) = ui.resize_with_physical_size(
                    self.width,
                    self.height,
                    self.surface_width,
                    self.surface_height,
                    scale_factor,
                ) {
                    eprintln!("UI DPI initialization failed: {error}");
                }
                self.ui_pixel_width = self.surface_width;
                self.ui_pixel_height = self.surface_height;
            }
        }
        self.window = Some(window);
        self.gpu = Some(gpu);
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
                self.ui_resize_pending = true;
                self.last_resize = Instant::now();
                self.request_redraw();
            }
            winit::event::WindowEvent::RedrawRequested if self.redraw_pending => {
                self.redraw_pending = false;
                self.draw_frame();
            }
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x as f32, position.y as f32);
                let scale = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0) as f32;
                let (x, y) = (
                    self.cursor_position.0 / scale,
                    self.cursor_position.1 / scale,
                );
                let target = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.event_target_at(x, y),
                    _ => None,
                });
                let mut state_changed = false;
                for layer in &mut self.layers {
                    if let Layer::Ui(ui) = layer {
                        state_changed |= ui.set_hovered(target.as_ref());
                    }
                }
                if self.hovered_target != target {
                    if let Some(previous) = self.hovered_target.take() {
                        self.dispatch_ui_event(UiEvent::pointer(
                            "mouseout",
                            Some(previous),
                            x,
                            y,
                            None,
                            0,
                            0,
                            self.modifiers.into(),
                        ));
                    }
                    if let Some(current) = target.clone() {
                        self.dispatch_ui_event(UiEvent::pointer(
                            "mouseover",
                            Some(current.clone()),
                            x,
                            y,
                            None,
                            0,
                            0,
                            self.modifiers.into(),
                        ));
                        self.hovered_target = Some(current);
                    }
                }
                self.dispatch_ui_event(UiEvent::pointer(
                    "mousemove",
                    target,
                    x,
                    y,
                    None,
                    if self.mouse_left_down { 1 } else { 0 },
                    0,
                    self.modifiers.into(),
                ));
                if self.mouse_left_down {
                    let mut changed = false;
                    for layer in &mut self.layers {
                        if let Layer::Ui(ui) = layer {
                            ui.set_caret_from_point_with_selection(x, y);
                            changed = true;
                        }
                    }
                    if changed {
                        self.ui_redraw_pending = true;
                        self.request_redraw();
                    }
                }
                if state_changed {
                    self.ui_redraw_pending = true;
                    self.request_redraw();
                }
            }
            winit::event::WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                self.mouse_left_down = true;
                let scale = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0) as f32;
                let (x, y) = (
                    self.cursor_position.0 / scale,
                    self.cursor_position.1 / scale,
                );
                let mut changed = false;
                let previous_focus = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.focused_event_target(),
                    _ => None,
                });
                self.mouse_down_target = None;
                for layer in &mut self.layers {
                    if let Layer::Ui(ui) = layer {
                        self.mouse_down_target = ui.event_target_at(x, y);
                        changed |= ui.set_pressed(self.mouse_down_target.as_ref());
                        if ui.focus_at(x, y) {
                            ui.set_caret_from_point(x, y);
                            ui.begin_selection();
                            changed = true;
                        }
                    }
                }
                self.dispatch_ui_event(UiEvent::pointer(
                    "mousedown",
                    self.mouse_down_target.clone(),
                    x,
                    y,
                    Some(0),
                    1,
                    1,
                    self.modifiers.into(),
                ));
                let current_focus = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.focused_event_target(),
                    _ => None,
                });
                if previous_focus != current_focus {
                    if let Some(target) = previous_focus {
                        self.dispatch_ui_event(UiEvent::focus("blur", Some(target)));
                    }
                    if let Some(target) = current_focus {
                        self.dispatch_ui_event(UiEvent::focus("focus", Some(target)));
                    }
                }
                if changed {
                    self.ui_redraw_pending = true;
                    self.request_redraw();
                }
            }
            winit::event::WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                self.mouse_left_down = false;
                let scale = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0) as f32;
                let point = (
                    self.cursor_position.0 / scale,
                    self.cursor_position.1 / scale,
                );
                let pressed_target = self.mouse_down_target.take();
                for layer in &mut self.layers {
                    if let Layer::Ui(ui) = layer {
                        if ui.set_pressed(None) {
                            self.ui_redraw_pending = true;
                        }
                    }
                }
                if self.ui_redraw_pending {
                    self.request_redraw();
                }
                let released_target = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.event_target_at(point.0, point.1),
                    _ => None,
                });
                if let (Some(pressed), Some(released)) = (pressed_target, released_target) {
                    self.dispatch_ui_event(UiEvent::pointer(
                        "mouseup",
                        Some(released.clone()),
                        point.0,
                        point.1,
                        Some(0),
                        0,
                        1,
                        self.modifiers.into(),
                    ));
                    if pressed == released {
                        let modifiers = Modifiers::from(self.modifiers);
                        let click_result = self.dispatch_ui_event(UiEvent::pointer(
                            "click",
                            Some(released.clone()),
                            point.0,
                            point.1,
                            Some(0),
                            0,
                            1,
                            modifiers,
                        ));
                        if !click_result.default_prevented {
                            let mut activated = false;
                            for layer in &mut self.layers {
                                if let Layer::Ui(ui) = layer {
                                    match ui.activate(&released) {
                                        Ok(true) => {
                                            activated = true;
                                        }
                                        Ok(false) => {}
                                        Err(error) => {
                                            eprintln!("UI control activation failed: {error}")
                                        }
                                    }
                                }
                            }
                            if activated {
                                self.ui_redraw_pending = true;
                                self.request_redraw();
                            }
                        }
                    }
                }
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0) as f32;
                let point = (
                    self.cursor_position.0 / scale,
                    self.cursor_position.1 / scale,
                );
                let target = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.event_target_at(point.0, point.1),
                    _ => None,
                });
                let (delta_x, delta_y) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x * 16.0, y * 16.0),
                    winit::event::MouseScrollDelta::PixelDelta(value) => {
                        (value.x as f32, value.y as f32)
                    }
                };
                let mut event = UiEvent::pointer(
                    "wheel",
                    target,
                    point.0,
                    point.1,
                    None,
                    0,
                    0,
                    self.modifiers.into(),
                );
                event.data = Some(format!("{delta_x},{delta_y}"));
                self.dispatch_ui_event(event);
            }
            winit::event::WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                let mut changed = false;
                let mut input_target = None;
                for layer in &mut self.layers {
                    if let Layer::Ui(ui) = layer {
                        if ui.input_text(&text) {
                            changed = true;
                            input_target = ui.focused_event_target();
                        }
                    }
                }
                if changed {
                    self.dispatch_ui_event(UiEvent::input(
                        "input",
                        input_target,
                        Some(text.clone()),
                        "insertText",
                        Some(text),
                    ));
                }
                if changed {
                    self.ui_redraw_pending = true;
                    self.request_redraw();
                }
            }
            winit::event::WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            winit::event::WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Pressed =>
            {
                let key = match &event.logical_key {
                    winit::keyboard::Key::Named(named) => format!("{named:?}"),
                    winit::keyboard::Key::Character(character) => character.to_string(),
                    _ => return,
                };
                let target = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.focused_event_target(),
                    _ => None,
                });
                let key_event = self.dispatch_ui_event(UiEvent::keyboard(
                    "keydown",
                    target,
                    key.clone(),
                    format!("{:?}", event.physical_key),
                    event.repeat,
                    self.modifiers.into(),
                ));
                if key_event.default_prevented {
                    return;
                }
                if self.modifiers.control_key() {
                    let normalized = key.to_ascii_lowercase();
                    if normalized == "c" || normalized == "keyc" {
                        for layer in &self.layers {
                            if let Layer::Ui(ui) = layer {
                                if let Some(text) = ui.selected_text() {
                                    self.clipboard = text;
                                    break;
                                }
                            }
                        }
                        return;
                    }
                    if normalized == "x" || normalized == "keyx" {
                        let mut changed = false;
                        for layer in &mut self.layers {
                            if let Layer::Ui(ui) = layer {
                                if let Some(text) = ui.cut_selection() {
                                    self.clipboard = text;
                                    changed = true;
                                    break;
                                }
                            }
                        }
                        if changed {
                            self.ui_redraw_pending = true;
                            self.request_redraw();
                        }
                        return;
                    }
                    if normalized == "v" || normalized == "keyv" {
                        let clipboard = self.clipboard.clone();
                        let mut changed = false;
                        for layer in &mut self.layers {
                            if let Layer::Ui(ui) = layer {
                                if ui.input_text(&clipboard) {
                                    changed = true;
                                    break;
                                }
                            }
                        }
                        if changed {
                            self.ui_redraw_pending = true;
                            self.request_redraw();
                        }
                        return;
                    }
                }
                if let Some(text) = event.text.as_ref().filter(|text| {
                    !text.is_empty()
                        && text.chars().all(|character| !character.is_control())
                        && !self.modifiers.control_key()
                        && !self.modifiers.alt_key()
                }) {
                    let mut changed = false;
                    for layer in &mut self.layers {
                        if let Layer::Ui(ui) = layer {
                            if ui.input_text(text.as_ref()) {
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        self.ui_redraw_pending = true;
                        self.request_redraw();
                    }
                    return;
                }
                let mut changed = false;
                for layer in &mut self.layers {
                    if let Layer::Ui(ui) = layer {
                        if ui.edit_key_with_modifiers(
                            &key,
                            self.modifiers.shift_key(),
                            self.modifiers.control_key(),
                        ) {
                            changed = true;
                        }
                    }
                }
                if changed {
                    self.ui_redraw_pending = true;
                    self.request_redraw();
                }
            }
            winit::event::WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Released =>
            {
                let key = match &event.logical_key {
                    winit::keyboard::Key::Named(named) => format!("{named:?}"),
                    winit::keyboard::Key::Character(character) => character.to_string(),
                    _ => return,
                };
                let target = self.layers.iter().find_map(|layer| match layer {
                    Layer::Ui(ui) => ui.focused_event_target(),
                    _ => None,
                });
                self.dispatch_ui_event(UiEvent::keyboard(
                    "keyup",
                    target,
                    key,
                    format!("{:?}", event.physical_key),
                    event.repeat,
                    self.modifiers.into(),
                ));
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.redraw_pending || self.ui_resize_pending {
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
        if self.ui_resize_pending && self.last_resize.elapsed() >= Duration::from_millis(25) {
            let scale = self
                .window
                .as_ref()
                .map(|window| window.scale_factor() as f32)
                .unwrap_or(1.0);
            for layer in &mut self.layers {
                if let Layer::Ui(ui) = layer {
                    if let Err(error) = ui.resize_with_physical_size(
                        self.width,
                        self.height,
                        self.surface_width,
                        self.surface_height,
                        scale,
                    ) {
                        eprintln!("UI resize failed: {error}");
                    }
                }
            }
            self.ui_redraw_pending = true;
            self.ui_resize_pending = false;
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.resize(self.surface_width, self.surface_height);
        let redraw_ui = self.ui_redraw_pending;
        let ui_only = self.layers.len() == 1 && matches!(&self.layers[0], Layer::Ui(_));
        if ui_only && !self.ui_resize_pending {
            if redraw_ui {
                if let Layer::Ui(ui) = &mut self.layers[0] {
                    ui.draw();
                    self.ui_pixels.clear();
                    self.ui_pixels.extend_from_slice(ui.rgba_pixels_ref());
                    (self.ui_pixel_width, self.ui_pixel_height) = ui.pixel_size();
                }
            }
            if self.ui_pixel_width == self.surface_width
                && self.ui_pixel_height == self.surface_height
            {
                self.ui_redraw_pending = false;
                gpu.present(&self.ui_pixels, self.ui_pixel_width, self.ui_pixel_height);
                return;
            }
        }
        let pixel_len = (self.surface_width.max(1) * self.surface_height.max(1) * 4) as usize;
        if self.composed_pixels.len() != pixel_len {
            self.composed_pixels.resize(pixel_len, 255);
        }
        let pixels = &mut self.composed_pixels;
        // Start with an opaque page background. GPU layers may replace it via
        // clear commands, while transparent UI pixels leave the background
        // intact during alpha compositing.
        pixels
            .chunks_exact_mut(4)
            .for_each(|p| p.copy_from_slice(&[255, 255, 255, 255]));
        for layer in &mut self.layers {
            match layer {
                Layer::Ui(ui) => {
                    if redraw_ui {
                        ui.draw();
                        self.ui_pixels.clear();
                        self.ui_pixels.extend_from_slice(ui.rgba_pixels_ref());
                        (self.ui_pixel_width, self.ui_pixel_height) = ui.pixel_size();
                    }
                    if self.ui_resize_pending {
                        blend_rgba_clipped(
                            pixels,
                            self.surface_width,
                            self.surface_height,
                            &self.ui_pixels,
                            self.ui_pixel_width,
                            self.ui_pixel_height,
                        );
                    } else {
                        blend_rgba_scaled(
                            pixels,
                            self.surface_width,
                            self.surface_height,
                            &self.ui_pixels,
                            self.ui_pixel_width,
                            self.ui_pixel_height,
                        );
                    }
                }
                Layer::WebGl(commands) => {
                    raster_webgl(pixels, self.surface_width, self.surface_height, commands)
                }
                Layer::WebGpu(commands) => {
                    raster_webgpu(pixels, self.surface_width, self.surface_height, commands)
                }
            }
        }
        self.ui_redraw_pending = false;
        gpu.present(pixels, self.surface_width, self.surface_height);
    }
}

struct GpuCompositor {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl GpuCompositor {
    async fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window)
            .map_err(|error| format!("failed to create compositor surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("failed to find compositor adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| format!("failed to create compositor device: {error}"))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| "compositor surface exposes no texture formats".to_owned())?;
        let alpha_mode = capabilities
            .alpha_modes
            .first()
            .copied()
            .ok_or_else(|| "compositor surface exposes no alpha modes".to_owned())?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            // The surface is configured on the first window-size notification.
            // Keep zero here so a real 1x1 window is not mistaken for an
            // already-configured surface.
            width: 0,
            height: 0,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        let texture = Self::create_texture(&device, 1, 1);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor-texture-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("compositor-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor-texture-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositor-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compositor.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compositor-blit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            texture,
            bind_group,
            pipeline,
        })
    }

    fn create_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("compositor-frame-texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.texture = Self::create_texture(&self.device, width, height);
        let layout = self.pipeline.get_bind_group_layout(0);
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let view = self.texture.create_view(&Default::default());
        self.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor-texture-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
    }

    fn present(&mut self, pixels: &[u8], width: u32, height: u32) {
        if pixels.is_empty() || width == 0 || height == 0 {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width.saturating_mul(4)),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("compositor-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("compositor-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
    }
}

/// Copy a tightly packed RGBA image into a packed RGB destination for tests.
/// The destination dimensions are authoritative when source and destination
/// sizes differ.
#[cfg(test)]
fn copy_rgba_to_rgb(
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

fn blend_rgba_scaled(
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    src: &[u8],
    src_width: u32,
    src_height: u32,
) {
    if dst_width == src_width && dst_height == src_height {
        blend_rgba(dst, src);
        return;
    }
    let dw = dst_width.max(1) as usize;
    let dh = dst_height.max(1) as usize;
    let sw = src_width.max(1) as usize;
    let sh = src_height.max(1) as usize;
    for y in 0..dh {
        let sy = y.saturating_mul(sh) / dh;
        for x in 0..dw {
            let sx = x.saturating_mul(sw) / dw;
            let si = (sy * sw + sx) * 4;
            let di = (y * dw + x) * 4;
            if si + 3 >= src.len() || di + 3 >= dst.len() {
                continue;
            }
            blend_rgba(&mut dst[di..di + 4], &src[si..si + 4]);
        }
    }
}

fn blend_rgba_clipped(
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    src: &[u8],
    src_width: u32,
    src_height: u32,
) {
    let width = dst_width.min(src_width) as usize;
    let height = dst_height.min(src_height) as usize;
    let dw = dst_width.max(1) as usize;
    let sw = src_width.max(1) as usize;
    for y in 0..height {
        let dst_row = y * dw * 4;
        let src_row = y * sw * 4;
        for x in 0..width {
            let di = dst_row + x * 4;
            let si = src_row + x * 4;
            if di + 3 >= dst.len() || si + 3 >= src.len() {
                continue;
            }
            blend_rgba(&mut dst[di..di + 4], &src[si..si + 4]);
        }
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
    use super::{blend_rgba, copy_rgba_to_rgb};

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
        copy_rgba_to_rgb(&mut destination, &source, 2, 1, 6, 1);
        assert_eq!(
            destination,
            [0x00ff0000, 0x00ff0000, 0x00ff0000, 0x0000ff00, 0x0000ff00, 0x0000ff00]
        );
    }
}
