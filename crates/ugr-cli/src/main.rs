use std::env;
use std::path::{Path, PathBuf};

use ugr_runtime::{Runtime, RuntimeConfig, V8Engine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Headless,
    WebGpu,
    WebGl,
    Canvas,
}

impl RenderMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "headless" | "none" => Ok(Self::Headless),
            "webgpu" | "wgpu" => Ok(Self::WebGpu),
            "webgl" | "webgl1" | "webgl2" => Ok(Self::WebGl),
            "canvas" | "canvas2d" => Ok(Self::Canvas),
            _ => Err(format!("unknown render mode '{value}'")),
        }
    }
}

struct ProjectConfig {
    entry: Option<PathBuf>,
    mode: RenderMode,
    runtime: RuntimeConfig,
}

fn load_project_config(path: &Path) -> Result<ProjectConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(ProjectConfig {
            entry: None,
            mode: RenderMode::Headless,
            runtime: RuntimeConfig::default(),
        });
    }
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let root = value
        .as_object()
        .ok_or("package.json must contain an object")?;
    let settings = root.get("ugr").and_then(serde_json::Value::as_object);
    let entry = settings
        .and_then(|v| v.get("entry"))
        .or_else(|| root.get("main"))
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from);
    let entry = entry.map(|entry| {
        if entry.is_absolute() {
            entry
        } else {
            path.parent().unwrap_or_else(|| Path::new(".")).join(entry)
        }
    });
    let mode_name = settings
        .and_then(|v| v.get("renderMode"))
        .or_else(|| root.get("renderMode"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("headless");
    let mut runtime = RuntimeConfig::default();
    if let Some(title) = settings
        .and_then(|v| v.get("title"))
        .and_then(serde_json::Value::as_str)
    {
        runtime.title = title.to_owned();
    }
    if let Some(width) = settings
        .and_then(|v| v.get("width"))
        .and_then(serde_json::Value::as_u64)
    {
        runtime.width = width as u32;
    }
    if let Some(height) = settings
        .and_then(|v| v.get("height"))
        .and_then(serde_json::Value::as_u64)
    {
        runtime.height = height as u32;
    }
    Ok(ProjectConfig {
        entry,
        mode: RenderMode::parse(mode_name)?,
        runtime,
    })
}

enum Renderer {
    WebGpu(Box<ugr_wgpu::WgpuRenderer>),
    WebGl(Box<ugr_webgl::GlRenderer>),
}

struct WindowApp {
    renderer: Renderer,
}

impl winit::application::ApplicationHandler for WindowApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        match &mut self.renderer {
            Renderer::WebGpu(renderer) => {
                if renderer.is_initialized() {
                    return;
                }
                if let Err(error) = pollster::block_on(renderer.initialize(event_loop)) {
                    eprintln!("WebGPU initialization failed: {error}");
                    event_loop.exit();
                }
            }
            Renderer::WebGl(renderer) => {
                if !renderer.is_initialized() {
                    if let Err(error) = renderer.initialize(event_loop) {
                        eprintln!("WebGL initialization failed: {error}");
                        event_loop.exit();
                    } else {
                        renderer.draw();
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            winit::event::WindowEvent::CloseRequested => event_loop.exit(),
            winit::event::WindowEvent::Resized(size) => {
                if let Renderer::WebGpu(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                } else if let Renderer::WebGl(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            winit::event::WindowEvent::RedrawRequested => match &mut self.renderer {
                Renderer::WebGpu(renderer) => renderer.draw(),
                Renderer::WebGl(renderer) => renderer.draw(),
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        match &self.renderer {
            Renderer::WebGpu(renderer) => renderer.request_redraw(),
            Renderer::WebGl(renderer) => renderer.request_redraw(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut script = None::<PathBuf>;
    let mut headless = false;
    let mut webgl = false;
    let mut webgpu = false;
    let mut package = PathBuf::from("package.json");
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--headless" => headless = true,
            "--webgl" => webgl = true,
            "--webgpu" => webgpu = true,
            "--script" => script = args.next().map(PathBuf::from),
            "--package" => {
                package = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--package requires a path")?;
            }
            value => {
                if script.is_none() {
                    script = Some(PathBuf::from(value));
                }
            }
        }
    }

    let project = load_project_config(&package)?;
    let config = project.runtime;
    let mode = if headless {
        RenderMode::Headless
    } else if webgl {
        RenderMode::WebGl
    } else if webgpu {
        RenderMode::WebGpu
    } else {
        project.mode
    };
    if script.is_none() {
        script = project.entry;
    }
    let mut runtime = Runtime::new(V8Engine::new(), config.clone());
    let mut commands = Vec::new();
    let mut webgpu_commands = Vec::new();
    let source = match script {
        Some(path) => std::fs::read_to_string(path)?,
        None => "'Hello from UGR'".to_owned(),
    };
    let result = match mode {
        RenderMode::WebGl => {
            let (result, values) = runtime.evaluate_with_webgl_commands(&source)?;
            commands = ugr_webgl::parse_commands(&values);
            result
        }
        RenderMode::WebGpu => {
            let (result, values) = runtime.evaluate_with_webgpu_commands(&source)?;
            webgpu_commands = values;
            result
        }
        RenderMode::Canvas => runtime.evaluate_with_canvas_commands(&source)?.0,
        RenderMode::Headless => runtime.evaluate(&source)?,
    };
    println!("{result}");

    if headless {
        return Ok(());
    }

    if mode == RenderMode::Headless || mode == RenderMode::Canvas {
        return Ok(());
    }

    let event_loop = winit::event_loop::EventLoop::new()?;
    let renderer = match mode {
        RenderMode::WebGpu => Renderer::WebGpu(Box::new(ugr_wgpu::WgpuRenderer::new(
            config.title.clone(),
            config.width,
            config.height,
            webgpu_commands,
        ))),
        RenderMode::WebGl => Renderer::WebGl(Box::new(ugr_webgl::GlRenderer::new(
            config.title.clone(),
            config.width,
            config.height,
            commands,
        ))),
        RenderMode::Headless | RenderMode::Canvas => unreachable!(),
    };
    let mut app = WindowApp { renderer };
    event_loop.run_app(&mut app)?;
    Ok(())
}
