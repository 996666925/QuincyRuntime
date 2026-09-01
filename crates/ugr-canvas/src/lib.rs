use skia_safe::{surfaces, Color, Paint, PaintStyle, PathBuilder, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum CanvasCommand {
    FillRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    },
    ClearRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    BeginPath,
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    Stroke {
        color: [f32; 4],
        width: f32,
    },
    Fill {
        color: [f32; 4],
    },
    Save,
    Restore,
    Transform([f32; 6]),
}

#[derive(Debug, Clone)]
struct CanvasStateSnapshot {
    fill_style: [f32; 4],
    stroke_style: [f32; 4],
    line_width: f32,
    global_alpha: f32,
    transform: [f32; 6],
}

#[derive(Debug, Clone)]
pub struct Canvas2DState {
    pub width: u32,
    pub height: u32,
    pub fill_style: [f32; 4],
    pub stroke_style: [f32; 4],
    pub line_width: f32,
    pub global_alpha: f32,
    pub transform: [f32; 6],
    pub commands: Vec<CanvasCommand>,
    surface: skia_safe::Surface,
    saved: Vec<CanvasStateSnapshot>,
}

impl Canvas2DState {
    pub fn new(width: u32, height: u32) -> Self {
        let surface = surfaces::raster_n32_premul((width as i32, height as i32))
            .expect("failed to create Skia raster surface");
        Self {
            width,
            height,
            fill_style: [0.0, 0.0, 0.0, 1.0],
            stroke_style: [0.0, 0.0, 0.0, 1.0],
            line_width: 1.0,
            global_alpha: 1.0,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            commands: Vec::new(),
            surface,
            saved: Vec::new(),
        }
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32) {
        let mut paint = Paint::default();
        paint.set_color(self.to_skia_color(self.fill_style));
        paint.set_anti_alias(true);
        self.surface
            .canvas()
            .draw_rect(Rect::from_xywh(x, y, width, height), &paint);
        self.commands.push(CanvasCommand::FillRect {
            x,
            y,
            width,
            height,
            color: self.color(self.fill_style),
        });
    }

    pub fn clear_rect(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.surface.canvas().save();
        self.surface
            .canvas()
            .clip_rect(Rect::from_xywh(x, y, width, height), None, None);
        self.surface.canvas().clear(Color::TRANSPARENT);
        self.surface.canvas().restore();
        self.commands.push(CanvasCommand::ClearRect {
            x,
            y,
            width,
            height,
        });
    }

    pub fn begin_path(&mut self) {
        self.commands.push(CanvasCommand::BeginPath);
    }
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(CanvasCommand::MoveTo { x, y });
    }
    pub fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(CanvasCommand::LineTo { x, y });
    }
    pub fn stroke(&mut self) {
        let mut path_builder = PathBuilder::new();
        let mut current = None;
        for command in &self.commands {
            match command {
                CanvasCommand::MoveTo { x, y } => {
                    path_builder.move_to((*x, *y));
                    current = Some(());
                }
                CanvasCommand::LineTo { x, y } if current.is_some() => {
                    path_builder.line_to((*x, *y));
                }
                _ => {}
            }
        }
        let path = path_builder.detach();
        let mut paint = Paint::default();
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(self.line_width);
        paint.set_color(self.to_skia_color(self.stroke_style));
        self.surface.canvas().draw_path(&path, &paint);
        self.commands.push(CanvasCommand::Stroke {
            color: self.color(self.stroke_style),
            width: self.line_width,
        });
    }
    pub fn fill(&mut self) {
        self.commands.push(CanvasCommand::Fill {
            color: self.color(self.fill_style),
        });
    }
    pub fn save(&mut self) {
        self.surface.canvas().save();
        self.saved.push(CanvasStateSnapshot {
            fill_style: self.fill_style,
            stroke_style: self.stroke_style,
            line_width: self.line_width,
            global_alpha: self.global_alpha,
            transform: self.transform,
        });
        self.commands.push(CanvasCommand::Save);
    }
    pub fn restore(&mut self) {
        self.surface.canvas().restore();
        if let Some(snapshot) = self.saved.pop() {
            self.fill_style = snapshot.fill_style;
            self.stroke_style = snapshot.stroke_style;
            self.line_width = snapshot.line_width;
            self.global_alpha = snapshot.global_alpha;
            self.transform = snapshot.transform;
        }
        self.commands.push(CanvasCommand::Restore);
    }
    pub fn set_transform(&mut self, transform: [f32; 6]) {
        self.transform = transform;
        self.commands.push(CanvasCommand::Transform(transform));
    }

    fn color(&self, mut color: [f32; 4]) -> [f32; 4] {
        color[3] *= self.global_alpha;
        color
    }

    fn to_skia_color(&self, color: [f32; 4]) -> Color {
        Color::from_argb(
            (color[3].clamp(0.0, 1.0) * 255.0) as u8,
            (color[0].clamp(0.0, 1.0) * 255.0) as u8,
            (color[1].clamp(0.0, 1.0) * 255.0) as u8,
            (color[2].clamp(0.0, 1.0) * 255.0) as u8,
        )
    }
}

impl Default for Canvas2DState {
    fn default() -> Self {
        Self::new(1, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_stateful_drawing_commands() {
        let mut canvas = Canvas2DState::new(320, 200);
        canvas.fill_style = [1.0, 0.0, 0.0, 1.0];
        canvas.global_alpha = 0.5;
        canvas.fill_rect(1.0, 2.0, 30.0, 40.0);
        assert_eq!(
            canvas.commands,
            vec![CanvasCommand::FillRect {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 40.0,
                color: [1.0, 0.0, 0.0, 0.5]
            }]
        );
    }
}
