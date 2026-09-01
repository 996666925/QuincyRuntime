"use strict";

const canvas = document.createElement("canvas");
const gl = canvas.getContext("webgl2");
if (!gl || gl.version !== "webgl2") {
  throw new Error("WebGL2 context is unavailable");
}

const vertexShader = gl.createShader(gl.VERTEX_SHADER);
gl.shaderSource(vertexShader, "#version 300 es\nin vec2 a_position; in vec3 a_color; out vec3 v_color; void main(){gl_Position=vec4(a_position,0.0,1.0);v_color=a_color;}");
gl.compileShader(vertexShader);
const fragmentShader = gl.createShader(gl.FRAGMENT_SHADER);
gl.shaderSource(fragmentShader, "#version 300 es\nprecision highp float; in vec3 v_color; out vec4 color; void main(){color=vec4(v_color,1.0);}");
gl.compileShader(fragmentShader);
const program = gl.createProgram();
gl.attachShader(program, vertexShader);
gl.attachShader(program, fragmentShader);
gl.linkProgram(program);
gl.useProgram(program);

const vao = gl.createVertexArray();
gl.bindVertexArray(vao);
const buffer = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
  0.0, 0.75, 1.0, 0.2, 0.2,
  -0.75, -0.75, 0.2, 1.0, 0.3,
  0.75, -0.75, 0.2, 0.4, 1.0,
]), gl.STATIC_DRAW);
const position = gl.getAttribLocation(program, "a_position");
const color = gl.getAttribLocation(program, "a_color");
gl.enableVertexAttribArray(position);
gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 20, 0);
gl.enableVertexAttribArray(color);
gl.vertexAttribPointer(color, 3, gl.FLOAT, false, 20, 8);
gl.viewport(0, 0, canvas.width || 1, canvas.height || 1);
gl.clearColor(0.05, 0.05, 0.08, 1.0);
gl.clear(gl.COLOR_BUFFER_BIT);
gl.drawArrays(gl.TRIANGLES, 0, 3);

"WebGL2 triangle rendered";
