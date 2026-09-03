//! The first, deliberately small layer of the Universal Game Runtime.
//!
//! The public API keeps JavaScript execution independent from the windowing
//! backend. Later phases can add WebGPU, WebGL and Web APIs without coupling
//! them to the game engine being hosted.

use std::path::Path;

/// Errors returned by the runtime bootstrap layer.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("could not read script {path}: {source}")]
    ScriptRead {
        path: String,
        source: std::io::Error,
    },
    #[error("JavaScript error: {0}")]
    JavaScript(String),
}

/// The JavaScript execution contract used by the host runtime.
pub trait JsEngine {
    fn evaluate(&mut self, source: &str) -> Result<String, RuntimeError>;
}

pub trait WebGlJsEngine: JsEngine {
    fn evaluate_with_webgl_commands(
        &mut self,
        source: &str,
    ) -> Result<(String, Vec<serde_json::Value>), RuntimeError>;
}

/// Optional command extraction for scripts using the WebGPU facade.
pub trait WebGpuJsEngine: JsEngine {
    fn evaluate_with_webgpu_commands(
        &mut self,
        source: &str,
    ) -> Result<(String, Vec<serde_json::Value>), RuntimeError>;
}

pub trait Canvas2dJsEngine: JsEngine {
    fn evaluate_with_canvas_commands(
        &mut self,
        source: &str,
    ) -> Result<(String, Vec<serde_json::Value>), RuntimeError>;
}

/// Runtime configuration shared by headless and windowed hosts.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            title: "Universal Game Runtime".to_owned(),
            width: 1280,
            height: 720,
        }
    }
}

/// Phase 0 runtime facade.
pub struct Runtime<E> {
    engine: E,
    config: RuntimeConfig,
}

impl<E: JsEngine> Runtime<E> {
    pub fn new(engine: E, config: RuntimeConfig) -> Self {
        Self { engine, config }
    }

    pub fn evaluate(&mut self, source: &str) -> Result<String, RuntimeError> {
        self.engine.evaluate(source)
    }

    pub fn evaluate_file<P: AsRef<Path>>(&mut self, path: P) -> Result<String, RuntimeError> {
        let path_ref = path.as_ref();
        let source =
            std::fs::read_to_string(path_ref).map_err(|source| RuntimeError::ScriptRead {
                path: path_ref.display().to_string(),
                source,
            })?;
        self.evaluate(&source)
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }
}

impl<E: WebGlJsEngine> Runtime<E> {
    pub fn evaluate_with_webgl_commands(
        &mut self,
        source: &str,
    ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
        self.engine.evaluate_with_webgl_commands(source)
    }
}

impl<E: WebGpuJsEngine> Runtime<E> {
    pub fn evaluate_with_webgpu_commands(
        &mut self,
        source: &str,
    ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
        self.engine.evaluate_with_webgpu_commands(source)
    }
}

impl<E: Canvas2dJsEngine> Runtime<E> {
    pub fn evaluate_with_canvas_commands(
        &mut self,
        source: &str,
    ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
        self.engine.evaluate_with_canvas_commands(source)
    }
}

pub use deno_engine::V8Engine;
pub type DenoEngine = V8Engine;

mod deno_engine {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use deno_core::OpState;

    use super::{Canvas2dJsEngine, WebGlJsEngine, WebGpuJsEngine};
    use super::{JsEngine, RuntimeError};

    const JS_BOOTSTRAP: &str = include_str!("bootstrap.js");

    #[derive(Default)]
    struct ResourceManager {
        next_handle: u32,
        buffers: HashMap<u32, Vec<u8>>,
        textures: HashMap<u32, Vec<u8>>,
        shaders: HashMap<u32, String>,
        pipelines: HashMap<u32, String>,
    }

    impl ResourceManager {
        fn allocate(&mut self) -> u32 {
            self.next_handle = self.next_handle.saturating_add(1);
            self.next_handle
        }
    }

    #[deno_core::op2(fast)]
    fn op_alloc_resource(state: Rc<RefCell<OpState>>) -> u32 {
        state
            .borrow_mut()
            .borrow_mut::<ResourceManager>()
            .allocate()
    }

    #[deno_core::op2(fast)]
    fn op_upload_buffer(
        state: Rc<RefCell<OpState>>,
        buffer: u32,
        _target: u32,
        _usage: u32,
        #[buffer] data: &[u8],
    ) {
        state
            .borrow_mut()
            .borrow_mut::<ResourceManager>()
            .buffers
            .insert(buffer, data.to_vec());
    }

    #[deno_core::op2(fast)]
    fn op_upload_texture(state: Rc<RefCell<OpState>>, texture: u32, #[buffer] data: &[u8]) {
        state
            .borrow_mut()
            .borrow_mut::<ResourceManager>()
            .textures
            .insert(texture, data.to_vec());
    }

    #[deno_core::op2(fast)]
    fn op_set_shader_source(state: Rc<RefCell<OpState>>, shader: u32, #[string] source: String) {
        state
            .borrow_mut()
            .borrow_mut::<ResourceManager>()
            .shaders
            .insert(shader, source);
    }

    #[deno_core::op2(fast)]
    fn op_register_pipeline(state: Rc<RefCell<OpState>>, pipeline: u32, #[string] source: String) {
        state
            .borrow_mut()
            .borrow_mut::<ResourceManager>()
            .pipelines
            .insert(pipeline, source);
    }

    #[deno_core::op2(fast)]
    fn op_release_resource(state: Rc<RefCell<OpState>>, resource: u32) {
        let mut state = state.borrow_mut();
        let manager = state.borrow_mut::<ResourceManager>();
        manager.buffers.remove(&resource);
        manager.textures.remove(&resource);
        manager.shaders.remove(&resource);
        manager.pipelines.remove(&resource);
    }

    #[deno_core::op2(fast)]
    fn op_release_buffer(state: Rc<RefCell<OpState>>, buffer: u32) {
        state
            .borrow_mut()
            .borrow_mut::<ResourceManager>()
            .buffers
            .remove(&buffer);
    }

    deno_core::extension!(
        ugr_runtime_ext,
        ops = [
            op_alloc_resource,
            op_upload_buffer,
            op_upload_texture,
            op_set_shader_source,
            op_register_pipeline,
            op_release_buffer,
            op_release_resource,
        ],
        state = |state| state.put(ResourceManager::default()),
    );

    pub struct V8Engine {
        runtime: deno_core::JsRuntime,
        initialized: bool,
    }

    impl V8Engine {
        pub fn new() -> Self {
            Self {
                runtime: deno_core::JsRuntime::new(deno_core::RuntimeOptions {
                    extensions: vec![ugr_runtime_ext::init()],
                    ..Default::default()
                }),
                initialized: false,
            }
        }
    }

    impl WebGlJsEngine for V8Engine {
        fn evaluate_with_webgl_commands(
            &mut self,
            source: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            self.evaluate_with_webgl_commands(source)
        }
    }

    impl WebGpuJsEngine for V8Engine {
        fn evaluate_with_webgpu_commands(
            &mut self,
            source: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            self.evaluate_with_webgpu_commands(source)
        }
    }

    impl Canvas2dJsEngine for V8Engine {
        fn evaluate_with_canvas_commands(
            &mut self,
            source: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            self.evaluate_with_canvas_commands(source)
        }
    }

    impl Default for V8Engine {
        fn default() -> Self {
            Self::new()
        }
    }

    impl JsEngine for V8Engine {
        fn evaluate(&mut self, source: &str) -> Result<String, RuntimeError> {
            self.evaluate_internal(source)
        }
    }

    impl V8Engine {
        pub fn evaluate_with_webgl_commands(
            &mut self,
            source: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            self.evaluate_with_queue(source, "__ugr_webgl_commands")
        }

        pub fn evaluate_with_webgpu_commands(
            &mut self,
            source: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            self.evaluate_with_queue(source, "__ugr_webgpu_commands")
        }

        pub fn evaluate_with_canvas_commands(
            &mut self,
            source: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            self.evaluate_with_queue(source, "__ugr_canvas_commands")
        }

        fn evaluate_with_queue(
            &mut self,
            source: &str,
            queue_name: &str,
        ) -> Result<(String, Vec<serde_json::Value>), RuntimeError> {
            let source_literal = serde_json::to_string(source).map_err(|error| {
                RuntimeError::JavaScript(format!("could not encode script source: {error}"))
            })?;
            let wrapper = format!(
                "(() => {{ {queue_name}.length = 0; const __result = eval({source_literal}); return JSON.stringify({{ result: String(__result), commands: {queue_name} }}); }})()"
            );
            let encoded = self.evaluate_internal(&wrapper)?;
            let value: serde_json::Value = serde_json::from_str(&encoded).map_err(|error| {
                RuntimeError::JavaScript(format!("invalid command result: {error}"))
            })?;
            let result = value
                .get("result")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let commands = value
                .get("commands")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .ok_or_else(|| RuntimeError::JavaScript("invalid command queue".into()))?;
            Ok((result, commands))
        }
    }

    impl V8Engine {
        fn evaluate_internal(&mut self, source: &str) -> Result<String, RuntimeError> {
            let source = if self.initialized {
                source.to_owned()
            } else {
                self.initialized = true;
                format!("{JS_BOOTSTRAP}\n{source}")
            };
            let value = self
                .runtime
                .execute_script("<ugr>", source)
                .map_err(|error| RuntimeError::JavaScript(error.to_string()))?;
            deno_core::scope!(scope, self.runtime);
            let value = deno_core::v8::Local::new(scope, value);
            let value = value.to_string(scope).ok_or_else(|| {
                RuntimeError::JavaScript("script returned no string value".into())
            })?;
            Ok(value.to_rust_string_lossy(scope))
        }

        #[cfg(test)]
        pub(super) fn uploaded_buffer_len(&self, handle: u32) -> Option<usize> {
            self.runtime
                .op_state()
                .borrow()
                .borrow::<ResourceManager>()
                .buffers
                .get(&handle)
                .map(Vec::len)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_stable() {
        let config = RuntimeConfig::default();
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
    }

    #[test]
    fn headless_hello_world() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        assert_eq!(
            runtime.evaluate("'Hello from UGR'").unwrap(),
            "Hello from UGR"
        );
    }

    #[test]
    fn webgpu_namespace_is_available() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        assert_eq!(
            runtime
                .evaluate("navigator.gpu.getPreferredCanvasFormat()")
                .unwrap(),
            "bgra8unorm-srgb"
        );
        assert_eq!(
            runtime
                .evaluate("document.createElement('canvas').getContext('webgl2').version")
                .unwrap(),
            "webgl2"
        );
    }

    #[test]
    fn dom_facade_supports_basic_nodes_styles_and_events() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let result = runtime
            .evaluate(
                r#"__ugr_install_document('<body><div id="root"><span class="title">Hello</span><input value="x"></div><style>.title { color: red; }</style></body>'); const root=document.getElementById('root'); const title=root.querySelector('.title'); const input=root.querySelector('input'); let clicked=0; input.addEventListener('click',()=>clicked++); input.click(); title.setAttribute('data-ready','yes'); `${title.textContent}|${title.getAttribute('data-ready')}|${input.value}|${clicked}|${getComputedStyle(title).getPropertyValue('color')}`"#,
            )
            .unwrap();
        assert_eq!(result, "Hello|yes|x|1|red");
    }

    #[test]
    fn dom_facade_supports_selectors_mutation_and_event_bubbling() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let result = runtime
            .evaluate(
                r#"__ugr_install_document('<body><div id="panel"><span class="label" data-kind="score">0</span></div><style>#panel .label[data-kind=score] { font-size: 12px; }</style></body>'); const panel=document.querySelector('#panel'); const label=document.querySelector('#panel .label[data-kind=score]'); const button=document.createElement('input'); button.type='button'; button.classList.add('primary'); button.dataset.action='start'; let events=[]; panel.addEventListener('click',()=>events.push('panel')); button.addEventListener('click',()=>events.push('button')); panel.appendChild(button); button.click(); label.textContent='10'; button.style.color='blue'; `${panel.querySelectorAll('input.primary').length}|${button.dataset.action}|${label.textContent}|${events.join(',')}|${getComputedStyle(label).fontSize || getComputedStyle(label).getPropertyValue('font-size')}|${button.style.color}`"#,
            )
            .unwrap();
        assert_eq!(result, "1|start|10|button,panel|12px|blue");
    }

    #[test]
    fn native_click_dispatches_to_dom_path() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let result = runtime
            .evaluate(
                r#"__ugr_install_document('<html><head><style>.hidden { display: none; }</style></head><body><button id="run">Run</button></body></html>'); const button=document.getElementById('run'); button.addEventListener('click',()=>button.textContent='Done'); __ugr_dispatch_click([0, 0, 0]); button.textContent"#,
            )
            .unwrap();
        assert_eq!(result, "Done");
    }

    #[test]
    fn native_event_dispatch_supports_keyboard_and_prevent_default() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let result = runtime
            .evaluate(
                r#"__ugr_install_document('<body><input value="x"></body>'); const input=document.querySelector('input'); let seen=''; input.addEventListener('keydown',(event)=>{seen=event.key; event.preventDefault();}); __ugr_dispatch_ui_event([0,0], {type:'keydown', key:'Enter', code:'Enter'}); `${seen}|${input.value}`"#,
            )
            .unwrap();
        assert_eq!(result, "Enter|x");
    }

    #[test]
    fn native_event_targets_survive_dom_reordering() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let result = runtime
            .evaluate(
                r#"__ugr_install_document('<body><button>A</button><button>B</button></body>'); const buttons=document.querySelectorAll('button'); const key=buttons[1].getAttribute('data-ugr-id'); let clicked=''; buttons[1].addEventListener('click',()=>clicked=buttons[1].textContent); buttons[0].parentNode.insertBefore(buttons[1], buttons[0]); __ugr_dispatch_ui_event({key}, {type:'click'}); clicked"#,
            )
            .unwrap();
        assert_eq!(result, "B");
    }

    #[test]
    fn serialized_dom_keeps_styles_and_mutated_input_value() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let result = runtime
            .evaluate(
                r##"__ugr_install_document('<html><head><style>.panel { background: #20242b; }</style></head><body><div class="panel"><input value="0"></div></body></html>'); document.querySelector('input').value='15'; document.documentElement.outerHTML"##,
            )
            .unwrap();
        assert!(result.contains("background: #20242b"));
        assert!(result.contains("value=\"15\""));
    }

    #[test]
    fn webgl_exports_stateful_commands_and_script_result() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let (result, commands) = runtime
            .evaluate_with_webgl_commands(
                "const c=document.createElement('canvas'); const gl=c.getContext('webgl2'); const b=gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER,b); gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([0,1,2]),gl.STATIC_DRAW); gl.clearColor(1,0,0,1); gl.clear(gl.COLOR_BUFFER_BIT); gl.drawArrays(gl.TRIANGLES,0,3); 'ok';",
            )
            .unwrap();
        assert_eq!(result, "ok");
        assert!(commands.iter().any(|v| v["op"] == "bufferData"));
        assert!(commands.iter().any(|v| v["op"] == "drawArrays"));
    }

    #[test]
    fn webgpu_resources_encode_commands() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let (result, commands) = runtime
            .evaluate_with_webgpu_commands("const a=navigator.gpu.requestAdapter(); const d=a.requestDevice(); const b=d.createBuffer({size:64,usage:4}); const s=d.createShaderModule({code:'@vertex fn main()->@builtin(position) vec4f{return vec4f(0.0);}'}); const p=d.createRenderPipeline({vertex:{module:s}}); const e=d.createCommandEncoder(); const pass=e.beginRenderPass({}); pass.setPipeline(p); pass.draw(3); pass.end(); d.queue.writeBuffer(b,0,new Uint8Array([1,2,3])); d.queue.submit([e.finish()]); 'ok'")
            .unwrap();
        assert_eq!(result, "ok");
        assert!(commands.iter().any(|v| v["op"] == "createRenderPipeline"));
        assert!(commands.iter().any(|v| v["op"] == "draw"));
        assert!(commands.iter().any(|v| v["op"] == "writeBuffer"));
    }

    #[test]
    fn typed_array_upload_is_retained_in_the_native_resource_manager() {
        let mut engine = V8Engine::new();
        let handle: u32 = engine
            .evaluate("const gl=document.createElement('canvas').getContext('webgl'); const b=gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER,b); gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([1,2,3]),gl.STATIC_DRAW); b.id")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(engine.uploaded_buffer_len(handle), Some(12));
    }

    #[test]
    fn canvas2d_records_drawing_commands() {
        let mut runtime = Runtime::new(V8Engine::new(), RuntimeConfig::default());
        let (result, commands) = runtime
            .evaluate_with_canvas_commands("const c=document.createElement('canvas'); const ctx=c.getContext('2d'); ctx.fillStyle='#ff0000'; ctx.globalAlpha=0.5; ctx.fillRect(10,20,30,40); 'ok'")
            .unwrap();
        assert_eq!(result, "ok");
        assert_eq!(commands[0]["op"], "fillRect");
        assert_eq!(commands[0]["args"][0], 10);
    }
}
