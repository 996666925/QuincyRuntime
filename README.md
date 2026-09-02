# UGR

Universal Web Game Native Runtime, Phase 4.

The repository is a Cargo workspace with independent runtime and rendering crates:

- `ugr-runtime`: JavaScript engine, WebGPU/WebGL facade and resource manager.
- `ugr-wgpu`: native wgpu rendering backend.
- `ugr-webgl`: native glow/glutin WebGL rendering backend.
- `ugr-canvas`: Canvas 2D state and drawing command backend.
- `ugr-html`: HTML entry-point and script loader.
- `ugr-ui`: Taffy layout and Skia native HTML/UI renderer.
- `ugr-compositor`: shared event loop and multi-backend renderer compositor.
- `ugr-cli`: command-line host and window event loop.

This milestone provides:

- a Rust library facade for script execution and runtime configuration;
- a winit native window host on Windows and other supported desktop targets;
- a `deno_core` JavaScript backend enabled by default;
- a deterministic headless hello-world path for CI and development machines
  without the V8 build toolchain.
- a D3D12-first wgpu renderer that draws a native triangle in the window.
- a Phase 2 JavaScript bootstrap exposing `navigator.gpu`, an adapter, device and
  queue facade to game scripts.
- independent WebGL 1/WebGL 2 context and resource-handle contracts, including
  extensions, framebuffer/texture/shader/VAO resource tracking and draw-call
  accounting.
- a real glow + glutin GLES3 window backend selected with `--webgl`.
- a unified CLI renderer lifecycle: WebGPU and WebGL are both hosted by the
  same winit `WindowApp` renderer abstraction.

## Run

```bash
cargo run -- --headless
cargo run -- path/to/game.js
cargo run -- --webgl
cargo run -- --webgl --script tests/scripts/webgl2_triangle.js
cargo run -- --script index.html
cargo run -- --ui --script tests/scripts/calculator.html
```

When no `--script` is supplied, the CLI reads `package.json` in the current
directory. `main` selects the entry script and `ugr.renderMode` selects the
host backend (`webgpu`, `webgl`, `canvas`, `ui`, or `headless`):

```json
{
  "main": "src/game.js",
  "ugr": {
    "renderMode": "webgpu",
    "title": "My Game",
    "width": 1280,
    "height": 720
  }
}
```

Use `ugr.renderModes` when more than one native layer is required. The listed
backends are evaluated from the same entry and driven by the shared compositor
event loop:

```json
{
  "main": "src/game.html",
  "ugr": {
    "renderModes": ["webgpu", "ui"],
    "width": 1280,
    "height": 720
  }
}
```

`--script`, `--webgpu`, `--webgl`, and `--headless` override the package
configuration. If no package file or render mode is present, scripts run in
headless mode and no graphics backend is initialized. The compositor presents
all configured layers in one native window. WebGL and WebGPU command layers are
rasterized into the shared composition buffer, then the Skia UI layer is
alpha-composited in `renderModes` order.

An entry may also be an HTML page. The runtime extracts inline scripts and
local `<script src="..."></script>` files in document order; remote scripts
are rejected.

The HTML bootstrap provides a lightweight game-oriented DOM: `div`, `span`,
`input`, and `canvas` elements; `getElementById`, `querySelector` and
`querySelectorAll`; attributes, text, `appendChild`/`removeChild`, basic input
focus/click events, inline styles, and simple tag/class/id CSS rules through
`getComputedStyle`. It is not a full browser layout engine. Native UI mode
computes box geometry with Taffy and paints backgrounds, borders, text, inputs
and buttons with Skia before presenting the raster surface in a native window.

Scripts can query the WebGPU bootstrap during startup:

```js
const adapter = navigator.gpu.requestAdapter();
const device = adapter.requestDevice();
device.queue.submit([]);
```

The same JavaScript bootstrap exposes the Canvas WebGL entry points:

```js
const canvas = document.createElement("canvas");
const gl = canvas.getContext("webgl2");
gl.clearColor(0, 0, 0, 1);
gl.drawArrays(gl.TRIANGLES, 0, 3);
```

Test scripts are available under `tests/scripts`:

```bash
cargo run -- --headless --script tests/scripts/hello.js
cargo run -- --headless --script tests/scripts/webgpu_triangle.js
cargo run -- --headless --script tests/scripts/webgl1_triangle.js
cargo run -- --headless --script tests/scripts/webgl2_triangle.js
cargo run -- --headless --script tests/scripts/webgl2_triangle.html
cargo run -- --webgl --script tests/scripts/webgl2_triangle.html
cargo run -- --headless --script tests/scripts/calculator.html
cargo run -- --ui --script tests/scripts/calculator.html
cargo run -- --headless --script tests/scripts/canvas2d.js
cargo run -- --headless --script tests/scripts/extensions.js
```

The native ANGLE/EGL backend remains an explicit integration boundary; the
WebGL state layer is kept independent from the wgpu WebGPU backend. The
single-window compositor owns presentation and keeps backend layers isolated.

The first command prints `Hello from UGR` without opening a window. The
windowed command keeps the native window alive until it is closed. The
`deno_core` downloads its pinned V8 runtime and requires the native build
prerequisites described by that crate.

## Verify

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The project intentionally keeps JavaScript execution behind the `JsEngine`
trait so later Web Platform layers do not depend on a particular game engine.
