"use strict";

const canvas = document.createElement("canvas");
const gl = canvas.getContext("webgl");
if (!gl || gl.version !== "webgl") {
  throw new Error("WebGL1 context is unavailable");
}

const vertexShader = gl.createShader(gl.VERTEX_SHADER);
gl.shaderSource(vertexShader, "attribute vec2 a_position; attribute vec3 a_color; varying vec3 v_color; void main(){gl_Position=vec4(a_position,0.0,1.0);v_color=a_color;}");
gl.compileShader(vertexShader);
const fragmentShader = gl.createShader(gl.FRAGMENT_SHADER);
gl.shaderSource(fragmentShader, "precision mediump float; varying vec3 v_color; void main(){gl_FragColor=vec4(v_color,1.0);}");
gl.compileShader(fragmentShader);
const program = gl.createProgram();
gl.attachShader(program, vertexShader);
gl.attachShader(program, fragmentShader);
gl.linkProgram(program);
gl.useProgram(program);

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
gl.clearColor(0.05, 0.05, 0.08, 1.0);
gl.clear(gl.COLOR_BUFFER_BIT);
gl.drawArrays(gl.TRIANGLES, 0, 3);

"WebGL1 triangle rendered";
