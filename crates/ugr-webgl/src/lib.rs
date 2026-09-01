use std::ffi::CString;
use std::num::NonZeroU32;

use glow::HasContext;
use glutin::config::GlConfig as _;
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext as _, Version};
use glutin::display::{GetGlDisplay as _, GlDisplay as _};
use glutin::prelude::GlSurface as _;
use glutin::surface::{Surface, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow as _};
use raw_window_handle::HasWindowHandle as _;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::Window;

#[derive(Debug, Clone, Copy)]
pub enum GlCommand {
    ClearColor(f32, f32, f32, f32),
    Clear(u32),
    Viewport(i32, i32, i32, i32),
    DrawArrays(i32, i32, i32),
    DrawElements(i32, i32, i32, i32),
}

pub fn parse_commands(values: &[serde_json::Value]) -> Vec<GlCommand> {
    values
        .iter()
        .filter_map(|value| {
            let op = value.get("op")?.as_str()?;
            let args = value.get("args").and_then(serde_json::Value::as_array);
            let number = |index: usize| args?.get(index)?.as_f64().map(|value| value as f32);
            match op {
                "clearColor" => Some(GlCommand::ClearColor(
                    number(0)?,
                    number(1)?,
                    number(2)?,
                    number(3)?,
                )),
                "clear" => Some(GlCommand::Clear(
                    number(0).unwrap_or(glow::COLOR_BUFFER_BIT as f32) as u32,
                )),
                "viewport" => Some(GlCommand::Viewport(
                    number(0)? as i32,
                    number(1)? as i32,
                    number(2)? as i32,
                    number(3)? as i32,
                )),
                "drawArrays" => Some(GlCommand::DrawArrays(
                    number(0)? as i32,
                    number(1)? as i32,
                    number(2)? as i32,
                )),
                "drawElements" => Some(GlCommand::DrawElements(
                    number(0)? as i32,
                    number(1)? as i32,
                    number(2)? as i32,
                    number(3)? as i32,
                )),
                _ => None,
            }
        })
        .collect()
}

const VERTEX_SHADER: &str = r#"#version 300 es
in vec2 a_position;
in vec3 a_color;
out vec3 v_color;
void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_color = a_color;
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 300 es
precision highp float;
in vec3 v_color;
out vec4 color;
void main() { color = vec4(v_color, 1.0); }
"#;

struct GlState {
    context: glutin::context::PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    _vbo: glow::Buffer,
}

pub struct GlRenderer {
    title: String,
    width: u32,
    height: u32,
    state: Option<GlState>,
    commands: Vec<GlCommand>,
}

impl GlRenderer {
    pub fn new(title: String, width: u32, height: u32, commands: Vec<GlCommand>) -> Self {
        Self {
            title,
            width,
            height,
            state: None,
            commands,
        }
    }

    pub fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let attributes = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(attributes));
        let (window, config) = display_builder
            .build(
                event_loop,
                glutin::config::ConfigTemplateBuilder::new(),
                |configs| {
                    configs
                        .max_by_key(|config| config.num_samples())
                        .expect("no OpenGL configuration available")
                },
            )
            .map_err(|error| format!("failed to create OpenGL display: {error}"))?;
        let window = window.ok_or_else(|| "OpenGL backend did not create a window".to_owned())?;
        let raw_handle = window.window_handle().ok().map(|handle| handle.as_raw());
        let context_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .build(raw_handle);
        let not_current = unsafe {
            config
                .display()
                .create_context(&config, &context_attributes)
                .map_err(|error| format!("failed to create GLES context: {error}"))?
        };
        let surface_attributes = window
            .build_surface_attributes(Default::default())
            .map_err(|error| format!("failed to build surface attributes: {error}"))?;
        let surface = unsafe {
            config
                .display()
                .create_window_surface(&config, &surface_attributes)
                .map_err(|error| format!("failed to create window surface: {error}"))?
        };
        let context = not_current
            .make_current(&surface)
            .map_err(|error| format!("failed to activate GLES context: {error}"))?;
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                config
                    .display()
                    .get_proc_address(&CString::new(name).expect("GL symbol contains NUL"))
            })
        };
        let (program, vao, vbo) = create_triangle(&gl)?;
        self.state = Some(GlState {
            context,
            surface,
            window,
            gl,
            program,
            vao,
            _vbo: vbo,
        });
        Ok(())
    }

    pub fn draw(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let size = state.window.inner_size();
        unsafe {
            state
                .gl
                .viewport(0, 0, size.width as i32, size.height as i32);
            state.gl.clear_color(0.03, 0.04, 0.07, 1.0);
            state.gl.clear(glow::COLOR_BUFFER_BIT);
            state.gl.use_program(Some(state.program));
            state.gl.bind_vertex_array(Some(state.vao));
            if self.commands.is_empty() {
                state.gl.draw_arrays(glow::TRIANGLES, 0, 3);
            } else {
                for command in &self.commands {
                    match *command {
                        GlCommand::ClearColor(r, g, b, a) => state.gl.clear_color(r, g, b, a),
                        GlCommand::Clear(mask) => state.gl.clear(mask),
                        GlCommand::Viewport(x, y, width, height) => {
                            // The JS canvas facade defaults to 1x1 until a game
                            // sets its size. Map that default to the native surface.
                            if width == 1 && height == 1 {
                                state
                                    .gl
                                    .viewport(0, 0, size.width as i32, size.height as i32);
                            } else {
                                state.gl.viewport(x, y, width, height)
                            }
                        }
                        GlCommand::DrawArrays(mode, first, count) => {
                            state.gl.draw_arrays(mode as u32, first, count)
                        }
                        GlCommand::DrawElements(mode, count, kind, offset) => {
                            // Index-buffer binding is not wired yet. Avoid sending
                            // an invalid indexed draw to the driver until it is.
                            let _ = (kind, offset);
                            state.gl.draw_arrays(mode as u32, 0, count);
                        }
                    }
                }
            }
            state.gl.bind_vertex_array(None);
            state.gl.use_program(None);
            let error = state.gl.get_error();
            if error != glow::NO_ERROR {
                eprintln!("OpenGL error after command replay: 0x{error:04x}");
            }
            state.gl.flush();
        }
        if let Err(error) = state.surface.swap_buffers(&state.context) {
            eprintln!("OpenGL swap failed: {error}");
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.state.is_some()
    }

    pub fn request_redraw(&self) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            state.surface.resize(&state.context, width, height);
        }
    }
}

impl ApplicationHandler for GlRenderer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            if let Err(error) = self.initialize(event_loop) {
                eprintln!("OpenGL initialization failed: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

pub fn run(title: String, width: u32, height: u32, commands: Vec<GlCommand>) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut app = GlRenderer::new(title, width, height, commands);
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())
}

fn create_triangle(
    gl: &glow::Context,
) -> Result<(glow::Program, glow::VertexArray, glow::Buffer), String> {
    unsafe {
        let vertex = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|error| format!("create vertex shader: {error}"))?;
        gl.shader_source(vertex, VERTEX_SHADER);
        gl.compile_shader(vertex);
        if !gl.get_shader_compile_status(vertex) {
            return Err(gl.get_shader_info_log(vertex));
        }
        let fragment = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|error| format!("create fragment shader: {error}"))?;
        gl.shader_source(fragment, FRAGMENT_SHADER);
        gl.compile_shader(fragment);
        if !gl.get_shader_compile_status(fragment) {
            return Err(gl.get_shader_info_log(fragment));
        }
        let program = gl
            .create_program()
            .map_err(|error| format!("create program: {error}"))?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.link_program(program);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
        if !gl.get_program_link_status(program) {
            return Err(gl.get_program_info_log(program));
        }
        let vao = gl
            .create_vertex_array()
            .map_err(|error| format!("create vertex array: {error}"))?;
        let vbo = gl
            .create_buffer()
            .map_err(|error| format!("create vertex buffer: {error}"))?;
        let vertices: [f32; 15] = [
            0.0, 0.72, 1.0, 0.2, 0.2, -0.72, -0.72, 0.2, 1.0, 0.3, 0.72, -0.72, 0.2, 0.4, 1.0,
        ];
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let bytes = std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            vertices.len() * std::mem::size_of::<f32>(),
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        let position = gl
            .get_attrib_location(program, "a_position")
            .ok_or_else(|| "a_position attribute not found".to_owned())?;
        gl.enable_vertex_attrib_array(position);
        gl.vertex_attrib_pointer_f32(position, 2, glow::FLOAT, false, 20, 0);
        let color = gl
            .get_attrib_location(program, "a_color")
            .ok_or_else(|| "a_color attribute not found".to_owned())?;
        gl.enable_vertex_attrib_array(color);
        gl.vertex_attrib_pointer_f32(color, 3, glow::FLOAT, false, 20, 8);
        gl.bind_vertex_array(None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        Ok((program, vao, vbo))
    }
}
