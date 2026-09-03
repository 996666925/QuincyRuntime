use std::env;
use std::path::{Path, PathBuf};

use ugr_runtime::{Runtime, RuntimeConfig, V8Engine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Headless,
    WebGpu,
    WebGl,
    Canvas,
    Ui,
}

impl RenderMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "headless" | "none" => Ok(Self::Headless),
            "webgpu" | "wgpu" => Ok(Self::WebGpu),
            "webgl" | "webgl1" | "webgl2" => Ok(Self::WebGl),
            "canvas" | "canvas2d" => Ok(Self::Canvas),
            "ui" | "html" | "native" => Ok(Self::Ui),
            _ => Err(format!("unknown render mode '{value}'")),
        }
    }
}

struct ProjectConfig {
    entry: Option<PathBuf>,
    modes: Vec<RenderMode>,
    runtime: RuntimeConfig,
}

fn load_project_config(path: &Path) -> Result<ProjectConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(ProjectConfig {
            entry: None,
            modes: vec![RenderMode::Headless],
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
    let mut modes = match settings
        .and_then(|v| v.get("renderModes"))
        .or_else(|| root.get("renderModes"))
        .and_then(serde_json::Value::as_array)
    {
        Some(values) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "renderModes entries must be strings".to_owned())
                    .and_then(RenderMode::parse)
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    if modes.is_empty() {
        modes.push(RenderMode::parse(mode_name)?);
    }
    Ok(ProjectConfig {
        entry,
        modes,
        runtime,
    })
}

use ugr_compositor::{Compositor, Layer};
mod ui_bridge;
use ui_bridge::UiEventBridge;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // deno_core may enqueue timers while bootstrapping. Keep a Tokio context
    // active for the complete JS and native rendering lifetime.
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let _tokio_guard = tokio_runtime.enter();
    let mut script = None::<PathBuf>;
    let mut headless = false;
    let mut webgl = false;
    let mut webgpu = false;
    let mut ui = false;
    let mut package = PathBuf::from("package.json");
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--headless" => headless = true,
            "--webgl" => webgl = true,
            "--webgpu" => webgpu = true,
            "--ui" | "--html" => ui = true,
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
    let modes = if headless {
        vec![RenderMode::Headless]
    } else if webgl {
        vec![RenderMode::WebGl]
    } else if webgpu {
        vec![RenderMode::WebGpu]
    } else if ui {
        vec![RenderMode::Ui]
    } else {
        project.modes
    };
    if script.is_none() {
        script = project.entry;
    }
    let mut runtime = Runtime::new(V8Engine::new(), config.clone());
    let entry_path = script.clone();
    let source = match script {
        Some(path) => ugr_html::load_entry(path)?,
        None => "'Hello from UGR'".to_owned(),
    };
    let mut result = String::new();
    let mut webgl_commands = Vec::new();
    let mut webgpu_commands = Vec::new();
    let mut has_windowed_renderer = false;
    for mode in &modes {
        let current = match mode {
            RenderMode::WebGl => {
                let (value, values) = runtime.evaluate_with_webgl_commands(&source)?;
                webgl_commands = ugr_webgl::parse_commands(&values);
                has_windowed_renderer = true;
                value
            }
            RenderMode::WebGpu => {
                let (value, values) = runtime.evaluate_with_webgpu_commands(&source)?;
                webgpu_commands = values;
                has_windowed_renderer = true;
                value
            }
            RenderMode::Canvas => runtime.evaluate_with_canvas_commands(&source)?.0,
            RenderMode::Headless | RenderMode::Ui => {
                let value = runtime.evaluate(&source)?;
                if *mode == RenderMode::Ui {
                    has_windowed_renderer = true;
                }
                value
            }
        };
        result = current;
    }
    let ui_markup = if modes.contains(&RenderMode::Ui) {
        let serialized = runtime
            .evaluate("document?.documentElement?.outerHTML || document?.outerHTML || ''")?;
        (!serialized.is_empty()).then_some(serialized)
    } else {
        None
    };
    println!("{result}");

    if headless {
        return Ok(());
    }

    if !has_windowed_renderer {
        return Ok(());
    }

    let event_loop = winit::event_loop::EventLoop::new()?;
    let mut layers = Vec::new();
    for mode in modes {
        match mode {
            RenderMode::WebGpu => layers.push(Layer::WebGpu(webgpu_commands.clone())),
            RenderMode::WebGl => layers.push(Layer::WebGl(webgl_commands.clone())),
            RenderMode::Ui => {
                let path = entry_path
                    .as_ref()
                    .ok_or("UI mode requires an HTML entry file")?;
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                if !extension.eq_ignore_ascii_case("html") && !extension.eq_ignore_ascii_case("htm")
                {
                    return Err("UI mode requires an .html or .htm entry file".into());
                }
                let markup = ui_markup.clone().unwrap_or(std::fs::read_to_string(path)?);
                let document = ugr_html::parse_document(&markup);
                layers.push(Layer::Ui(ugr_ui::UiRenderer::from_document(
                    &document,
                    config.width,
                    config.height,
                )?));
            }
            RenderMode::Headless | RenderMode::Canvas => {}
        }
    }
    let mut app = Compositor::new(config.title.clone(), config.width, config.height, layers)?;
    app.set_ui_event_handler(UiEventBridge::new(runtime).into_handler());
    event_loop.run_app(&mut app)?;
    Ok(())
}
