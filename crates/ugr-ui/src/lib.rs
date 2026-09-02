//! Native HTML layout and drawing backend.
//!
//! The DOM remains owned by `ugr-html`; this crate turns that document into a
//! Taffy layout tree and paints the resulting boxes with the workspace Skia
//! dependency. It deliberately exposes no JavaScript-specific state.

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroU32;
use std::sync::Arc;

use skia_safe::{
    surfaces, Canvas, Color, Font, FontMgr, FontStyle, Paint, PaintStyle, Rect as SkiaRect,
    Typeface,
};
use taffy::prelude::*;
use ugr_html::{CssRule, HtmlDocument, HtmlNode};
use winit::event_loop::{ActiveEventLoop, OwnedDisplayHandle};
use winit::window::Window;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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
}

/// Winit presenter for the Skia raster surface. Layout and painting remain in
/// `UiRenderer`; this adapter only copies pixels to a native window.
pub struct UiWindowRenderer {
    title: String,
    width: u32,
    height: u32,
    document: HtmlDocument,
    ui: UiRenderer,
    context: softbuffer::Context<OwnedDisplayHandle>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
}

impl UiWindowRenderer {
    pub fn new(
        document: &HtmlDocument,
        title: String,
        width: u32,
        height: u32,
        display: OwnedDisplayHandle,
    ) -> Result<Self, String> {
        Ok(Self {
            title,
            width,
            height,
            document: document.clone(),
            ui: UiRenderer::from_document(document, width, height)?,
            context: softbuffer::Context::new(display)
                .map_err(|e| format!("failed to create UI display context: {e}"))?,
            window: None,
            surface: None,
        })
    }

    pub fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if self.surface.is_some() {
            return Ok(());
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.title.clone())
                        .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height)),
                )
                .map_err(|e| format!("failed to create UI window: {e}"))?,
        );
        let surface = softbuffer::Surface::new(&self.context, window.clone())
            .map_err(|e| format!("failed to create UI surface: {e}"))?;
        self.window = Some(window);
        self.surface = Some(surface);
        self.ui.draw();
        self.present();
        Ok(())
    }

    pub fn draw(&mut self) {
        self.ui.draw();
        self.present();
    }

    fn present(&mut self) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let Some(width) = NonZeroU32::new(self.width.max(1)) else {
            return;
        };
        let Some(height) = NonZeroU32::new(self.height.max(1)) else {
            return;
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let pixels = self.ui.rgba_pixels();
        let mut buffer = match surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(_) => return,
        };
        let dst_width = buffer.width().get() as usize;
        let dst_height = buffer.height().get() as usize;
        let src_width = self.ui.pixel_width.max(1) as usize;
        let src_height = self.ui.pixel_height.max(1) as usize;
        for y in 0..dst_height {
            let sy = y.saturating_mul(src_height) / dst_height;
            for x in 0..dst_width {
                let sx = x.saturating_mul(src_width) / dst_width;
                let src = (sy * src_width + sx) * 4;
                let dst = y * dst_width + x;
                if src + 2 >= pixels.len() || dst >= buffer.len() {
                    continue;
                }
                buffer[dst] = (u32::from(pixels[src]) << 16)
                    | (u32::from(pixels[src + 1]) << 8)
                    | u32::from(pixels[src + 2]);
            }
        }
        let _ = buffer.present();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if let Ok(ui) = UiRenderer::from_document(&self.document, width.max(1), height.max(1)) {
            self.ui = ui;
        }
    }
    pub fn is_initialized(&self) -> bool {
        self.surface.is_some()
    }
    pub fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
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
        let scale_factor = scale_factor.max(1.0);
        let pixel_width = ((width.max(1) as f32 * scale_factor).round() as u32).max(1);
        let pixel_height = ((height.max(1) as f32 * scale_factor).round() as u32).max(1);
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
        let typeface = FontMgr::new()
            .legacy_make_typeface(None, FontStyle::normal())
            .ok_or_else(|| "failed to load the system default typeface".to_owned())?;
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
        })
    }

    pub fn root(&self) -> NodeId {
        self.root
    }
    pub fn layout(&self, node: NodeId) -> Option<UiRect> {
        self.layout.get(&node).copied()
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
            &self.layout,
            &self.taffy,
            Color::BLACK,
            &self.typeface,
        );
        canvas.restore();
    }

    /// Rebuild layout and the raster surface for a new physical window size.
    /// Keeping the source document here avoids stretching a logical-size
    /// surface into a differently-sized softbuffer after a DPI/resize event.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        let resized = Self::from_document_scaled(&self.document, width.max(1), height.max(1), 1.0)?;
        *self = resized;
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
        *self = resized;
        Ok(())
    }

    pub fn snapshot(&self) -> &skia_safe::Surface {
        &self.surface
    }

    /// Copy the current Skia raster surface to tightly packed RGBA bytes.
    pub fn rgba_pixels(&mut self) -> Vec<u8> {
        let Some(pixmap) = self.surface.peek_pixels() else {
            return Vec::new();
        };
        let mut pixels = Vec::with_capacity((self.pixel_width * self.pixel_height * 4) as usize);
        for y in 0..self.pixel_height as i32 {
            for x in 0..self.pixel_width as i32 {
                let color = pixmap.get_color((x, y));
                pixels.extend_from_slice(&[color.r(), color.g(), color.b(), color.a()]);
            }
        }
        pixels
    }
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
        "#text" | "head" | "title" | "meta" | "link" | "style" | "script"
    )
}

fn computed_style(node: &HtmlNode, rules: &[CssRule]) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
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
    if let Some(values) = declarations.get("padding").and_then(|v| box_values(v)) {
        style.padding = taffy::Rect {
            top: LengthPercentage::length(values[0]),
            right: LengthPercentage::length(values[1]),
            bottom: LengthPercentage::length(values[2]),
            left: LengthPercentage::length(values[3]),
        };
    }
    if let Some(values) = declarations.get("margin").and_then(|v| box_values(v)) {
        style.margin = taffy::Rect {
            top: LengthPercentageAuto::length(values[0]),
            right: LengthPercentageAuto::length(values[1]),
            bottom: LengthPercentageAuto::length(values[2]),
            left: LengthPercentageAuto::length(values[3]),
        };
    }
    style
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
    layout: &HashMap<NodeId, UiRect>,
    taffy: &TaffyTree<()>,
    inherited_color: Color,
    typeface: &Typeface,
) {
    let Some(rect) = layout.get(&node) else {
        return;
    };
    let declarations = styles.get(&node);
    if declarations
        .and_then(|d| d.get("display"))
        .is_some_and(|v| v.eq_ignore_ascii_case("none"))
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
    if let Some(element) = elements.get(&node) {
        if declarations
            .and_then(|d| d.get("background-color"))
            .is_none()
            && declarations.and_then(|d| d.get("background")).is_none()
            && matches!(element.tag.as_str(), "input" | "button")
        {
            let mut paint = Paint::default();
            paint.set_color(if element.tag == "button" {
                Color::from_rgb(225, 228, 232)
            } else {
                Color::from_rgb(248, 249, 250)
            });
            canvas.draw_rect(
                SkiaRect::from_xywh(rect.x, rect.y, rect.width, rect.height),
                &paint,
            );
        }
        let text = if element.tag == "input" {
            element
                .attributes
                .get("value")
                .or_else(|| element.attributes.get("placeholder"))
                .map(String::as_str)
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
            let text_x = if declarations
                .and_then(|d| d.get("text-align"))
                .is_some_and(|v| v.eq_ignore_ascii_case("right"))
            {
                rect.x + (rect.width - font.measure_str(text, Some(&paint)).0 - 6.0).max(6.0)
            } else {
                rect.x + 6.0
            };
            canvas.draw_str(text, (text_x, rect.y + font.size()), &font, &paint);
        }
        if declarations.is_some_and(|d| d.contains_key("border"))
            || matches!(element.tag.as_str(), "input" | "button")
        {
            let mut paint = Paint::default();
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(1.0);
            paint.set_color(Color::from_rgb(80, 80, 80));
            canvas.draw_rect(
                SkiaRect::from_xywh(rect.x, rect.y, rect.width.max(1.0), rect.height.max(1.0)),
                &paint,
            );
        }
    }
    for child in taffy.child_ids(node) {
        draw_node(
            canvas, child, elements, styles, layout, taffy, text_color, typeface,
        );
    }
}

fn parse_declarations(value: &str) -> BTreeMap<String, String> {
    value
        .split(';')
        .filter_map(|part| part.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect()
}
fn px(value: &str) -> Option<f32> {
    value
        .trim()
        .strip_suffix("px")
        .unwrap_or(value.trim())
        .parse()
        .ok()
}
fn box_values(value: &str) -> Option<[f32; 4]> {
    let values = value
        .split_whitespace()
        .map(px)
        .collect::<Option<Vec<_>>>()?;
    match values.as_slice() {
        [all] => Some([*all, *all, *all, *all]),
        [vertical, horizontal] => Some([*vertical, *horizontal, *vertical, *horizontal]),
        [top, horizontal, bottom] => Some([*top, *horizontal, *bottom, *horizontal]),
        [top, right, bottom, left] => Some([*top, *right, *bottom, *left]),
        _ => None,
    }
}
fn dimension(value: &str) -> Option<Dimension> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .parse::<f32>()
            .ok()
            .map(|v| Dimension::percent(v / 100.0));
    }
    px(value).map(Dimension::length)
}
fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("white") {
        return Some(Color::WHITE);
    }
    if value.eq_ignore_ascii_case("black") {
        return Some(Color::BLACK);
    }
    let value = value.strip_prefix('#')?;
    let value = if value.len() == 3 {
        return Some(Color::from_rgb(
            u8::from_str_radix(&value[0..1].repeat(2), 16).ok()?,
            u8::from_str_radix(&value[1..2].repeat(2), 16).ok()?,
            u8::from_str_radix(&value[2..3].repeat(2), 16).ok()?,
        ));
    } else if value.len() == 6 {
        value
    } else {
        return None;
    };
    Some(Color::from_rgb(
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
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
            r#"<html><body><div style="width:200px;height:200px;background-color:black;color:white">123</div></body></html>"#,
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
}
