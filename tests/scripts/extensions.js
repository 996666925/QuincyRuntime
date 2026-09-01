"use strict";

const canvas = document.createElement("canvas");
const gl = canvas.getContext("webgl2");
const extensions = gl.getSupportedExtensions();
if (!extensions.includes("OES_element_index_uint")) {
  throw new Error("required WebGL extension is unavailable");
}

"WebGL extension query ready";
