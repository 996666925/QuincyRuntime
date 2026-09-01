use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use winit::window::Window;

pub struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipelines: HashMap<u64, wgpu::RenderPipeline>,
}

/// Window/event-loop adapter for the native WebGPU backend.
pub struct WgpuRenderer {
    title: String,
    width: u32,
    height: u32,
    state: Option<GpuState>,
    window: Option<Arc<Window>>,
    commands: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    Presented,
    Reconfigure,
    Skipped,
}

#[derive(Debug, Clone, Copy)]
struct DrawCall {
    pipeline: u64,
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

impl GpuState {
    pub async fn new(window: Arc<Window>) -> Result<Self, String> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(window)
            .map_err(|error| format!("failed to create GPU surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("failed to find D3D12 adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|error| format!("failed to create GPU device: {error}"))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| "D3D12 surface exposes no texture formats".to_owned())?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: 1,
            height: 1,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .ok_or_else(|| "D3D12 surface exposes no alpha modes".to_owned())?,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipelines: HashMap::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Executes the WebGPU command stream produced by JavaScript.
    pub fn render(&mut self, commands: &[Value]) -> RenderStatus {
        let mut current_pipeline = None;
        let mut draws = Vec::new();
        let mut load = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        let mut has_render_pass = false;
        for command in commands {
            match command.get("op").and_then(Value::as_str) {
                Some("createRenderPipeline") => {
                    if let Some(id) = command.get("pipeline").and_then(Value::as_u64) {
                        if let Ok(pipeline) = self.create_pipeline(command.get("descriptor")) {
                            self.pipelines.insert(id, pipeline);
                        }
                    }
                }
                Some("beginRenderPass") => {
                    has_render_pass = true;
                    if let Some(attachment) = command
                        .get("descriptor")
                        .and_then(|d| d.get("colorAttachments"))
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(Value::as_object)
                    {
                        if attachment.get("loadOp").and_then(Value::as_str) == Some("load") {
                            load = wgpu::LoadOp::Load;
                        } else {
                            load =
                                wgpu::LoadOp::Clear(color_from_json(attachment.get("clearValue")));
                        }
                    }
                }
                Some("setPipeline") => {
                    current_pipeline = command.get("pipeline").and_then(Value::as_u64)
                }
                Some("draw") => {
                    if let Some(pipeline) = current_pipeline {
                        draws.push(DrawCall {
                            pipeline,
                            vertex_count: command
                                .get("vertexCount")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as u32,
                            instance_count: command
                                .get("instanceCount")
                                .and_then(Value::as_u64)
                                .unwrap_or(1) as u32,
                            first_vertex: command
                                .get("firstVertex")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as u32,
                            first_instance: command
                                .get("firstInstance")
                                .and_then(Value::as_u64)
                                .unwrap_or(0) as u32,
                        });
                    }
                }
                _ => {}
            }
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                return RenderStatus::Reconfigure
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return RenderStatus::Skipped,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        if !has_render_pass {
            self.queue.present(frame);
            return RenderStatus::Presented;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("js-webgpu-command-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("js-webgpu-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for draw in draws {
                if let Some(pipeline) = self.pipelines.get(&draw.pipeline) {
                    pass.set_pipeline(pipeline);
                    pass.draw(
                        draw.first_vertex..draw.first_vertex.saturating_add(draw.vertex_count),
                        draw.first_instance
                            ..draw.first_instance.saturating_add(draw.instance_count),
                    );
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        RenderStatus::Presented
    }

    fn create_pipeline(&self, descriptor: Option<&Value>) -> Result<wgpu::RenderPipeline, String> {
        let descriptor = descriptor.ok_or_else(|| "pipeline descriptor is missing".to_owned())?;
        let vertex = descriptor
            .get("vertex")
            .ok_or_else(|| "pipeline vertex stage is missing".to_owned())?;
        let vertex_code =
            shader_code(vertex).ok_or_else(|| "vertex shader code is missing".to_owned())?;
        let vertex_entry = entry_point(vertex);
        let vertex_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("js-webgpu-vertex-shader"),
                source: wgpu::ShaderSource::Wgsl(vertex_code.into()),
            });
        let fragment_stage = descriptor.get("fragment");
        let fragment_shader = fragment_stage.and_then(shader_code).map(|code| {
            self.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("js-webgpu-fragment-shader"),
                    source: wgpu::ShaderSource::Wgsl(code.into()),
                })
        });
        let fragment_entry = fragment_stage
            .map(entry_point)
            .unwrap_or_else(|| "main".to_owned());
        let target = wgpu::ColorTargetState {
            format: fragment_stage
                .and_then(|stage| stage.get("targets"))
                .and_then(Value::as_array)
                .and_then(|targets| targets.first())
                .and_then(|target| target.get("format"))
                .and_then(Value::as_str)
                .and_then(texture_format)
                .unwrap_or(self.config.format),
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        };
        let targets = [Some(target)];
        Ok(self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("js-webgpu-render-pipeline"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &vertex_shader,
                    entry_point: Some(&vertex_entry),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: fragment_shader.as_ref().map(|shader| wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(&fragment_entry),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                primitive: primitive_state(descriptor.get("primitive")),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            }))
    }
}

impl WgpuRenderer {
    pub fn new(title: String, width: u32, height: u32, commands: Vec<Value>) -> Self {
        Self {
            title,
            width,
            height,
            state: None,
            window: None,
            commands,
        }
    }

    pub async fn initialize(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> Result<(), String> {
        if self.state.is_some() {
            return Ok(());
        }
        let attributes = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| format!("failed to create native window: {error}"))?,
        );
        let size = window.inner_size();
        let mut state = GpuState::new(window.clone()).await?;
        state.resize(size.width, size.height);
        self.window = Some(window);
        self.state = Some(state);
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.state.is_some()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(state) = self.state.as_mut() {
            state.resize(width, height);
        }
    }

    pub fn draw(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.render(&self.commands) == RenderStatus::Reconfigure {
            if let Some(window) = self.window.as_ref() {
                let size = window.inner_size();
                state.resize(size.width, size.height);
            }
        }
    }

    pub fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn shader_code(stage: &Value) -> Option<&str> {
    stage.get("module")?.get("code")?.as_str()
}
fn entry_point(stage: &Value) -> String {
    stage
        .get("entryPoint")
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_owned()
}
fn color_from_json(value: Option<&Value>) -> wgpu::Color {
    let value = value.and_then(Value::as_object);
    wgpu::Color {
        r: value
            .and_then(|v| v.get("r"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        g: value
            .and_then(|v| v.get("g"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        b: value
            .and_then(|v| v.get("b"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        a: value
            .and_then(|v| v.get("a"))
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
    }
}
fn primitive_state(value: Option<&Value>) -> wgpu::PrimitiveState {
    let topology = value
        .and_then(|v| v.get("topology"))
        .and_then(Value::as_str)
        .map(|topology| match topology {
            "triangle-strip" => wgpu::PrimitiveTopology::TriangleStrip,
            "line-list" => wgpu::PrimitiveTopology::LineList,
            "line-strip" => wgpu::PrimitiveTopology::LineStrip,
            "point-list" => wgpu::PrimitiveTopology::PointList,
            _ => wgpu::PrimitiveTopology::TriangleList,
        })
        .unwrap_or(wgpu::PrimitiveTopology::TriangleList);
    wgpu::PrimitiveState {
        topology,
        ..Default::default()
    }
}
fn texture_format(value: &str) -> Option<wgpu::TextureFormat> {
    match value {
        "bgra8unorm" => Some(wgpu::TextureFormat::Bgra8Unorm),
        "bgra8unorm-srgb" => Some(wgpu::TextureFormat::Bgra8UnormSrgb),
        "rgba8unorm" => Some(wgpu::TextureFormat::Rgba8Unorm),
        "rgba8unorm-srgb" => Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        "rgba16float" => Some(wgpu::TextureFormat::Rgba16Float),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{color_from_json, primitive_state};
    use serde_json::json;
    #[test]
    fn parses_js_clear_color() {
        let color = color_from_json(Some(&json!({"r": 0.1, "g": 0.2, "b": 0.3, "a": 0.4})));
        assert!((color.r - 0.1).abs() < f64::EPSILON);
        assert!((color.a - 0.4).abs() < f64::EPSILON);
    }
    #[test]
    fn parses_js_topology() {
        assert_eq!(
            primitive_state(Some(&json!({"topology": "line-list"}))).topology,
            wgpu::PrimitiveTopology::LineList
        );
    }
}
