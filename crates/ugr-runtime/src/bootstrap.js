
(() => {
  let nextDomId = 1;
  const id = () => Deno.core.ops.op_alloc_resource();
  globalThis.__ugr_webgl_commands = [];
  globalThis.__ugr_webgpu_commands = [];
  globalThis.__ugr_canvas_commands = [];
  const record = (op, args = []) => __ugr_webgl_commands.push({ op, args });
  const resource = (kind) => ({ id: Deno.core.ops.op_alloc_resource(), kind });
  const extensions = ["OES_element_index_uint", "OES_vertex_array_object", "WEBGL_depth_texture", "EXT_texture_filter_anisotropic", "EXT_color_buffer_float"];
  const createCanvas2DContext = () => {
    const state = { fillStyle: "#000000", strokeStyle: "#000000", lineWidth: 1, globalAlpha: 1, stack: [] };
    const record2d = (op, args = []) => __ugr_canvas_commands.push({ op, args });
    const color = (value) => { const m = String(value).match(/^#([0-9a-f]{6})$/i); return m ? [parseInt(m[1].slice(0, 2), 16) / 255, parseInt(m[1].slice(2, 4), 16) / 255, parseInt(m[1].slice(4, 6), 16) / 255, state.globalAlpha] : [0, 0, 0, state.globalAlpha]; };
    return { get fillStyle() { return state.fillStyle; }, set fillStyle(v) { state.fillStyle = String(v); }, get strokeStyle() { return state.strokeStyle; }, set strokeStyle(v) { state.strokeStyle = String(v); }, get lineWidth() { return state.lineWidth; }, set lineWidth(v) { state.lineWidth = Number(v) || 1; }, get globalAlpha() { return state.globalAlpha; }, set globalAlpha(v) { state.globalAlpha = Math.max(0, Math.min(1, Number(v))); }, fillRect(x, y, w, h) { record2d("fillRect", [x, y, w, h, color(state.fillStyle)]); }, clearRect(x, y, w, h) { record2d("clearRect", [x, y, w, h]); }, beginPath() { record2d("beginPath"); }, moveTo(x, y) { record2d("moveTo", [x, y]); }, lineTo(x, y) { record2d("lineTo", [x, y]); }, stroke() { record2d("stroke", [color(state.strokeStyle), state.lineWidth]); }, fill() { record2d("fill", [color(state.fillStyle)]); }, save() { state.stack.push([state.fillStyle, state.strokeStyle, state.lineWidth, state.globalAlpha]); record2d("save"); }, restore() { const v = state.stack.pop(); if (v) [state.fillStyle, state.strokeStyle, state.lineWidth, state.globalAlpha] = v; record2d("restore"); }, setTransform(a, b, c, d, e, f) { record2d("setTransform", [a, b, c, d, e, f]); }, resetTransform() { record2d("setTransform", [1, 0, 0, 1, 0, 0]); } };
  };

  const createWebGlContext = (version, attributes = {}) => {
    const is2 = version === "webgl2";
    const state = { buffers: {}, textures: {}, vao: null, framebuffer: null, program: null, arrayBuffer: null, elementBuffer: null };
    const gl = {
      version, drawingBufferWidth: 1, drawingBufferHeight: 1,
      getContextAttributes() { return { alpha: attributes.alpha !== false, antialias: attributes.antialias !== false, depth: attributes.depth !== false, stencil: !!attributes.stencil, preserveDrawingBuffer: !!attributes.preserveDrawingBuffer }; },
      getSupportedExtensions() { return extensions.slice(); }, getExtension(name) { return extensions.includes(name) ? { name } : null; },
      getError() { return 0; }, isContextLost() { return false; },
      createBuffer() { const v = resource("buffer"); state.buffers[v.id] = { data: [] }; return v; },
    deleteBuffer(v) { if (v) { delete state.buffers[v.id]; Deno.core.ops.op_release_buffer(v.id); } record("deleteBuffer", [v?.id || 0]); },
      bindBuffer(target, v) { if (target === 0x8892) state.arrayBuffer = v; if (target === 0x8893) state.elementBuffer = v; record("bindBuffer", [target, v?.id || 0]); },
    bufferData(target, data, usage) { const v = target === 0x8893 ? state.elementBuffer : state.arrayBuffer; if (v && ArrayBuffer.isView(data)) { state.buffers[v.id].byteLength = data.byteLength; const bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength); Deno.core.ops.op_upload_buffer(v.id, target, usage || 0, bytes); record("bufferData", [target, data.byteLength, usage || 0]); } else { record("bufferData", [target, Number(data) || 0, usage || 0]); } },
    bufferSubData(target, offset, data) { const v = target === 0x8893 ? state.elementBuffer : state.arrayBuffer; if (v && ArrayBuffer.isView(data)) { const bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength); Deno.core.ops.op_upload_buffer(v.id, target, 0, bytes); } record("bufferSubData", [target, offset, data?.byteLength || 0]); },
      createTexture() { return resource("texture"); }, deleteTexture(v) { if (v) Deno.core.ops.op_release_resource(v.id); record("deleteTexture", [v?.id || 0]); }, bindTexture(target, v) { state.texture = v; record("bindTexture", [target, v?.id || 0]); },
      texImage2D(...args) { const data = args[args.length - 1]; if (state.texture && ArrayBuffer.isView(data)) { const bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength); Deno.core.ops.op_upload_texture(state.texture.id, bytes); } record("texImage2D", args.slice(0, -1).concat(ArrayBuffer.isView(data) ? data.byteLength : data)); }, texParameteri(target, pname, param) { record("texParameteri", [target, pname, param]); },
      texStorage2D(target, levels, internalFormat, width, height) { if (is2) record("texStorage2D", [target, levels, internalFormat, width, height]); }, generateMipmap(target) { record("generateMipmap", [target]); }, activeTexture(texture) { record("activeTexture", [texture]); }, readPixels(...args) { record("readPixels", args.map((v) => ArrayBuffer.isView(v) ? Array.from(v) : v)); },
      createShader(type) { const v = resource("shader"); v.type = type; v.source = ""; return v; }, shaderSource(v, source) { if (v) { v.source = String(source); Deno.core.ops.op_set_shader_source(v.id, v.source); } record("shaderSource", [v?.id || 0, String(source)]); }, compileShader(v) { if (v) v.compiled = true; record("compileShader", [v?.id || 0]); },
      getShaderParameter(v, p) { return p === 0x8B81 ? !!v?.compiled : true; }, getShaderInfoLog() { return ""; }, deleteShader(v) { record("deleteShader", [v?.id || 0]); },
      createProgram() { return resource("program"); }, attachShader(p, s) { record("attachShader", [p?.id || 0, s?.id || 0]); }, linkProgram(p) { if (p) p.linked = true; record("linkProgram", [p?.id || 0]); }, getProgramParameter(p, q) { return q === 0x8B82 ? !!p?.linked : true; }, getProgramInfoLog() { return ""; }, useProgram(p) { state.program = p; record("useProgram", [p?.id || 0]); }, deleteProgram(p) { record("deleteProgram", [p?.id || 0]); },
      createFramebuffer() { return resource("framebuffer"); }, bindFramebuffer(target, v) { state.framebuffer = v; record("bindFramebuffer", [target, v?.id || 0]); }, deleteFramebuffer(v) { record("deleteFramebuffer", [v?.id || 0]); },
      createRenderbuffer() { return resource("renderbuffer"); }, bindRenderbuffer(target, v) { record("bindRenderbuffer", [target, v?.id || 0]); }, renderbufferStorage(...args) { record("renderbufferStorage", args); },
      createVertexArray() { return is2 ? resource("vertexArray") : null; }, bindVertexArray(v) { state.vao = v; record("bindVertexArray", [v?.id || 0]); }, deleteVertexArray(v) { record("deleteVertexArray", [v?.id || 0]); },
      enableVertexAttribArray(i) { record("enableVertexAttribArray", [i]); }, disableVertexAttribArray(i) { record("disableVertexAttribArray", [i]); }, vertexAttribPointer(...args) { record("vertexAttribPointer", args); }, vertexAttribDivisor(i, d) { if (is2) record("vertexAttribDivisor", [i, d]); },
      getAttribLocation(p, name) { return { id: id(), program: p?.id || 0, name: String(name) }; }, vertexAttrib1f(i, x) { record("vertexAttrib1f", [i?.id || i, x]); }, vertexAttrib2f(i, x, y) { record("vertexAttrib2f", [i?.id || i, x, y]); }, vertexAttrib3f(i, x, y, z) { record("vertexAttrib3f", [i?.id || i, x, y, z]); }, vertexAttrib4f(i, x, y, z, w) { record("vertexAttrib4f", [i?.id || i, x, y, z, w]); },
      viewport(x, y, w, h) { record("viewport", [x, y, w, h]); }, scissor(x, y, w, h) { record("scissor", [x, y, w, h]); }, enable(cap) { record("enable", [cap]); }, disable(cap) { record("disable", [cap]); }, blendFunc(s, d) { record("blendFunc", [s, d]); }, depthFunc(f) { record("depthFunc", [f]); },
      clearColor(r, g, b, a) { record("clearColor", [r, g, b, a]); }, clearDepth(v) { record("clearDepth", [v]); }, clearStencil(v) { record("clearStencil", [v]); }, clear(mask) { record("clear", [mask === undefined ? 0x4000 : mask]); },
      drawArrays(mode, first, count) { record("drawArrays", [mode, first, count]); }, drawElements(mode, count, type, offset) { record("drawElements", [mode, count, type, offset]); }, drawArraysInstanced(mode, first, count, instances) { if (is2) record("drawArraysInstanced", [mode, first, count, instances]); },
      uniform1f(loc, x) { record("uniform1f", [loc?.id || 0, x]); }, uniform4f(loc, a, b, c, d) { record("uniform4f", [loc?.id || 0, a, b, c, d]); }, uniformMatrix4fv(loc, transpose, value) { record("uniformMatrix4fv", [loc?.id || 0, !!transpose, Array.from(value || [])]); }, getUniformLocation(p, name) { return { id: id(), program: p?.id || 0, name: String(name) }; },
      TRIANGLES: 0x0004, ARRAY_BUFFER: 0x8892, ELEMENT_ARRAY_BUFFER: 0x8893, STATIC_DRAW: 0x88E4, FLOAT: 0x1406, UNSIGNED_SHORT: 0x1403, UNSIGNED_INT: 0x1405, COLOR_BUFFER_BIT: 0x4000, DEPTH_BUFFER_BIT: 0x0100, STENCIL_BUFFER_BIT: 0x0400, DEPTH_TEST: 0x0B71, BLEND: 0x0BE2,
    };
    return gl;
  };

  class GPUBuffer { constructor(d = {}) { this.id = id(); this.size = d.size || 0; this.usage = d.usage || 0; this.destroyed = false; } destroy() { this.destroyed = true; __ugr_webgpu_commands.push({ op: "destroyBuffer", id: this.id }); } }
  class GPUTexture { constructor(d = {}) { this.id = id(); this.width = d.size?.width || d.size || 1; this.height = d.size?.height || 1; this.format = d.format || "rgba8unorm"; } createView(d = {}) { return { id: id(), texture: this.id, descriptor: d }; } destroy() { Deno.core.ops.op_release_resource(this.id); __ugr_webgpu_commands.push({ op: "destroyTexture", id: this.id }); } }
  class GPUShaderModule { constructor(d = {}) { this.id = id(); this.code = String(d.code || ""); Deno.core.ops.op_set_shader_source(this.id, this.code); } }
  class GPURenderPipeline { constructor(d = {}) { this.id = id(); this.descriptor = d; Deno.core.ops.op_register_pipeline(this.id, JSON.stringify(d)); __ugr_webgpu_commands.push({ op: "createRenderPipeline", pipeline: this.id, descriptor: d }); } }
  class GPUCommandBuffer { constructor(commands) { this.id = id(); this.commands = commands; } }
  class GPURenderPassEncoder { constructor(commands, descriptor) { this.commands = commands; this.descriptor = descriptor; } setPipeline(p) { this.commands.push({ op: "setPipeline", pipeline: p?.id || 0 }); } setVertexBuffer(slot, b, offset = 0, size) { this.commands.push({ op: "setVertexBuffer", slot, buffer: b?.id || 0, offset, size }); } draw(vertexCount, instanceCount = 1, firstVertex = 0, firstInstance = 0) { this.commands.push({ op: "draw", vertexCount, instanceCount, firstVertex, firstInstance }); } drawIndexed(indexCount, instanceCount = 1, firstIndex = 0, baseVertex = 0, firstInstance = 0) { this.commands.push({ op: "drawIndexed", indexCount, instanceCount, firstIndex, baseVertex, firstInstance }); } end() { this.commands.push({ op: "endRenderPass" }); } }
  class GPUCommandEncoder { constructor() { this.commands = []; } beginRenderPass(d = {}) { this.commands.push({ op: "beginRenderPass", descriptor: d }); return new GPURenderPassEncoder(this.commands, d); } beginComputePass(d = {}) { this.commands.push({ op: "beginComputePass", descriptor: d }); return { setPipeline: (p) => this.commands.push({ op: "setComputePipeline", pipeline: p?.id || 0 }), dispatchWorkgroups: (x, y = 1, z = 1) => this.commands.push({ op: "dispatchWorkgroups", x, y, z }), end: () => this.commands.push({ op: "endComputePass" }) }; } copyBufferToBuffer(src, so, dst, doff, size) { this.commands.push({ op: "copyBufferToBuffer", src: src?.id || 0, so, dst: dst?.id || 0, doff, size }); } finish() { return new GPUCommandBuffer(this.commands.slice()); } }
  class GPUCanvasContext { constructor(canvas) { this.canvas = canvas; this.configuration = null; } configure(d) { this.configuration = d; } unconfigure() { this.configuration = null; } getCurrentTexture() { return new GPUTexture({ size: { width: this.canvas.width || 1, height: this.canvas.height || 1 }, format: this.configuration?.format || "bgra8unorm-srgb" }); } }
  const queue = { writeBuffer(buffer, offset, data) { const bytes = ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : data; if (buffer && ArrayBuffer.isView(bytes)) Deno.core.ops.op_upload_buffer(buffer.id, 0, buffer.usage || 0, bytes); __ugr_webgpu_commands.push({ op: "writeBuffer", buffer: buffer?.id || 0, offset, byteLength: bytes?.byteLength || 0 }); }, writeTexture(destination, data, layout, size) { const bytes = ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : data; __ugr_webgpu_commands.push({ op: "writeTexture", destination, byteLength: bytes?.byteLength || 0, layout, size }); }, submit(buffers) { for (const b of buffers || []) __ugr_webgpu_commands.push(...(b?.commands || [])); }, onSubmittedWorkDone() { return Promise.resolve(); } };
  const device = { queue, lost: Promise.resolve({ reason: "destroyed", message: "" }), createBuffer(d) { return new GPUBuffer(d); }, createTexture(d) { return new GPUTexture(d); }, createSampler(d) { return { id: id(), descriptor: d }; }, createShaderModule(d) { return new GPUShaderModule(d); }, createRenderPipeline(d) { if (!d?.vertex) throw new TypeError("render pipeline requires a vertex stage"); return new GPURenderPipeline(d); }, createComputePipeline(d) { if (!d?.compute) throw new TypeError("compute pipeline requires a compute stage"); const p = { id: id(), descriptor: d }; Deno.core.ops.op_register_pipeline(p.id, JSON.stringify(d)); return p; }, createCommandEncoder() { return new GPUCommandEncoder(); }, createBindGroupLayout(d) { return { id: id(), descriptor: d }; }, createBindGroup(d) { return { id: id(), descriptor: d }; }, createPipelineLayout(d) { return { id: id(), descriptor: d }; } };
  const adapter = { name: "UGR D3D12 Adapter", features: new Set(), limits: { maxTextureDimension2D: 16384 }, isFallbackAdapter: false, requestDevice() { return device; } };
  const gpu = { requestAdapter() { return adapter; }, getPreferredCanvasFormat() { return "bgra8unorm-srgb"; } };
  globalThis.document = { createElement(tag) { return tag === "canvas" ? (() => { const canvas = { width: 1, height: 1, getContext(kind, attrs) { if (kind === "webgl" || kind === "experimental-webgl") return createWebGlContext("webgl", attrs); if (kind === "webgl2") return createWebGlContext("webgl2", attrs); if (kind === "webgpu") return new GPUCanvasContext(canvas); if (kind === "2d") return createCanvas2DContext(); return null; } }; return canvas; })() : {}; } };
  const legacyCreateElement = globalThis.document.createElement.bind(globalThis.document);
  const domMatch = (node, selector) => {
    selector = String(selector).trim();
    if (!selector || !node || node.nodeType !== 1) return false;
    if (selector.startsWith('#')) return node.id === selector.slice(1);
    if (selector.startsWith('.')) return node.className.split(/\s+/).includes(selector.slice(1));
    const parts = selector.split('.');
    return node.tagName.toLowerCase() === parts[0].toLowerCase() && (!parts[1] || node.className.split(/\s+/).includes(parts[1]));
  };
  const makeDomElement = (tag, attributes = {}) => {
    attributes = { ...attributes, 'data-ugr-id': attributes['data-ugr-id'] || `ugr-${nextDomId++}` };
    const canvas = tag === 'canvas' ? legacyCreateElement('canvas') : null;
    const node = { nodeType: 1, tagName: String(tag).toUpperCase(), nodeName: String(tag).toUpperCase(), parentNode: null, children: [], attributes: { ...attributes }, style: {}, _listeners: {}, textContent: '', value: attributes.value || '', type: attributes.type || (tag === 'input' ? 'text' : ''), checked: 'checked' in attributes, disabled: 'disabled' in attributes, selected: 'selected' in attributes, hidden: 'hidden' in attributes, tabIndex: Number(attributes.tabindex ?? -1) };
    node.id = attributes.id || '';
    node.className = attributes.class || attributes.className || '';
    node.appendChild = (child) => { if (child.parentNode) child.parentNode.removeChild(child); child.parentNode = node; node.children.push(child); return child; };
    node.removeChild = (child) => { const index = node.children.indexOf(child); if (index >= 0) { node.children.splice(index, 1); child.parentNode = null; } return child; };
    node.insertBefore = (child, reference) => { if (!reference || !node.children.includes(reference)) return node.appendChild(child); if (child.parentNode) child.parentNode.removeChild(child); child.parentNode = node; node.children.splice(node.children.indexOf(reference), 0, child); return child; };
    node.setAttribute = (name, value) => { name = String(name).toLowerCase(); value = String(value); node.attributes[name] = value; if (name === 'id') node.id = value; if (name === 'class') node.className = value; if (name === 'value') node.value = value; if (name === 'type') node.type = value; };
    node.getAttribute = (name) => node.attributes[String(name).toLowerCase()] ?? null;
    node.hasAttribute = (name) => Object.prototype.hasOwnProperty.call(node.attributes, String(name).toLowerCase());
    node.removeAttribute = (name) => { name = String(name).toLowerCase(); delete node.attributes[name]; if (name === 'id') node.id = ''; if (name === 'class') node.className = ''; };
    node.addEventListener = (type, listener, options = {}) => { if (!listener) return; const entry = { listener, once: !!(options && options.once) }; (node._listeners[type] ||= []).push(entry); };
    node.removeEventListener = (type, listener) => { node._listeners[type] = (node._listeners[type] || []).filter((entry) => (entry.listener || entry) !== listener); };
    node.dispatchEvent = (event) => { const value = typeof event === 'string' ? { type: event, target: node } : { ...event, target: node }; for (const entry of node._listeners[value.type] || []) { const listener = entry.listener || entry; if (typeof listener === 'function') listener.call(node, value); else if (listener && typeof listener.handleEvent === 'function') listener.handleEvent(value); if (entry.once) node.removeEventListener(value.type, listener); } return !value.defaultPrevented; };
    node.focus = () => { node.ownerDocument && (node.ownerDocument.activeElement = node); node.dispatchEvent({ type: 'focus' }); };
    node.blur = () => { node.dispatchEvent({ type: 'blur' }); };
    node.querySelectorAll = (selector) => { const result = []; const visit = (child) => { for (const value of child.children) { if (domMatch(value, selector)) result.push(value); visit(value); } }; visit(node); return result; };
    node.querySelector = (selector) => node.querySelectorAll(selector)[0] || null;
    node.getBoundingClientRect = () => { const number = (value, fallback) => { const parsed = Number.parseFloat(String(value || '').replace('px', '')); return Number.isFinite(parsed) ? parsed : fallback; }; const width = number(node.style.width || node.getAttribute('width'), 0); const height = number(node.style.height || node.getAttribute('height'), 0); return { x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height, toJSON() { return this; } }; };
    if (canvas) { node.width = Number(attributes.width) || canvas.width; node.height = Number(attributes.height) || canvas.height; node.getContext = (kind, options) => canvas.getContext(kind, options); }
    if (tag === 'input' || tag === 'button' || tag === 'a') { node.click = () => { if (!node.disabled) node.dispatchEvent({ type: 'click', bubbles: true, cancelable: true }); }; }
    if (tag === 'form') { node.submit = () => node.dispatchEvent({ type: 'submit', bubbles: true, cancelable: true }); node.reset = () => { for (const field of node.querySelectorAll('input')) field.value = field.getAttribute('value') || ''; node.dispatchEvent({ type: 'reset', bubbles: true }); }; }
    if (tag === 'select') { Object.defineProperty(node, 'options', { get: () => node.children.filter((child) => child.tagName === 'OPTION') }); Object.defineProperty(node, 'selectedIndex', { get: () => node.options.findIndex((option) => option.selected), set: (index) => node.options.forEach((option, current) => { option.selected = current === Number(index); }) }); }
    return node;
  };
  const parseDomMarkup = (markup) => {
    const root = makeDomElement('document'); root.nodeType = 9; root.tagName = '#document'; root.ownerDocument = root;
    const stack = [root]; const tokenPattern = /<!--[\s\S]*?-->|<[^>]+>|[^<]+/g; let match;
    while ((match = tokenPattern.exec(String(markup)))) { const token = match[0]; if (token.startsWith('<!--')) continue; if (!token.startsWith('<')) { const text = token.trim(); if (text) stack[stack.length - 1].textContent += text; continue; }
      if (token.startsWith('</')) { if (stack.length > 1) stack.pop(); continue; }
      if (token.startsWith('<!')) continue;
      const open = token.slice(1, -1).replace(/\/$/, '').trim(); const nameMatch = open.match(/^[^\s]+/); if (!nameMatch) continue; const tag = nameMatch[0].toLowerCase(); const attrs = {}; const attrPattern = /([^\s=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s]+)))?/g; let attr; const rest = open.slice(nameMatch[0].length); while ((attr = attrPattern.exec(rest))) attrs[attr[1].toLowerCase()] = attr[2] ?? attr[3] ?? attr[4] ?? '';
      const node = makeDomElement(tag, attrs); node.ownerDocument = root; stack[stack.length - 1].appendChild(node); if (!/^(input|img|br|hr|meta|link|canvas)$/.test(tag) && !token.endsWith('/>')) stack.push(node);
    }
    const walk = (node) => { for (const child of node.children) { child.ownerDocument = root; walk(child); } }; walk(root); return root;
  };
  const installDocument = (markup) => { const parsed = parseDomMarkup(markup); const document = parsed; document.createElement = (tag) => { const element = makeDomElement(String(tag).toLowerCase()); element.ownerDocument = document; return element; }; document.getElementById = (id) => document.querySelector(`#${id}`); document.body = parsed.querySelector('body') || parsed; document.head = parsed.querySelector('head') || parsed; document.documentElement = parsed.querySelector('html') || parsed; document.activeElement = null; document.createTextNode = (text) => ({ nodeType: 3, textContent: String(text), parentNode: null }); document.defaultView = globalThis; globalThis.document = document; };
  globalThis.__ugr_install_document = installDocument;
  const enhanceDomElement = (node, document) => {
    if (!node || node.nodeType !== 1 || node.__ugrEnhanced) return;
    node.__ugrEnhanced = true;
    node.ownerDocument = document;
    const rawText = node.textContent || '';
    Object.defineProperty(node, 'textContent', { configurable: true, get() { return node.children.reduce((value, child) => value + child.textContent, node._textContent || ''); }, set(value) { node._textContent = String(value); node.children = []; } });
    node._textContent = rawText;
    let elementId = node.id || '';
    let elementClassName = node.className || '';
    Object.defineProperty(node, 'id', { configurable: true, get: () => elementId, set: (value) => { elementId = String(value); node.attributes.id = elementId; } });
    Object.defineProperty(node, 'className', { configurable: true, get: () => elementClassName, set: (value) => { elementClassName = String(value); node.attributes.class = elementClassName; } });
    Object.defineProperty(node, 'parentElement', { get() { return node.parentNode && node.parentNode.nodeType === 1 ? node.parentNode : null; } });
    Object.defineProperty(node, 'firstChild', { get() { return node.children[0] || null; } });
    Object.defineProperty(node, 'lastChild', { get() { return node.children[node.children.length - 1] || null; } });
    Object.defineProperty(node, 'childNodes', { get() { return node.children; } });
    Object.defineProperty(node, 'nextSibling', { get() { const siblings = node.parentNode?.children || []; const index = siblings.indexOf(node); return index >= 0 ? siblings[index + 1] || null : null; } });
    node.remove = () => { if (node.parentNode) node.parentNode.removeChild(node); };
    node.replaceChildren = (...children) => { for (const child of [...node.children]) node.removeChild(child); children.forEach((child) => node.appendChild(child)); };
    node.contains = (candidate) => { if (candidate === node) return true; return node.children.some((child) => child === candidate || child.contains?.(candidate)); };
    Object.defineProperty(node, 'innerHTML', { configurable: true, get() { return node.children.map(serializeDom).join('') || node._textContent || ''; }, set(value) { const parsed = parseDomMarkup(String(value)); node.children = []; node._textContent = ''; for (const child of parsed.children) { node.appendChild(child); enhanceDomElement(child, node.ownerDocument); } } });
    Object.defineProperty(node, 'outerHTML', { get() { return serializeDom(node); } });
    node.matches = (selector) => advancedMatch(node, selector);
    node.closest = (selector) => { let current = node; while (current) { if (advancedMatch(current, selector)) return current; current = current.parentNode; } return null; };
    node.querySelectorAll = (selector) => queryDescendants(node, selector);
    node.querySelector = (selector) => node.querySelectorAll(selector)[0] || null;
    node.classList = { contains: (name) => node.className.split(/\s+/).filter(Boolean).includes(String(name)), add: (...names) => { const values = new Set(node.className.split(/\s+/).filter(Boolean)); names.forEach((name) => values.add(String(name))); node.className = [...values].join(' '); node.setAttribute('class', node.className); }, remove: (...names) => { const remove = new Set(names.map(String)); node.className = node.className.split(/\s+/).filter((name) => name && !remove.has(name)).join(' '); node.setAttribute('class', node.className); }, toggle: (name, force) => { const present = node.classList.contains(name); const next = force === undefined ? !present : !!force; if (next !== present) (next ? node.classList.add : node.classList.remove)(name); return next; }, toString: () => node.className };
    node.dataset = new Proxy({}, { get: (_, name) => { const key = `data-${String(name).replace(/[A-Z]/g, (value) => `-${value.toLowerCase()}`)}`; return node.getAttribute(key) ?? node.attributes[key] ?? undefined; }, set: (_, name, value) => { node.setAttribute(`data-${String(name).replace(/[A-Z]/g, (v) => `-${v.toLowerCase()}`)}`, value); return true; } });
    node.style = new Proxy(node.style || {}, { get(target, name) { const key = String(name).replace(/[A-Z]/g, (value) => `-${value.toLowerCase()}`); if (name === 'cssText') return Object.entries(target).map(([property, value]) => `${property}:${value}`).join(';'); if (name === 'setProperty') return (property, value) => { target[String(property).toLowerCase()] = String(value); }; if (name === 'getPropertyValue') return (property) => target[String(property).toLowerCase()] || ''; if (name === 'removeProperty') return (property) => { const old = target[String(property).toLowerCase()] || ''; delete target[String(property).toLowerCase()]; return old; }; return target[key] || target[name] || ''; }, set(target, name, value) { if (name === 'cssText') { for (const declaration of String(value).split(';')) { const [key, val] = declaration.split(':'); if (key && val) target[key.trim().toLowerCase()] = val.trim(); } } else { const key = String(name).replace(/[A-Z]/g, (value) => `-${value.toLowerCase()}`); target[key] = String(value); } return true; } });
    const dispatch = node.dispatchEvent;
    node.dispatchEvent = (event) => { const value = typeof event === 'string' ? { type: event } : event; value.target ||= node; value.currentTarget = node; dispatch(value); if (!value.cancelBubble && node.parentNode && node.parentNode.dispatchEvent) node.parentNode.dispatchEvent(value); return true; };
    node.click = node.click || (() => node.dispatchEvent({ type: 'click' }));
    for (const child of node.children) enhanceDomElement(child, document);
  };
  const serializeDom = (node) => { if (node.nodeType === 3) return node.textContent; const attributes = { ...(node.attributes || {}) }; if (node.id) attributes.id = node.id; if (node.className) attributes.class = node.className; if (node.tagName?.toLowerCase() === 'input') attributes.value = node.value ?? ''; if (node.style && Object.keys(node.style).length) attributes.style = node.style.cssText; const attrs = Object.entries(attributes).map(([name, value]) => ` ${name}="${String(value).replace(/"/g, '&quot;')}"`).join(''); return `<${node.tagName.toLowerCase()}${attrs}>${node.children.map(serializeDom).join('') || node._textContent || ''}</${node.tagName.toLowerCase()}>`; };
  const matchesSimple = (node, selector) => { selector = selector.trim(); if (!selector || !node || node.nodeType !== 1) return false; const attribute = selector.match(/\[([^=\]]+)(?:=["']?([^\]"']+)["']?)?\]/); if (attribute && (!node.hasAttribute(attribute[1]) || (attribute[2] && node.getAttribute(attribute[1]) !== attribute[2]))) return false; selector = selector.replace(attribute ? attribute[0] : '', ''); const id = selector.match(/#([\w-]+)/); if (id && node.id !== id[1]) return false; const classes = [...selector.matchAll(/\.([\w-]+)/g)].map((match) => match[1]); if (classes.some((name) => !node.className.split(/\s+/).includes(name))) return false; const tag = selector.replace(/[#.].*$/, '').trim(); return !tag || tag === '*' || node.tagName.toLowerCase() === tag.toLowerCase(); };
  const advancedMatch = (node, selector) => String(selector).split(',').some((part) => { const tokens = part.trim().split(/\s+|>/).filter(Boolean); let current = node; for (let index = tokens.length - 1; index >= 0; index -= 1) { if (!matchesSimple(current, tokens[index])) return false; if (index > 0) { current = current.parentNode; while (current && !matchesSimple(current, tokens[index - 1])) current = current.parentNode; if (!current) return false; } } return true; });
  const queryDescendants = (node, selector) => { const result = []; const visit = (parent) => { for (const child of parent.children || []) { if (advancedMatch(child, selector)) result.push(child); visit(child); } }; visit(node); return result; };
  const enhanceDocument = (markup) => { const document = globalThis.document; for (const child of document.children) enhanceDomElement(child, document); const createElement = document.createElement; document.createElement = (tag) => { const element = createElement(tag); enhanceDomElement(element, document); return element; }; document.querySelectorAll = (selector) => queryDescendants(document, selector); document.querySelector = (selector) => document.querySelectorAll(selector)[0] || null; document.getElementsByTagName = (tag) => document.querySelectorAll(tag); document.getElementsByClassName = (name) => document.querySelectorAll(`.${name}`); document.createElementNS = (_, tag) => document.createElement(tag); document.createEvent = (type) => new Event(type); document.readyState = 'complete'; document.dispatchEvent(new Event('DOMContentLoaded')); return markup; };
  const installEnhancedDocumentBase = (markup) => { installDocument(markup); enhanceDocument(markup); };
  const upgradeDocumentEvents = (document) => {
    const upgrade = (node) => {
      if (!node || node.nodeType !== 1) return;
      node.dispatchEvent = (event) => {
        const value = typeof event === 'string' ? new Event(event) : event;
        if (!value.target) value.target = node;
        value.defaultPrevented ||= false;
        value.cancelBubble ||= false;
        value.preventDefault ||= (() => { value.defaultPrevented = true; });
        value.stopPropagation ||= (() => { value.cancelBubble = true; });
        value.stopImmediatePropagation ||= (() => { value.cancelBubble = true; value.__ugrImmediate = true; });
        value.currentTarget = node;
        for (const entry of [...(node._listeners[value.type] || [])]) {
          const listener = entry.listener || entry;
          if (typeof listener === 'function') listener.call(node, value);
          else if (listener && typeof listener.handleEvent === 'function') listener.handleEvent(value);
          if (entry.once) node.removeEventListener(value.type, listener);
          if (value.__ugrImmediate) break;
        }
        if (!value.cancelBubble && node.parentNode && node.parentNode.dispatchEvent) node.parentNode.dispatchEvent(value);
        return !value.defaultPrevented;
      };
      for (const child of node.children || []) upgrade(child);
    };
    for (const child of document.children || []) upgrade(child);
  };
  const installEnhancedDocument = (markup) => { installEnhancedDocumentBase(markup); upgradeDocumentEvents(globalThis.document); return markup; };
  globalThis.__ugr_install_document = installEnhancedDocument;
  // Native compositor events identify elements by the same renderable-child
  // path used by the Rust layout tree.
  globalThis.__ugr_dispatch_ui_event = (target, init = {}) => {
    const ignored = new Set(['#text', 'head', 'title', 'meta', 'link', 'style', 'script', 'option']);
    let node = globalThis.document;
    const key = target && !Array.isArray(target) ? target.key : '';
    if (key) {
      const find = (parent) => {
        for (const child of parent.children || []) {
          if (child.attributes?.['data-ugr-id'] === key) return child;
          const found = find(child);
          if (found) return found;
        }
        return null;
      };
      node = find(globalThis.document);
    } else {
      for (const index of (Array.isArray(target) ? target : [])) {
        const children = (node.children || []).filter((child) => !ignored.has(String(child.tagName || '').toLowerCase()));
        node = children[index];
        if (!node) return JSON.stringify({ defaultPrevented: false, markup: '' });
      }
    }
    if (init.value !== undefined && node && ('value' in node)) {
      node.value = String(init.value);
      if (node.setAttribute && node.tagName?.toLowerCase() === 'textarea') node.setAttribute('value', node.value);
    }
    const before = globalThis.document?.documentElement?.outerHTML || globalThis.document?.outerHTML || '';
    const event = { ...init, target: node, bubbles: true, cancelable: true };
    const allowed = !!(node && node.dispatchEvent && node.dispatchEvent(event));
    const after = globalThis.document?.documentElement?.outerHTML || globalThis.document?.outerHTML || '';
    return JSON.stringify({
      defaultPrevented: !allowed || !!event.defaultPrevented,
      markup: before === after ? '' : after
    });
  };
  globalThis.__ugr_dispatch_click = (path = []) => __ugr_dispatch_ui_event(path, { type: 'click' });
  globalThis.getComputedStyle = (element) => { const computed = {}; const document = element && element.ownerDocument; for (const styleNode of document ? document.querySelectorAll('style') : []) { for (const rule of String(styleNode.textContent || '').split('}')) { const parts = rule.split('{'); if (parts.length !== 2 || !advancedMatch(element, parts[0].trim())) continue; for (const declaration of parts[1].split(';')) { const [name, value] = declaration.split(':'); if (name && value) computed[name.trim()] = value.trim(); } } } Object.assign(computed, element.style || {}); const styleText = element.getAttribute && element.getAttribute('style'); if (styleText) for (const declaration of styleText.split(';')) { const [name, value] = declaration.split(':'); if (name && value) computed[name.trim()] = value.trim(); } computed.getPropertyValue = (name) => computed[name] || ''; computed.setProperty = (name, value) => { computed[name] = String(value); }; return computed; };
  globalThis.Event = class Event { constructor(type, init = {}) { this.type = type; Object.assign(this, init); } };
  globalThis.navigator = { gpu };
})();
    
