//! Native HTML layout and drawing backend.
//!
//! The DOM remains owned by `ugr-html`; this crate turns that document into a
//! Taffy layout tree and paints the resulting boxes with the workspace Skia
//! dependency. It deliberately exposes no JavaScript-specific state.

use std::collections::{BTreeMap, HashMap};

use skia_safe::{
    surfaces, AlphaType, Canvas, Color, ColorType, Font, Paint, PaintStyle, Rect as SkiaRect,
    Typeface,
};
use taffy::prelude::*;
use ugr_html::{CssRule, HtmlDocument, HtmlNode};

mod fonts;
mod style;

use fonts::default_typeface;
use style::{
    border as parse_border, box_values, color as parse_color, declarations as parse_declarations,
    dimension, dimension_auto, pixels as px,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A rendered DOM target carrying both a stable JS identity and a structural
/// path fallback for documents produced by older runtime snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiEventTarget {
    pub path: Vec<usize>,
    pub tag: String,
    pub key: String,
}

/// A snapshot of a rendered DOM node.  The snapshot is intentionally cheap to
/// clone so tools and native widgets can inspect the UI without borrowing the
/// renderer across a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct UiNodeInfo {
    pub node: NodeId,
    pub target: UiEventTarget,
    pub rect: UiRect,
    pub element: HtmlNode,
}

pub struct UiRenderer {
    /// CSS/layout viewport dimensions in logical pixels.
    pub width: u32,
    pub height: u32,
    pixel_width: u32,
    pixel_height: u32,
    scale_factor: f32,
    document: HtmlDocument,
    taffy: TaffyTree<()>,
    root: NodeId,
    elements: HashMap<NodeId, HtmlNode>,
    styles: HashMap<NodeId, BTreeMap<String, String>>,
    layout: HashMap<NodeId, UiRect>,
    surface: skia_safe::Surface,
    typeface: Typeface,
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    caret: usize,
    selection_anchor: Option<usize>,
    pixel_cache: Vec<u8>,
    pixel_cache_dirty: bool,
}

impl UiRenderer {
    pub fn from_html(source: &str, width: u32, height: u32) -> Result<Self, String> {
        Self::from_document(&ugr_html::parse_document(source), width, height)
    }

    pub fn from_document(document: &HtmlDocument, width: u32, height: u32) -> Result<Self, String> {
        Self::from_document_scaled(document, width, height, 1.0)
    }

    pub fn from_document_scaled(
        document: &HtmlDocument,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<Self, String> {
        let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let pixel_width = ((width.max(1) as f32 * scale_factor).round() as u32).max(1);
        let pixel_height = ((height.max(1) as f32 * scale_factor).round() as u32).max(1);
        Self::from_document_with_physical_size(
            document,
            width,
            height,
            pixel_width,
            pixel_height,
            scale_factor,
        )
    }

    pub fn from_document_with_physical_size(
        document: &HtmlDocument,
        width: u32,
        height: u32,
        pixel_width: u32,
        pixel_height: u32,
        scale_factor: f32,
    ) -> Result<Self, String> {
        let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let mut taffy = TaffyTree::new();
        let mut elements = HashMap::new();
        let mut styles = HashMap::new();
        let root = build_tree(
            &mut taffy,
            &mut elements,
            &mut styles,
            &document.styles,
            &document.root,
        )?;
        taffy
            .compute_layout(
                root,
                Size {
                    width: AvailableSpace::Definite(width as f32),
                    height: AvailableSpace::Definite(height as f32),
                },
            )
            .map_err(|error| format!("UI layout failed: {error:?}"))?;
        let mut layout = HashMap::new();
        collect_layout(&taffy, root, &mut layout, 0.0, 0.0)?;
        let surface = surfaces::raster_n32_premul((pixel_width as i32, pixel_height as i32))
            .ok_or_else(|| "failed to create Skia raster surface".to_owned())?;
        let typeface = default_typeface()?;
        Ok(Self {
            width,
            height,
            pixel_width,
            pixel_height,
            scale_factor,
            document: document.clone(),
            taffy,
            root,
            elements,
            styles,
            layout,
            surface,
            typeface,
            focused: None,
            hovered: None,
            pressed: None,
            caret: 0,
            selection_anchor: None,
            pixel_cache: Vec::new(),
            pixel_cache_dirty: true,
        })
    }

    pub fn root(&self) -> NodeId {
        self.root
    }
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.pixel_width, self.pixel_height)
    }
    pub fn layout(&self, node: NodeId) -> Option<UiRect> {
        self.layout.get(&node).copied()
    }

    /// Access the parsed document backing this renderer.
    pub fn document(&self) -> &HtmlDocument {
        &self.document
    }

    /// Return all rendered nodes in paint order. This is useful for native
    /// accessibility/tooling layers and avoids exposing the internal Taffy
    /// tree directly.
    pub fn nodes(&self) -> Vec<UiNodeInfo> {
        let mut result = Vec::new();
        let mut path = Vec::new();
        self.collect_nodes(self.root, &mut path, &mut result);
        result
    }

    pub fn node_by_id(&self, id: &str) -> Option<UiNodeInfo> {
        self.nodes().into_iter().find(|node| {
            node.element
                .attributes
                .get("id")
                .is_some_and(|value| value == id)
        })
    }

    pub fn node(&self, node: NodeId) -> Option<&HtmlNode> {
        self.elements.get(&node)
    }

    /// Set an attribute and rebuild only the affected layout snapshot. The
    /// stable `data-ugr-id` target makes this safe across DOM reorderings.
    pub fn set_attribute(
        &mut self,
        target: &UiEventTarget,
        name: &str,
        value: &str,
    ) -> Result<bool, String> {
        let changed = self.mutate_document_target(target, |element| {
            let previous = element
                .attributes
                .insert(name.to_ascii_lowercase(), value.to_owned());
            previous.as_deref() != Some(value)
        })?;
        if changed {
            self.rebuild_preserving_state()?;
        }
        Ok(changed)
    }

    pub fn remove_attribute(&mut self, target: &UiEventTarget, name: &str) -> Result<bool, String> {
        let changed = self.mutate_document_target(target, |element| {
            element
                .attributes
                .remove(&name.to_ascii_lowercase())
                .is_some()
        })?;
        if changed {
            self.rebuild_preserving_state()?;
        }
        Ok(changed)
    }

    pub fn set_text(&mut self, target: &UiEventTarget, text: &str) -> Result<bool, String> {
        let changed = self.mutate_document_target(target, |element| {
            if element.text == text {
                false
            } else {
                element.text = text.to_owned();
                true
            }
        })?;
        if changed {
            self.rebuild_preserving_state()?;
        }
        Ok(changed)
    }

    pub fn set_hovered(&mut self, target: Option<&UiEventTarget>) -> bool {
        let next = target.and_then(|target| self.find_node(target));
        if self.hovered == next {
            return false;
        }
        self.hovered = next;
        self.pixel_cache_dirty = true;
        true
    }

    pub fn set_pressed(&mut self, target: Option<&UiEventTarget>) -> bool {
        let next = target.and_then(|target| self.find_node(target));
        if self.pressed == next {
            return false;
        }
        self.pressed = next;
        self.pixel_cache_dirty = true;
        true
    }

    /// Apply the browser's small set of native activation defaults that are
    /// meaningful to the raster backend. JavaScript can cancel this by
    /// calling `preventDefault()` on the click event.
    pub fn activate(&mut self, target: &UiEventTarget) -> Result<bool, String> {
        let Some(node) = self.find_node(target) else {
            return Ok(false);
        };
        let Some(element) = self.elements.get(&node) else {
            return Ok(false);
        };
        if element.tag != "input" {
            return Ok(false);
        }
        let input_type = element
            .attributes
            .get("type")
            .cloned()
            .unwrap_or_else(|| "text".into());
        if !matches!(input_type.as_str(), "checkbox" | "radio") {
            return Ok(false);
        }
        let changed = self.mutate_document_target(target, |element| {
            if input_type == "radio" {
                if element.attributes.contains_key("checked") {
                    return false;
                }
                element.attributes.insert("checked".into(), String::new());
            } else if element.attributes.contains_key("checked") {
                element.attributes.remove("checked");
            } else {
                element.attributes.insert("checked".into(), String::new());
            }
            true
        })?;
        if changed {
            self.rebuild_preserving_state()?;
            self.pixel_cache_dirty = true;
        }
        Ok(changed)
    }

    pub fn clear_focus(&mut self) -> bool {
        let had_focus = self.focused.take().is_some();
        self.selection_anchor = None;
        if had_focus {
            self.pixel_cache_dirty = true;
        }
        had_focus
    }

    fn collect_nodes(&self, node: NodeId, path: &mut Vec<usize>, output: &mut Vec<UiNodeInfo>) {
        if let (Some(element), Some(rect)) = (self.elements.get(&node), self.layout.get(&node)) {
            if element.tag != "document" && element.tag != "#text" {
                output.push(UiNodeInfo {
                    node,
                    target: UiEventTarget {
                        path: path.clone(),
                        tag: element.tag.clone(),
                        key: element
                            .attributes
                            .get("data-ugr-id")
                            .cloned()
                            .unwrap_or_default(),
                    },
                    rect: *rect,
                    element: element.clone(),
                });
            }
        }
        for (index, child) in self.taffy.child_ids(node).enumerate() {
            path.push(index);
            self.collect_nodes(child, path, output);
            path.pop();
        }
    }

    fn find_node(&self, target: &UiEventTarget) -> Option<NodeId> {
        self.elements
            .iter()
            .find_map(|(node, element)| {
                let key_matches = !target.key.is_empty()
                    && element.attributes.get("data-ugr-id") == Some(&target.key);
                let tag_matches = element.tag == target.tag;
                (tag_matches && key_matches).then_some(*node)
            })
            .or_else(|| self.node_at_path(&target.path))
    }

    fn node_at_path(&self, path: &[usize]) -> Option<NodeId> {
        let mut node = self.root;
        for index in path {
            node = self.taffy.child_ids(node).nth(*index)?;
        }
        Some(node)
    }

    fn mutate_document_target<F>(
        &mut self,
        target: &UiEventTarget,
        mutator: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(&mut HtmlNode) -> bool,
    {
        if !target.key.is_empty() {
            if let Some(node) = find_document_node_by_key_mut(&mut self.document.root, &target.key)
            {
                return Ok(mutator(node));
            }
        }
        let Some(node) =
            find_document_node_by_rendered_path_mut(&mut self.document.root, &target.path)
        else {
            return Ok(false);
        };
        Ok(mutator(node))
    }

    fn rebuild_preserving_state(&mut self) -> Result<(), String> {
        let rebuilt = Self::from_document_with_physical_size(
            &self.document,
            self.width,
            self.height,
            self.pixel_width,
            self.pixel_height,
            self.scale_factor,
        )?;
        self.replace_preserving_edit_state(rebuilt);
        Ok(())
    }

    /// Return the deepest rendered element under a logical viewport point.
    /// Children are visited in reverse paint order so overlapping elements
    /// receive the event before their parents.
    pub fn event_target_at(&self, x: f32, y: f32) -> Option<UiEventTarget> {
        let mut path = Vec::new();
        self.event_target_at_node(self.root, x, y, &mut path)
    }

    pub fn focused_event_target(&self) -> Option<UiEventTarget> {
        let focused = self.focused?;
        let mut path = Vec::new();
        if self.event_path(self.root, focused, &mut path) {
            return self.elements.get(&focused).map(|element| UiEventTarget {
                path,
                tag: element.tag.clone(),
                key: element
                    .attributes
                    .get("data-ugr-id")
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        None
    }

    fn event_path(&self, node: NodeId, target: NodeId, path: &mut Vec<usize>) -> bool {
        if node == target {
            return true;
        }
        let children = self.taffy.child_ids(node).collect::<Vec<_>>();
        for (index, child) in children.iter().enumerate() {
            path.push(index);
            if self.event_path(*child, target, path) {
                return true;
            }
            path.pop();
        }
        false
    }

    fn event_target_at_node(
        &self,
        node: NodeId,
        x: f32,
        y: f32,
        path: &mut Vec<usize>,
    ) -> Option<UiEventTarget> {
        let rect = self.layout.get(&node)?;
        if x < rect.x || y < rect.y || x > rect.x + rect.width || y > rect.y + rect.height {
            return None;
        }
        let children = self.taffy.child_ids(node).collect::<Vec<_>>();
        for (index, child) in children.iter().enumerate().rev() {
            path.push(index);
            if let Some(target) = self.event_target_at_node(*child, x, y, path) {
                return Some(target);
            }
            path.pop();
        }
        let element = self.elements.get(&node)?;
        if element.tag == "document" || element.tag == "#text" {
            return None;
        }
        Some(UiEventTarget {
            path: path.clone(),
            tag: element.tag.clone(),
            key: element
                .attributes
                .get("data-ugr-id")
                .cloned()
                .unwrap_or_default(),
        })
    }

    /// Replace the backing document after JavaScript handled an event.
    pub fn update_from_html(&mut self, source: &str) -> Result<(), String> {
        let document = ugr_html::parse_document(source);
        let mut updated = Self::from_document_with_physical_size(
            &document,
            self.width,
            self.height,
            self.pixel_width,
            self.pixel_height,
            self.scale_factor,
        )?;
        let old_nodes = editable_nodes(&self.taffy, self.root, &self.elements);
        let new_nodes = editable_nodes(&updated.taffy, updated.root, &updated.elements);
        let focused_index = self
            .focused
            .and_then(|focused| old_nodes.iter().position(|node| *node == focused));
        updated.focused = focused_index.and_then(|index| new_nodes.get(index).copied());
        if let Some(node) = updated.focused {
            let length = updated
                .elements
                .get(&node)
                .and_then(|element| element.attributes.get("value"))
                .map_or(0, |value| value.chars().count());
            updated.caret = self.caret.min(length);
            updated.selection_anchor = self.selection_anchor.map(|anchor| anchor.min(length));
        }
        *self = updated;
        Ok(())
    }

    pub fn focus_at(&mut self, x: f32, y: f32) -> bool {
        let target = self
            .elements
            .iter()
            .filter(|(_, element)| element.tag == "input" || element.tag == "textarea")
            .filter_map(|(node, _)| self.layout.get(node).map(|rect| (*node, *rect)))
            .find(|(_, rect)| {
                x >= rect.x && y >= rect.y && x <= rect.x + rect.width && y <= rect.y + rect.height
            })
            .map(|(node, _)| node);
        self.focused = target;
        self.caret = target
            .and_then(|node| self.elements.get(&node))
            .and_then(|element| element.attributes.get("value"))
            .map_or(0, |value| value.chars().count());
        self.selection_anchor = None;
        target.is_some()
    }

    pub fn set_caret_from_point(&mut self, x: f32, y: f32) {
        self.set_caret_from_point_internal(x, y, false);
    }

    /// Update the caret during a mouse drag without discarding the selection
    /// anchor established when the button was pressed.
    pub fn set_caret_from_point_with_selection(&mut self, x: f32, y: f32) {
        self.set_caret_from_point_internal(x, y, true);
    }

    pub fn begin_selection(&mut self) {
        if self.focused.is_some() {
            self.selection_anchor = Some(self.caret);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let node = self.focused?;
        let anchor = self.selection_anchor?;
        let value = self
            .elements
            .get(&node)
            .and_then(|element| element.attributes.get("value"))?;
        let start = anchor.min(self.caret).min(value.chars().count());
        let end = anchor.max(self.caret).min(value.chars().count());
        (start != end).then(|| value.chars().skip(start).take(end - start).collect())
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let copied = self.selected_text()?;
        let node = self.focused?;
        let element = self.elements.get_mut(&node)?;
        let value = element.attributes.entry("value".into()).or_default();
        let anchor = self.selection_anchor.take()?;
        let start_index = anchor.min(self.caret).min(value.chars().count());
        let end_index = anchor.max(self.caret).min(value.chars().count());
        let start = value
            .char_indices()
            .nth(start_index)
            .map_or(0, |(index, _)| index);
        let end = value
            .char_indices()
            .nth(end_index)
            .map_or(value.len(), |(index, _)| index);
        value.replace_range(start..end, "");
        self.caret = start_index;
        Some(copied)
    }

    fn set_caret_from_point_internal(&mut self, x: f32, y: f32, preserve_selection: bool) {
        let Some(node) = self.focused else {
            return;
        };
        let Some(rect) = self.layout.get(&node).copied() else {
            return;
        };
        if y < rect.y || y > rect.y + rect.height {
            return;
        }
        if !preserve_selection && (x < rect.x || x > rect.x + rect.width) {
            return;
        }
        let x = x.clamp(rect.x, rect.x + rect.width);
        let value = self
            .elements
            .get(&node)
            .and_then(|element| element.attributes.get("value"))
            .map(String::as_str)
            .unwrap_or_default();
        let mut font = Font::new(self.typeface.clone(), 16.0);
        if let Some(size) = self
            .styles
            .get(&node)
            .and_then(|style| style.get("font-size"))
            .and_then(|size| px(size))
        {
            font.set_size(size.max(1.0));
        }
        let declarations = self.styles.get(&node);
        let text_origin = declarations
            .map(|style| text_origin_x(rect, style, value, &font))
            .unwrap_or(rect.x);
        let target = (x - text_origin).max(0.0);
        // Choose the nearest character boundary, matching browser hit testing
        // instead of always snapping to the boundary on the left.
        let mut best_index = 0;
        let mut best_distance = f32::INFINITY;
        for index in 0..=value.chars().count() {
            let prefix = value.chars().take(index).collect::<String>();
            let distance = (font.measure_str(&prefix, None).0 - target).abs();
            if distance < best_distance {
                best_distance = distance;
                best_index = index;
            }
        }
        self.caret = best_index;
        if !preserve_selection {
            self.selection_anchor = None;
        }
    }

    pub fn input_text(&mut self, text: &str) -> bool {
        if text.is_empty() || text.chars().any(char::is_control) {
            return false;
        }
        let Some(node) = self.focused else {
            return false;
        };
        let Some(element) = self.elements.get_mut(&node) else {
            return false;
        };
        let value = element.attributes.entry("value".into()).or_default();
        if let Some(anchor) = self.selection_anchor.take() {
            let (start, end) = if anchor <= self.caret {
                (anchor, self.caret)
            } else {
                (self.caret, anchor)
            };
            let start_byte = value
                .char_indices()
                .nth(start)
                .map_or(0, |(index, _)| index);
            let end_byte = value
                .char_indices()
                .nth(end)
                .map_or(value.len(), |(index, _)| index);
            value.replace_range(start_byte..end_byte, "");
            self.caret = start;
        }
        let byte = value
            .char_indices()
            .nth(self.caret)
            .map_or(value.len(), |(index, _)| index);
        value.insert_str(byte, text);
        self.caret += text.chars().count();
        true
    }

    pub fn edit_key(&mut self, key: &str) -> bool {
        self.edit_key_with_modifiers(key, false, false)
    }

    pub fn edit_key_with_modifiers(&mut self, key: &str, shift: bool, ctrl: bool) -> bool {
        let Some(node) = self.focused else {
            return false;
        };
        let Some(element) = self.elements.get_mut(&node) else {
            return false;
        };
        let value = element.attributes.entry("value".into()).or_default();
        if ctrl && (key == "KeyA" || key == "A") {
            self.selection_anchor = Some(0);
            self.caret = value.chars().count();
            return true;
        }
        if let Some(anchor) = self.selection_anchor.take() {
            if anchor != self.caret && matches!(key, "Backspace" | "Delete" | "NumpadDecimal") {
                let start_index = anchor.min(self.caret);
                let end_index = anchor.max(self.caret);
                let start = value
                    .char_indices()
                    .nth(start_index)
                    .map_or(0, |(index, _)| index);
                let end = value
                    .char_indices()
                    .nth(end_index)
                    .map_or(value.len(), |(index, _)| index);
                value.replace_range(start..end, "");
                self.caret = start_index;
                return true;
            }
            self.selection_anchor = Some(anchor);
        }
        let old_caret = self.caret;
        match key {
            "ArrowLeft" if ctrl => self.caret = previous_word_boundary(value, self.caret),
            "ArrowRight" if ctrl => self.caret = next_word_boundary(value, self.caret),
            "Backspace" if ctrl && self.caret > 0 => {
                let start_index = previous_word_boundary(value, self.caret);
                let start = value
                    .char_indices()
                    .nth(start_index)
                    .map_or(0, |(index, _)| index);
                let end = value
                    .char_indices()
                    .nth(self.caret)
                    .map_or(value.len(), |(index, _)| index);
                value.replace_range(start..end, "");
                self.caret = start_index;
            }
            "Delete" | "NumpadDecimal" if ctrl => {
                let end_index = next_word_boundary(value, self.caret);
                let start = value
                    .char_indices()
                    .nth(self.caret)
                    .map_or(value.len(), |(index, _)| index);
                let end = value
                    .char_indices()
                    .nth(end_index)
                    .map_or(value.len(), |(index, _)| index);
                value.replace_range(start..end, "");
            }
            "Backspace" if self.caret > 0 => {
                let start = value
                    .char_indices()
                    .nth(self.caret - 1)
                    .map_or(0, |(index, _)| index);
                let end = value
                    .char_indices()
                    .nth(self.caret)
                    .map_or(value.len(), |(index, _)| index);
                value.replace_range(start..end, "");
                self.caret -= 1;
            }
            "Delete" | "NumpadDecimal" => {
                let start = value
                    .char_indices()
                    .nth(self.caret)
                    .map_or(value.len(), |(index, _)| index);
                let end = value
                    .char_indices()
                    .nth(self.caret + 1)
                    .map_or(value.len(), |(index, _)| index);
                if start < end {
                    value.replace_range(start..end, "");
                }
            }
            "ArrowLeft" => self.caret = self.caret.saturating_sub(1),
            "ArrowRight" => self.caret = (self.caret + 1).min(value.chars().count()),
            "Home" => self.caret = 0,
            "End" => self.caret = value.chars().count(),
            _ => return false,
        }
        if shift {
            self.selection_anchor.get_or_insert(old_caret);
        } else {
            self.selection_anchor = None;
        }
        true
    }

    pub fn draw(&mut self) {
        let canvas = self.surface.canvas();
        // Transparent clear allows the compositor to place UI above a GPU layer.
        canvas.clear(Color::TRANSPARENT);
        canvas.save();
        canvas.scale((self.scale_factor, self.scale_factor));
        draw_node(
            canvas,
            self.root,
            &self.elements,
            &self.styles,
            &self.document.styles,
            &self.layout,
            &self.taffy,
            Color::BLACK,
            &self.typeface,
            self.focused
                .zip(self.selection_anchor)
                .map(|(node, anchor)| (node, anchor, self.caret)),
            self.focused,
            self.hovered,
            self.pressed,
        );
        if let Some(node) = self.focused {
            if let Some(rect) = self.layout.get(&node) {
                let value = self
                    .elements
                    .get(&node)
                    .and_then(|e| e.attributes.get("value"))
                    .map(String::as_str)
                    .unwrap_or_default();
                let caret = self.caret.min(value.chars().count());
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgb(30, 30, 30));
                paint.set_stroke_width(1.0);
                let mut font = Font::new(self.typeface.clone(), 16.0);
                if let Some(size) = self
                    .styles
                    .get(&node)
                    .and_then(|style| style.get("font-size"))
                    .and_then(|size| px(size))
                {
                    font.set_size(size.max(1.0));
                }
                let prefix = value.chars().take(caret).collect::<String>();
                let text_origin = self
                    .styles
                    .get(&node)
                    .map(|style| text_origin_x(*rect, style, value, &font))
                    .unwrap_or(rect.x);
                let border = self.styles.get(&node).map(border_width).unwrap_or_default();
                let x = (text_origin + font.measure_str(&prefix, Some(&paint)).0)
                    .clamp(rect.x + border, rect.x + rect.width - border);
                let (_, metrics) = font.metrics();
                let baseline = rect.y + (rect.height - (metrics.descent - metrics.ascent)) / 2.0
                    - metrics.ascent;
                canvas.draw_line(
                    (
                        x.floor() + 0.5,
                        (baseline + metrics.ascent).max(rect.y + border),
                    ),
                    (
                        x.floor() + 0.5,
                        (baseline + metrics.descent).min(rect.y + rect.height - border),
                    ),
                    &paint,
                );
            }
        }
        canvas.restore();
        self.pixel_cache_dirty = true;
    }

    /// Rebuild layout and the raster surface for a new physical window size.
    /// Keeping the source document here avoids stretching a logical-size
    /// surface into a differently-sized compositor surface after a DPI/resize
    /// event.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        let resized = Self::from_document_scaled(&self.document, width.max(1), height.max(1), 1.0)?;
        self.replace_preserving_edit_state(resized);
        Ok(())
    }

    pub fn resize_for_scale(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f32,
    ) -> Result<(), String> {
        let resized =
            Self::from_document_scaled(&self.document, width.max(1), height.max(1), scale_factor)?;
        self.replace_preserving_edit_state(resized);
        Ok(())
    }

    pub fn resize_with_physical_size(
        &mut self,
        width: u32,
        height: u32,
        pixel_width: u32,
        pixel_height: u32,
        scale_factor: f32,
    ) -> Result<(), String> {
        let resized = Self::from_document_with_physical_size(
            &self.document,
            width.max(1),
            height.max(1),
            pixel_width.max(1),
            pixel_height.max(1),
            scale_factor,
        )?;
        self.replace_preserving_edit_state(resized);
        Ok(())
    }

    /// Rebuilding the layout creates fresh `HtmlNode` clones from the source
    /// document. Carry over the mutable form state so a resize behaves like a
    /// visual/layout change rather than a document reload.
    fn replace_preserving_edit_state(&mut self, mut resized: Self) {
        let old_nodes = editable_nodes(&self.taffy, self.root, &self.elements);
        let new_nodes = editable_nodes(&resized.taffy, resized.root, &resized.elements);
        let values = old_nodes
            .iter()
            .map(|node| {
                self.elements
                    .get(node)
                    .and_then(|element| element.attributes.get("value").cloned())
            })
            .collect::<Vec<_>>();
        for (node, value) in new_nodes.iter().zip(values) {
            if let Some(element) = resized.elements.get_mut(node) {
                match value {
                    Some(value) => {
                        element.attributes.insert("value".into(), value);
                    }
                    None => {
                        element.attributes.remove("value");
                    }
                }
            }
        }
        let focused_index = self
            .focused
            .and_then(|focused| old_nodes.iter().position(|node| *node == focused));
        resized.focused = focused_index.and_then(|index| new_nodes.get(index).copied());
        resized.hovered = self
            .hovered
            .and_then(|node| old_nodes.iter().position(|old| *old == node))
            .and_then(|index| new_nodes.get(index).copied());
        resized.pressed = self
            .pressed
            .and_then(|node| old_nodes.iter().position(|old| *old == node))
            .and_then(|index| new_nodes.get(index).copied());
        if let Some(node) = resized.focused {
            let length = resized
                .elements
                .get(&node)
                .and_then(|element| element.attributes.get("value"))
                .map_or(0, |value| value.chars().count());
            resized.caret = self.caret.min(length);
            resized.selection_anchor = self.selection_anchor.map(|anchor| anchor.min(length));
        }
        *self = resized;
    }

    pub fn snapshot(&self) -> &skia_safe::Surface {
        &self.surface
    }

    /// Copy the current Skia raster surface to tightly packed RGBA bytes.
    pub fn rgba_pixels(&mut self) -> Vec<u8> {
        self.refresh_pixel_cache();
        self.pixel_cache.clone()
    }

    /// Returns cached RGBA pixels, refreshing the Skia readback only after a
    /// draw. Compositors can use this to avoid repeating the expensive readback
    /// for input events that do not change the frame.
    pub fn rgba_pixels_ref(&mut self) -> &[u8] {
        self.refresh_pixel_cache();
        &self.pixel_cache
    }

    fn refresh_pixel_cache(&mut self) {
        if !self.pixel_cache_dirty {
            return;
        }
        let Some(pixmap) = self.surface.peek_pixels() else {
            self.pixel_cache.clear();
            self.pixel_cache_dirty = false;
            return;
        };
        self.pixel_cache.clear();
        self.pixel_cache
            .reserve((self.pixel_width * self.pixel_height * 4) as usize);
        let width = self.pixel_width as usize;
        let height = self.pixel_height as usize;
        let row_bytes = pixmap.row_bytes();
        let is_bgra = pixmap.color_type() == ColorType::BGRA8888;
        let is_premul = pixmap.alpha_type() == AlphaType::Premul;
        // Skia's N32 raster surface is a native 32-bit premultiplied buffer.
        // Read rows directly instead of crossing the FFI boundary once per
        // pixel through Pixmap::get_color().
        unsafe {
            let base = pixmap.addr() as *const u8;
            for y in 0..height {
                let row = std::slice::from_raw_parts(base.add(y * row_bytes), width * 4);
                for pixel in row.as_chunks::<4>().0 {
                    let (mut r, mut g, mut b, a) = if is_bgra {
                        (pixel[2], pixel[1], pixel[0], pixel[3])
                    } else {
                        (pixel[0], pixel[1], pixel[2], pixel[3])
                    };
                    if is_premul && a != 0 && a != 255 {
                        let alpha = u16::from(a);
                        r = ((u16::from(r) * 255 + alpha / 2) / alpha).min(255) as u8;
                        g = ((u16::from(g) * 255 + alpha / 2) / alpha).min(255) as u8;
                        b = ((u16::from(b) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                    self.pixel_cache.extend_from_slice(&[r, g, b, a]);
                }
            }
        }
        self.pixel_cache_dirty = false;
    }
}

fn find_document_node_by_key_mut<'a>(
    node: &'a mut HtmlNode,
    key: &str,
) -> Option<&'a mut HtmlNode> {
    if node
        .attributes
        .get("data-ugr-id")
        .is_some_and(|value| value == key)
    {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_document_node_by_key_mut(child, key) {
            return Some(found);
        }
    }
    None
}

fn find_document_node_by_rendered_path_mut<'a>(
    node: &'a mut HtmlNode,
    path: &[usize],
) -> Option<&'a mut HtmlNode> {
    if path.is_empty() {
        return Some(node);
    }
    let mut rendered_index = 0;
    for child in &mut node.children {
        if is_non_rendering_tag(&child.tag) {
            continue;
        }
        if rendered_index == path[0] {
            return find_document_node_by_rendered_path_mut(child, &path[1..]);
        }
        rendered_index += 1;
    }
    None
}

fn editable_nodes(
    taffy: &TaffyTree<()>,
    node: NodeId,
    elements: &HashMap<NodeId, HtmlNode>,
) -> Vec<NodeId> {
    let mut result = Vec::new();
    if elements
        .get(&node)
        .is_some_and(|element| matches!(element.tag.as_str(), "input" | "textarea"))
    {
        result.push(node);
    }
    for child in taffy.child_ids(node) {
        result.extend(editable_nodes(taffy, child, elements));
    }
    result
}

fn build_tree(
    taffy: &mut TaffyTree<()>,
    elements: &mut HashMap<NodeId, HtmlNode>,
    styles: &mut HashMap<NodeId, BTreeMap<String, String>>,
    rules: &[CssRule],
    node: &HtmlNode,
) -> Result<NodeId, String> {
    let declarations = computed_style(node, rules);
    let children = node
        .children
        .iter()
        .filter(|child| !is_non_rendering_tag(&child.tag))
        .map(|child| build_tree(taffy, elements, styles, rules, child))
        .collect::<Result<Vec<_>, _>>()?;
    let style = style_for(node, &declarations);
    let id = if children.is_empty() {
        taffy.new_leaf(style)
    } else {
        taffy.new_with_children(style, &children)
    }
    .map_err(|error| format!("could not create UI node: {error:?}"))?;
    elements.insert(id, node.clone());
    styles.insert(id, declarations);
    Ok(id)
}

fn is_non_rendering_tag(tag: &str) -> bool {
    matches!(
        tag,
        "#text" | "head" | "title" | "meta" | "link" | "style" | "script" | "option"
    )
}

fn computed_style(node: &HtmlNode, rules: &[CssRule]) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    apply_user_agent_defaults(node, &mut result);
    for rule in rules {
        if matches_selector(node, &rule.selector) {
            result.extend(
                rule.declarations
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
    }
    if let Some(inline) = node.attributes.get("style") {
        result.extend(parse_declarations(inline));
    }
    result
}

/// Small user-agent stylesheet matching the defaults users expect from a
/// browser. Author rules and inline styles are applied afterwards and win.
fn apply_user_agent_defaults(node: &HtmlNode, style: &mut BTreeMap<String, String>) {
    match node.tag.as_str() {
        "body" => {
            style.insert("display".into(), "block".into());
            style.insert("margin".into(), "8px".into());
            style.insert("color".into(), "black".into());
        }
        "h1" => {
            style.insert("font-size".into(), "32px".into());
            style.insert("margin".into(), "21px 0".into());
        }
        "h2" => {
            style.insert("font-size".into(), "24px".into());
            style.insert("margin".into(), "19px 0".into());
        }
        "h3" => {
            style.insert("font-size".into(), "19px".into());
            style.insert("margin".into(), "17px 0".into());
        }
        "p" => {
            style.insert("margin".into(), "16px 0".into());
        }
        "button" => {
            style.insert("display".into(), "block".into());
            style.insert("padding".into(), "2px 8px".into());
            style.insert("min-height".into(), "28px".into());
            style.insert("background-color".into(), "#efefef".into());
            style.insert("border-radius".into(), "2px".into());
            style.insert("border".into(), "1px solid #767676".into());
            style.insert("color".into(), "black".into());
            style.insert("font-size".into(), "13.3333px".into());
        }
        "input" | "textarea" | "select" => {
            style.insert("display".into(), "block".into());
            style.insert("padding".into(), "1px 2px".into());
            style.insert("min-height".into(), "20px".into());
            style.insert("background-color".into(), "white".into());
            style.insert("border".into(), "2px inset #767676".into());
            style.insert("border-radius".into(), "2px".into());
            style.insert("color".into(), "black".into());
            style.insert("font-size".into(), "13.3333px".into());
            if node.tag == "textarea" {
                style.insert("width".into(), "185px".into());
                style.insert("height".into(), "40px".into());
            } else if node.tag == "select"
                || (!node.attributes.contains_key("value")
                    && !node.attributes.contains_key("placeholder"))
            {
                style.insert("width".into(), "185px".into());
            }
            if node.tag == "input" {
                let input_type = node
                    .attributes
                    .get("type")
                    .map(String::as_str)
                    .unwrap_or("text");
                if matches!(input_type, "checkbox" | "radio") {
                    style.insert("width".into(), "13px".into());
                    style.insert("height".into(), "13px".into());
                } else {
                    style.insert("width".into(), "185px".into());
                    style.insert("height".into(), "20px".into());
                }
            }
        }
        "ul" | "ol" => {
            style.insert("margin".into(), "16px 0".into());
            style.insert("padding".into(), "0 0 0 40px".into());
        }
        _ => {}
    }
}

fn matches_selector(node: &HtmlNode, selector: &str) -> bool {
    if selector.contains(',') {
        return selector.split(',').any(|part| matches_selector(node, part));
    }
    let selector = selector.split_whitespace().last().unwrap_or_default();
    let mut tag = selector;
    let mut requirements = Vec::new();
    if let Some(index) = selector.find(['.', '#']) {
        tag = &selector[..index];
        let mut rest = &selector[index..];
        while !rest.is_empty() {
            let kind = rest.as_bytes()[0] as char;
            let token = rest[1..].split(['.', '#']).next().unwrap_or_default();
            requirements.push((kind, token));
            rest = &rest[1 + token.len()..];
        }
    }
    if !tag.is_empty() && tag != "*" && !tag.eq_ignore_ascii_case(&node.tag) {
        return false;
    }
    let id = node
        .attributes
        .get("id")
        .map(String::as_str)
        .unwrap_or_default();
    let classes = node
        .attributes
        .get("class")
        .map(String::as_str)
        .unwrap_or_default();
    requirements.into_iter().all(|(kind, token)| {
        kind == '#' && id == token
            || kind == '.' && classes.split_whitespace().any(|class| class == token)
    })
}

/// Apply interaction pseudo-class declarations during painting. These are
/// deliberately excluded from layout construction: changing a hover color or
/// border must never move neighboring controls while the pointer is moving.
#[allow(clippy::too_many_arguments)]
fn apply_state_styles(
    node: &HtmlNode,
    node_id: NodeId,
    taffy: &TaffyTree<()>,
    rules: &[CssRule],
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    focused: Option<NodeId>,
    output: &mut BTreeMap<String, String>,
) {
    for rule in rules {
        if matches_state_selector(
            node,
            node_id,
            taffy,
            &rule.selector,
            hovered,
            pressed,
            focused,
        ) {
            output.extend(
                rule.declarations
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
    }
}

fn matches_state_selector(
    node: &HtmlNode,
    node_id: NodeId,
    taffy: &TaffyTree<()>,
    selector: &str,
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    focused: Option<NodeId>,
) -> bool {
    selector.split(',').any(|part| {
        let mut base = String::with_capacity(part.len());
        let mut matches_state = true;
        let mut rest = part.trim();
        while let Some(index) = rest.find(':') {
            base.push_str(&rest[..index]);
            rest = &rest[index + 1..];
            let end = rest.find([':', ' ', '\t']).unwrap_or(rest.len());
            let pseudo = &rest[..end];
            let active = match pseudo.to_ascii_lowercase().as_str() {
                "hover" => hovered.is_some_and(|target| contains_node(taffy, node_id, target)),
                "active" => pressed.is_some_and(|target| contains_node(taffy, node_id, target)),
                "focus" | "focus-visible" => focused == Some(node_id),
                "checked" => node.attributes.contains_key("checked"),
                "disabled" => node.attributes.contains_key("disabled"),
                "enabled" => !node.attributes.contains_key("disabled"),
                _ => false,
            };
            matches_state &= active;
            rest = &rest[end..];
        }
        base.push_str(rest);
        matches_state && matches_selector(node, base.trim())
    })
}

fn contains_node(taffy: &TaffyTree<()>, root: NodeId, target: NodeId) -> bool {
    root == target
        || taffy
            .child_ids(root)
            .any(|child| contains_node(taffy, child, target))
}

fn has_author_property(node: &HtmlNode, rules: &[CssRule], property: &str) -> bool {
    node.attributes
        .get("style")
        .map(|style| parse_declarations(style))
        .is_some_and(|style| style.contains_key(property))
        || rules.iter().any(|rule| {
            rule.declarations.contains_key(property) && matches_selector(node, &rule.selector)
        })
}

fn style_for(node: &HtmlNode, declarations: &BTreeMap<String, String>) -> Style {
    let display = match declarations.get("display").map(String::as_str) {
        Some(value) if value.eq_ignore_ascii_case("none") => Display::None,
        Some(value) if value.eq_ignore_ascii_case("flex") => Display::Flex,
        Some(value) if value.eq_ignore_ascii_case("inline") => Display::Block,
        _ => Display::Block,
    };
    let flex_direction = if declarations
        .get("flex-direction")
        .is_some_and(|value| value.eq_ignore_ascii_case("row"))
    {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };
    let mut style = Style {
        display,
        flex_direction,
        ..Style::default()
    };
    if declarations
        .get("flex-wrap")
        .is_some_and(|value| value.eq_ignore_ascii_case("wrap"))
    {
        style.flex_wrap = FlexWrap::Wrap;
    }
    if let Some(value) = declarations.get("width").and_then(|v| dimension(v)) {
        style.size.width = value;
    }
    if let Some(value) = declarations.get("height").and_then(|v| dimension(v)) {
        style.size.height = value;
    }
    if let Some(value) = declarations
        .get("min-width")
        .and_then(|v| dimension_auto(v))
    {
        style.min_size.width = value;
    }
    if let Some(value) = declarations
        .get("min-height")
        .and_then(|v| dimension_auto(v))
    {
        style.min_size.height = value;
    }
    // Taffy does not measure text nodes by itself. Provide a small intrinsic
    // size so spans, labels and controls remain visible without CSS dimensions.
    let text = if node.tag == "input" {
        node.attributes
            .get("value")
            .or_else(|| node.attributes.get("placeholder"))
            .map(String::as_str)
            .unwrap_or_default()
    } else {
        node.text.as_str()
    };
    if !text.is_empty() {
        if declarations.get("width").is_none() {
            style.size.width = Dimension::length((text.chars().count() as f32 * 8.0) + 12.0);
        }
        if declarations.get("height").is_none() {
            style.size.height = Dimension::length(24.0);
        }
    }
    if let Some(values) = css_box_values(declarations, "padding") {
        style.padding = taffy::Rect {
            top: LengthPercentage::length(values[0]),
            right: LengthPercentage::length(values[1]),
            bottom: LengthPercentage::length(values[2]),
            left: LengthPercentage::length(values[3]),
        };
    }
    if let Some(values) = css_box_values(declarations, "margin") {
        style.margin = taffy::Rect {
            top: LengthPercentageAuto::length(values[0]),
            right: LengthPercentageAuto::length(values[1]),
            bottom: LengthPercentageAuto::length(values[2]),
            left: LengthPercentageAuto::length(values[3]),
        };
    }
    style
}

fn css_box_values(declarations: &BTreeMap<String, String>, property: &str) -> Option<[f32; 4]> {
    let mut values = declarations
        .get(property)
        .and_then(|v| box_values(v))
        .unwrap_or([0.0; 4]);
    let sides = [
        format!("{property}-top"),
        format!("{property}-right"),
        format!("{property}-bottom"),
        format!("{property}-left"),
    ];
    for (index, side) in sides.iter().enumerate() {
        if let Some(value) = declarations.get(side).and_then(|v| px(v)) {
            values[index] = value;
        }
    }
    let has_side = sides.iter().any(|side| declarations.contains_key(side));
    if declarations.contains_key(property) || has_side {
        Some(values)
    } else {
        None
    }
}

fn border_width(declarations: &BTreeMap<String, String>) -> f32 {
    declarations
        .get("border")
        .map(|value| parse_border(value).0)
        .unwrap_or_default()
}

fn text_origin_x(
    rect: UiRect,
    declarations: &BTreeMap<String, String>,
    text: &str,
    font: &Font,
) -> f32 {
    let padding = css_box_values(declarations, "padding").unwrap_or([0.0; 4]);
    let border = border_width(declarations);
    let content_width = (rect.width - border * 2.0 - padding[1] - padding[3]).max(0.0);
    let text_width = font.measure_str(text, None).0;
    let left = rect.x + border + padding[3];
    match declarations.get("text-align").map(String::as_str) {
        Some(value) if value.eq_ignore_ascii_case("right") => {
            left + (content_width - text_width).max(0.0)
        }
        Some(value) if value.eq_ignore_ascii_case("center") => {
            left + (content_width - text_width).max(0.0) / 2.0
        }
        _ => left,
    }
}

fn previous_word_boundary(value: &str, caret: usize) -> usize {
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = caret.min(chars.len());
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    index
}

fn next_word_boundary(value: &str, caret: usize) -> usize {
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = caret.min(chars.len());
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    while index < chars.len() && !chars[index].is_whitespace() {
        index += 1;
    }
    index
}

fn collect_layout(
    taffy: &TaffyTree<()>,
    node: NodeId,
    output: &mut HashMap<NodeId, UiRect>,
    parent_x: f32,
    parent_y: f32,
) -> Result<(), String> {
    let value = taffy
        .layout(node)
        .map_err(|error| format!("could not read UI layout: {error:?}"))?;
    output.insert(
        node,
        UiRect {
            x: parent_x + value.location.x,
            y: parent_y + value.location.y,
            width: value.size.width,
            height: value.size.height,
        },
    );
    for child in taffy.child_ids(node) {
        collect_layout(
            taffy,
            child,
            output,
            parent_x + value.location.x,
            parent_y + value.location.y,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    canvas: &Canvas,
    node: NodeId,
    elements: &HashMap<NodeId, HtmlNode>,
    styles: &HashMap<NodeId, BTreeMap<String, String>>,
    rules: &[CssRule],
    layout: &HashMap<NodeId, UiRect>,
    taffy: &TaffyTree<()>,
    inherited_color: Color,
    typeface: &Typeface,
    selection: Option<(NodeId, usize, usize)>,
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
) {
    let Some(rect) = layout.get(&node) else {
        return;
    };
    let Some(element) = elements.get(&node) else {
        return;
    };
    let mut visual_style = styles.get(&node).cloned().unwrap_or_default();
    apply_state_styles(
        element,
        node,
        taffy,
        rules,
        hovered,
        pressed,
        focused,
        &mut visual_style,
    );
    let has_author_background = has_author_property(element, rules, "background")
        || has_author_property(element, rules, "background-color");
    let has_author_border = has_author_property(element, rules, "border");
    let declarations = Some(&visual_style);
    if declarations
        .and_then(|d| d.get("display"))
        .is_some_and(|v| v.eq_ignore_ascii_case("none"))
    {
        return;
    }
    if declarations
        .and_then(|d| d.get("visibility"))
        .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
    {
        return;
    }
    if let Some(color) = declarations
        .and_then(|d| d.get("background-color").or_else(|| d.get("background")))
        .and_then(|v| parse_color(v))
    {
        let mut paint = Paint::default();
        paint.set_color(color);
        if let Some(radius) = declarations
            .and_then(|d| d.get("border-radius"))
            .and_then(|v| px(v))
        {
            canvas.draw_round_rect(
                SkiaRect::from_xywh(rect.x, rect.y, rect.width, rect.height),
                radius,
                radius,
                &paint,
            );
        } else {
            canvas.draw_rect(
                SkiaRect::from_xywh(rect.x, rect.y, rect.width, rect.height),
                &paint,
            );
        }
    }
    let text_color = declarations
        .and_then(|d| d.get("color"))
        .and_then(|v| parse_color(v))
        .unwrap_or(inherited_color);
    if let Some((_, anchor, caret)) = selection.filter(|(selected, _, _)| *selected == node) {
        if let Some(element) = elements.get(&node) {
            let value = element
                .attributes
                .get("value")
                .map(String::as_str)
                .unwrap_or_default();
            let start_index = anchor.min(caret).min(value.chars().count());
            let end_index = anchor.max(caret).min(value.chars().count());
            if end_index > start_index {
                let mut font = Font::new(typeface.clone(), 16.0);
                if let Some(size) = declarations
                    .and_then(|d| d.get("font-size"))
                    .and_then(|v| px(v))
                {
                    font.set_size(size.max(1.0));
                }
                let start_text = value.chars().take(start_index).collect::<String>();
                let end_text = value.chars().take(end_index).collect::<String>();
                let text_origin = declarations
                    .map(|style| text_origin_x(*rect, style, value, &font))
                    .unwrap_or(rect.x);
                let border = declarations.map(border_width).unwrap_or_default();
                let start = (text_origin + font.measure_str(&start_text, None).0)
                    .clamp(rect.x + border, rect.x + rect.width - border);
                let end = (text_origin + font.measure_str(&end_text, None).0)
                    .clamp(rect.x + border, rect.x + rect.width - border);
                let mut paint = Paint::default();
                paint.set_color(Color::from_argb(100, 50, 120, 220));
                canvas.draw_rect(
                    SkiaRect::from_xywh(
                        start,
                        rect.y + 2.0,
                        (end - start).max(1.0),
                        (rect.height - 4.0).max(1.0),
                    ),
                    &paint,
                );
            }
        }
    }
    if let Some(element) = elements.get(&node) {
        let is_control = matches!(
            element.tag.as_str(),
            "input" | "button" | "textarea" | "select"
        );
        let uses_default_button_state = element.tag == "button"
            && !has_author_background
            && (hovered.is_some_and(|target| contains_node(taffy, node, target))
                || pressed.is_some_and(|target| contains_node(taffy, node, target)));
        if is_control
            && (declarations
                .and_then(|d| d.get("background-color"))
                .is_none()
                && declarations.and_then(|d| d.get("background")).is_none()
                || uses_default_button_state)
        {
            let mut paint = Paint::default();
            paint.set_color(if pressed == Some(node) {
                Color::from_rgb(195, 200, 208)
            } else if hovered == Some(node) && element.tag == "button" {
                Color::from_rgb(210, 218, 230)
            } else if element.tag == "button" {
                Color::from_rgb(225, 228, 232)
            } else {
                Color::from_rgb(248, 249, 250)
            });
            let bounds = SkiaRect::from_xywh(rect.x, rect.y, rect.width, rect.height);
            if let Some(radius) = declarations
                .and_then(|d| d.get("border-radius"))
                .and_then(|v| px(v))
            {
                canvas.draw_round_rect(bounds, radius, radius, &paint);
            } else {
                canvas.draw_rect(bounds, &paint);
            }
            if declarations
                .and_then(|d| d.get("border"))
                .is_some_and(|value| value.to_ascii_lowercase().contains("inset"))
            {
                let mut highlight = Paint::default();
                highlight.set_style(PaintStyle::Stroke);
                highlight.set_stroke_width(1.0);
                highlight.set_color(Color::from_rgb(255, 255, 255));
                let inner = SkiaRect::from_xywh(
                    rect.x + 1.0,
                    rect.y + 1.0,
                    (rect.width - 2.0).max(1.0),
                    (rect.height - 2.0).max(1.0),
                );
                canvas.draw_rect(inner, &highlight);
            }
        }
        if matches!(element.tag.as_str(), "input" | "button")
            && matches!(
                element.attributes.get("type").map(String::as_str),
                Some("checkbox") | Some("radio")
            )
        {
            let checked = element.attributes.contains_key("checked");
            let mark = checked || pressed == Some(node);
            if mark {
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgb(30, 90, 180));
                let inset = (rect.height.min(rect.width) * 0.25).max(2.0);
                canvas.draw_rect(
                    SkiaRect::from_xywh(
                        rect.x + inset,
                        rect.y + inset,
                        (rect.width - inset * 2.0).max(1.0),
                        (rect.height - inset * 2.0).max(1.0),
                    ),
                    &paint,
                );
            }
        }
        let text = if matches!(element.tag.as_str(), "input" | "textarea") {
            element
                .attributes
                .get("value")
                .or_else(|| element.attributes.get("placeholder"))
                .map(String::as_str)
                .unwrap_or_default()
        } else if element.tag == "select" {
            element
                .children
                .iter()
                .find(|child| child.tag == "option")
                .map(|option| option.text.as_str())
                .unwrap_or_default()
        } else {
            element.text.as_str()
        };
        if !text.is_empty() {
            let mut paint = Paint::default();
            paint.set_color(text_color);
            let mut font = Font::new(typeface.clone(), 16.0);
            // Browsers use a 16px default font size. Skia's default SkFont
            // has a zero size, which silently produces no visible glyphs.
            font.set_size(16.0);
            if let Some(size) = declarations
                .and_then(|d| d.get("font-size"))
                .and_then(|v| px(v))
            {
                font.set_size(size.max(1.0));
            }
            let padding = declarations
                .and_then(|d| css_box_values(d, "padding"))
                .unwrap_or([0.0; 4]);
            let border = declarations.map(border_width).unwrap_or_default();
            let content_width = (rect.width - border * 2.0 - padding[1] - padding[3]).max(0.0);
            let text_width = font.measure_str(text, Some(&paint)).0;
            let text_left = rect.x + border + padding[3];
            let is_button = element.tag == "button";
            let is_control =
                is_button || matches!(element.tag.as_str(), "input" | "textarea" | "select");
            let text_x = if is_button
                && !declarations
                    .and_then(|d| d.get("text-align"))
                    .is_some_and(|v| v.eq_ignore_ascii_case("left"))
            {
                text_left + (content_width - text_width).max(0.0) / 2.0
            } else if declarations
                .and_then(|d| d.get("text-align"))
                .is_some_and(|v| v.eq_ignore_ascii_case("right"))
            {
                text_left + (content_width - text_width).max(0.0)
            } else if declarations
                .and_then(|d| d.get("text-align"))
                .is_some_and(|v| v.eq_ignore_ascii_case("center"))
            {
                text_left + (content_width - text_width).max(0.0) / 2.0
            } else {
                text_left
            };
            let text_y = if is_control {
                let (_, metrics) = font.metrics();
                rect.y + (rect.height - (metrics.descent - metrics.ascent)) / 2.0 - metrics.ascent
            } else {
                rect.y + padding[0] + font.size()
            };
            if is_control {
                // Native controls clip their value to the content box. Without
                // this, long input values paint over the border and adjacent
                // controls instead of being scrolled/clipped by the widget.
                canvas.save();
                canvas.clip_rect(
                    SkiaRect::from_xywh(
                        rect.x + border,
                        rect.y + border,
                        (rect.width - border * 2.0).max(0.0),
                        (rect.height - border * 2.0).max(0.0),
                    ),
                    None,
                    false,
                );
                canvas.draw_str(text, (text_x, text_y), &font, &paint);
                canvas.restore();
            } else {
                canvas.draw_str(text, (text_x, text_y), &font, &paint);
            }
        }
        if declarations.is_some_and(|d| d.contains_key("border"))
            || matches!(
                element.tag.as_str(),
                "input" | "button" | "textarea" | "select"
            )
        {
            let mut paint = Paint::default();
            paint.set_style(PaintStyle::Stroke);
            let (border_width, mut border_color) = declarations
                .and_then(|d| d.get("border"))
                .map(|value| parse_border(value))
                .unwrap_or((1.0, Color::from_rgb(80, 80, 80)));
            if focused == Some(node) && is_control && !has_author_border {
                border_color = Color::from_rgb(40, 110, 210);
            }
            paint.set_stroke_width(border_width);
            paint.set_color(border_color);
            let bounds =
                SkiaRect::from_xywh(rect.x, rect.y, rect.width.max(1.0), rect.height.max(1.0));
            if let Some(radius) = declarations
                .and_then(|d| d.get("border-radius"))
                .and_then(|v| px(v))
            {
                canvas.draw_round_rect(bounds, radius, radius, &paint);
            } else {
                canvas.draw_rect(bounds, &paint);
            }
        }
    }
    for child in taffy.child_ids(node) {
        draw_node(
            canvas, child, elements, styles, rules, layout, taffy, text_color, typeface, selection,
            focused, hovered, pressed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn computes_layout_and_draws_nested_nodes() {
        let document = ugr_html::parse_document(
            r#"<div style="width:100px;height:50px;background-color:#ff0000"><span style="color:#ffffff">Hello</span></div>"#,
        );
        let mut renderer = UiRenderer::from_document(&document, 320, 200).unwrap();
        renderer.draw();
        assert_eq!(renderer.width, 320);
        let pixels = renderer.rgba_pixels();
        assert_eq!(pixels.len(), 320 * 200 * 4);
        assert!(pixels
            .chunks(4)
            .any(|pixel| pixel.get(3).copied().unwrap_or(0) != 0));
    }
    #[test]
    fn applies_stylesheet_rules() {
        let renderer = UiRenderer::from_html(
            r#"<style>.card { width: 42px; }</style><div class="card"></div>"#,
            100,
            100,
        )
        .unwrap();
        assert!(renderer.layout(renderer.root()).unwrap().width >= 42.0);
    }

    #[test]
    fn calculator_document_produces_visible_pixels() {
        let source = std::fs::read_to_string("../../tests/scripts/calculator.html").unwrap();
        let mut renderer = UiRenderer::from_html(&source, 1280, 720).unwrap();
        renderer.draw();
        let pixels = renderer.rgba_pixels();
        assert!(pixels
            .chunks(4)
            .any(|pixel| pixel.get(3).copied().unwrap_or(0) != 0));
    }

    #[test]
    fn renders_text_with_css_foreground_color() {
        let mut renderer = UiRenderer::from_html(
            r#"<html><body style="margin:0"><div style="width:200px;height:200px;background-color:black;color:white">123</div></body></html>"#,
            320,
            200,
        )
        .unwrap();
        renderer.draw();
        let pixels = renderer.rgba_pixels();
        assert!(renderer
            .elements
            .values()
            .any(|element| element.text == "123"));
        let first_bright_x = pixels
            .chunks(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200)
            .map(|(index, _)| index % 320)
            .min()
            .unwrap();
        assert!(first_bright_x < 6);
        assert!(pixels
            .chunks(4)
            .any(|pixel| { pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200 && pixel[3] > 0 }));
    }

    #[test]
    fn does_not_render_document_metadata() {
        let renderer = UiRenderer::from_html(
            r#"<html><head><title>Window title</title><meta charset="utf-8"></head><body><div>Visible</div></body></html>"#,
            320,
            200,
        )
        .unwrap();
        assert!(renderer
            .elements
            .values()
            .all(|element| !matches!(element.tag.as_str(), "head" | "title" | "meta")));
    }

    #[test]
    fn renders_scaled_surface_without_resampling_text() {
        let mut renderer = UiRenderer::from_document_scaled(
            &ugr_html::parse_document(
                r#"<div style="width:100px;height:40px;background:black;color:white">Hi</div>"#,
            ),
            320,
            200,
            2.0,
        )
        .unwrap();
        renderer.draw();
        assert_eq!(renderer.rgba_pixels().len(), 640 * 400 * 4);
    }

    #[test]
    fn applies_browser_like_control_defaults() {
        let renderer = UiRenderer::from_html(
            r#"<html><body style="margin:0"><input><textarea></textarea><select><option>One</option></select></body></html>"#,
            320,
            200,
        )
        .unwrap();
        let controls = renderer
            .elements
            .values()
            .filter(|element| matches!(element.tag.as_str(), "input" | "textarea" | "select"))
            .count();
        assert_eq!(controls, 3);
        let input = renderer
            .elements
            .iter()
            .find(|(_, element)| element.tag == "input")
            .map(|(node, _)| renderer.layout(*node).unwrap())
            .unwrap();
        assert_eq!(input.width, 185.0);
        assert_eq!(input.height, 20.0);
    }

    #[test]
    fn applies_individual_margin_sides() {
        let renderer = UiRenderer::from_html(
            r#"<div><input id="display" style="width:100px;height:20px;margin-bottom:8px"><div id="keys" style="width:100px;height:20px"></div></div>"#,
            200,
            100,
        )
        .unwrap();
        let display = renderer
            .elements
            .iter()
            .find(|(_, element)| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|id| id == "display")
            })
            .map(|(node, _)| renderer.layout(*node).unwrap())
            .unwrap();
        let keys = renderer
            .elements
            .iter()
            .find(|(_, element)| element.attributes.get("id").is_some_and(|id| id == "keys"))
            .map(|(node, _)| renderer.layout(*node).unwrap())
            .unwrap();
        assert!(keys.y >= display.y + display.height + 8.0);
    }

    #[test]
    fn edits_focused_input_text() {
        let mut renderer = UiRenderer::from_html(
            r#"<body style="margin:0"><input value="ab"></body>"#,
            200,
            100,
        )
        .unwrap();
        assert!(renderer.focus_at(4.0, 4.0));
        assert!(renderer.input_text("中"));
        assert!(renderer.edit_key("Backspace"));
        let value = renderer
            .elements
            .values()
            .find(|element| element.tag == "input")
            .and_then(|element| element.attributes.get("value"))
            .unwrap();
        assert_eq!(value, "ab");
    }

    #[test]
    fn deletes_at_caret_and_supports_forward_delete() {
        let mut renderer = UiRenderer::from_html(
            r#"<body style="margin:0"><input value="abc"></body>"#,
            200,
            100,
        )
        .unwrap();
        assert!(renderer.focus_at(4.0, 4.0));
        renderer.caret = 1;
        assert!(renderer.edit_key("Delete"));
        let value = renderer
            .elements
            .values()
            .find(|element| element.tag == "input")
            .and_then(|element| element.attributes.get("value"))
            .unwrap();
        assert_eq!(value, "ac");
    }

    #[test]
    fn deletes_selected_text_as_a_single_edit() {
        let mut renderer = UiRenderer::from_html(
            r#"<body style="margin:0"><input value="abcd"></body>"#,
            200,
            100,
        )
        .unwrap();
        assert!(renderer.focus_at(4.0, 4.0));
        renderer.selection_anchor = Some(1);
        renderer.caret = 3;
        assert!(renderer.edit_key("Backspace"));
        let value = renderer
            .elements
            .values()
            .find(|element| element.tag == "input")
            .and_then(|element| element.attributes.get("value"))
            .unwrap();
        assert_eq!(value, "ad");
    }

    #[test]
    fn caret_hit_testing_uses_border_and_padding() {
        let mut renderer = UiRenderer::from_html(
            r#"<body style="margin:0"><input value="abc" style="padding: 1px 2px; border: 2px solid black"></body>"#,
            200,
            100,
        )
        .unwrap();
        let node = renderer
            .elements
            .iter()
            .find(|(_, element)| element.tag == "input")
            .map(|(node, _)| *node)
            .unwrap();
        let rect = renderer.layout(node).unwrap();
        assert!(renderer.focus_at(rect.x + 1.0, rect.y + 1.0));
        let style = renderer.styles.get(&node).unwrap();
        let font = Font::new(renderer.typeface.clone(), 13.3333);
        let first_width = font.measure_str("a", None).0;
        let padding_left = css_box_values(style, "padding").unwrap()[3];
        renderer.set_caret_from_point(
            rect.x + border_width(style) + padding_left + first_width + 0.1,
            rect.y + rect.height / 2.0,
        );
        assert_eq!(renderer.caret, 1);
    }

    #[test]
    fn preserves_input_state_when_resizing() {
        let mut renderer = UiRenderer::from_html(
            r#"<body style="margin:0"><input value="before"></body>"#,
            200,
            100,
        )
        .unwrap();
        assert!(renderer.focus_at(4.0, 4.0));
        renderer.edit_key("End");
        assert!(renderer.input_text(" after"));
        renderer
            .resize_with_physical_size(320, 180, 640, 360, 2.0)
            .unwrap();
        let input = renderer
            .elements
            .values()
            .find(|element| element.tag == "input")
            .unwrap();
        assert_eq!(
            input.attributes.get("value").map(String::as_str),
            Some("before after")
        );
        assert!(renderer.focused.is_some());
        assert_eq!(renderer.caret, "before after".chars().count());
    }

    #[test]
    fn exposes_and_mutates_stable_nodes() {
        let mut renderer = UiRenderer::from_html(
            r#"<body><button id="action" data-ugr-id="stable">Run</button></body>"#,
            200,
            100,
        )
        .unwrap();
        let node = renderer.node_by_id("action").unwrap();
        assert_eq!(node.target.key, "stable");
        assert!(renderer.set_text(&node.target, "Done").unwrap());
        assert_eq!(renderer.node_by_id("action").unwrap().element.text, "Done");
        assert!(renderer
            .set_attribute(&node.target, "aria-label", "completed")
            .unwrap());
        assert_eq!(
            renderer
                .node_by_id("action")
                .unwrap()
                .element
                .attributes
                .get("aria-label")
                .map(String::as_str),
            Some("completed")
        );
    }

    #[test]
    fn activates_checkbox_and_preserves_state() {
        let mut renderer = UiRenderer::from_html(
            r#"<body><input id="flag" type="checkbox"></body>"#,
            200,
            100,
        )
        .unwrap();
        let target = renderer.node_by_id("flag").unwrap().target;
        assert!(renderer.activate(&target).unwrap());
        assert!(renderer
            .node_by_id("flag")
            .unwrap()
            .element
            .attributes
            .contains_key("checked"));
        assert!(renderer.activate(&target).unwrap());
        assert!(!renderer
            .node_by_id("flag")
            .unwrap()
            .element
            .attributes
            .contains_key("checked"));
    }

    #[test]
    fn matches_interaction_pseudo_classes() {
        let document = ugr_html::parse_document(
            r#"<button class="primary" data-ugr-id="button">Run</button>"#,
        );
        let node = document.root.children.first().unwrap();
        let renderer = UiRenderer::from_document(&document, 200, 100).unwrap();
        let id = renderer
            .elements
            .iter()
            .find(|(_, element)| element.tag == "button")
            .map(|(id, _)| *id)
            .unwrap();
        assert!(matches_state_selector(
            node,
            id,
            &renderer.taffy,
            "button.primary:hover",
            Some(id),
            None,
            None,
        ));
        assert!(matches_state_selector(
            node,
            id,
            &renderer.taffy,
            "button.primary:active",
            None,
            Some(id),
            None,
        ));
        assert!(!matches_state_selector(
            node,
            id,
            &renderer.taffy,
            "button.primary:hover",
            None,
            None,
            None,
        ));
    }
}
