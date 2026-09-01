"use strict";

const canvas = document.createElement("canvas");
canvas.width = 640;
canvas.height = 480;
const context = canvas.getContext("webgpu");
const adapter = navigator.gpu.requestAdapter();
const device = adapter.requestDevice();
const format = navigator.gpu.getPreferredCanvasFormat();
context.configure({ device, format, alphaMode: "opaque" });

const shader = device.createShaderModule({
  code: `
    struct VertexOut {
      @builtin(position) position: vec4<f32>,
      @location(0) color: vec3<f32>,
    };
    @vertex fn vertexMain(@builtin(vertex_index) i: u32) -> VertexOut {
      var positions = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.75), vec2<f32>(-0.75, -0.75), vec2<f32>(0.75, -0.75)
      );
      var colors = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 0.2, 0.2), vec3<f32>(0.2, 1.0, 0.3), vec3<f32>(0.2, 0.4, 1.0)
      );
      var out: VertexOut;
      out.position = vec4<f32>(positions[i], 0.0, 1.0);
      out.color = colors[i];
      return out;
    }
    @fragment fn fragmentMain(in: VertexOut) -> @location(0) vec4<f32> {
      return vec4<f32>(in.color, 1.0);
    }
  `,
});

const pipeline = device.createRenderPipeline({
  layout: "auto",
  vertex: { module: shader, entryPoint: "vertexMain" },
  fragment: { module: shader, entryPoint: "fragmentMain", targets: [{ format }] },
  primitive: { topology: "triangle-list" },
});

const encoder = device.createCommandEncoder();
const view = context.getCurrentTexture().createView();
const pass = encoder.beginRenderPass({
  colorAttachments: [{
    view,
    clearValue: { r: 0.03, g: 0.04, b: 0.07, a: 1.0 },
    loadOp: "clear",
    storeOp: "store",
  }],
});
pass.setPipeline(pipeline);
pass.draw(3);
pass.end();
device.queue.submit([encoder.finish()]);

"WebGPU triangle rendered";
