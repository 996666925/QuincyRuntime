use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebGlVersion {
    WebGl1,
    WebGl2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAttributes {
    pub alpha: bool,
    pub antialias: bool,
    pub depth: bool,
    pub stencil: bool,
    pub preserve_drawing_buffer: bool,
}

impl Default for ContextAttributes {
    fn default() -> Self {
        Self {
            alpha: true,
            antialias: true,
            depth: true,
            stencil: false,
            preserve_drawing_buffer: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Buffer,
    Texture,
    Shader,
    Program,
    Framebuffer,
    Renderbuffer,
    VertexArray,
}

#[derive(Debug)]
pub struct WebGlContext {
    pub version: WebGlVersion,
    pub attributes: ContextAttributes,
    next_handle: u32,
    resources: HashMap<u32, ResourceKind>,
    extensions: HashSet<&'static str>,
    pub draw_calls: u64,
}

impl WebGlContext {
    pub fn new(version: WebGlVersion, attributes: ContextAttributes) -> Self {
        let extensions = [
            "OES_element_index_uint",
            "WEBGL_depth_texture",
            "EXT_texture_filter_anisotropic",
        ]
        .into_iter()
        .collect();
        Self {
            version,
            attributes,
            next_handle: 1,
            resources: HashMap::new(),
            extensions,
            draw_calls: 0,
        }
    }

    pub fn get_supported_extensions(&self) -> Vec<&'static str> {
        let mut extensions: Vec<_> = self.extensions.iter().copied().collect();
        extensions.sort_unstable();
        extensions
    }

    pub fn get_extension(&self, name: &str) -> bool {
        self.extensions.contains(name)
    }

    pub fn create_resource(&mut self, kind: ResourceKind) -> u32 {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        self.resources.insert(handle, kind);
        handle
    }

    pub fn delete_resource(&mut self, handle: u32) -> bool {
        self.resources.remove(&handle).is_some()
    }

    pub fn resource_kind(&self, handle: u32) -> Option<ResourceKind> {
        self.resources.get(&handle).copied()
    }

    pub fn draw_arrays(&mut self, _mode: u32, _first: i32, _count: i32) {
        self.draw_calls = self.draw_calls.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webgl2_tracks_resources_and_draws() {
        let mut context = WebGlContext::new(WebGlVersion::WebGl2, ContextAttributes::default());
        let buffer = context.create_resource(ResourceKind::Buffer);
        let vao = context.create_resource(ResourceKind::VertexArray);
        assert_eq!(context.resource_kind(buffer), Some(ResourceKind::Buffer));
        assert_eq!(context.resource_kind(vao), Some(ResourceKind::VertexArray));
        context.draw_arrays(4, 0, 3);
        assert_eq!(context.draw_calls, 1);
        assert!(context.get_extension("OES_element_index_uint"));
        assert!(context.delete_resource(buffer));
    }
}
