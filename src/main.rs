use cosmic::{
    app::{Command, Core, Settings},
    executor,
    iced::{
        advanced::graphics::text::{self, cosmic_text},
        alignment::{Horizontal, Vertical},
        keyboard::{self, key::Named, Key},
        mouse::{self, Cursor},
        widget::{
            canvas::{
                self,
                event::Status,
                path::lyon_path::geom::euclid::{Transform2D, UnknownUnit, Vector2D},
            },
            text::{LineHeight, Shaping},
        },
        Color, Font, Length, Pixels, Point, Rectangle, Size, Vector,
    },
    iced_renderer,
    widget::{self, nav_bar::Model},
    Application, Element, Renderer, Theme,
};
use lopdf::{Document, Object, ObjectId};
use std::{env, mem};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let path = env::args().nth(1).unwrap();
    let doc = Document::load(path).unwrap();

    /*
    println!("{:#?}", doc.get_toc());
    for page_id in doc.page_iter() {
        println!("page {:?}", page_id);
        match doc.get_and_decode_page_content(page_id) {
            Ok(content) => {
                println!("{:#?}", content);
            }
            Err(err) => {
                eprintln!("failed to decode page {:?} content: {}", page_id, err);
            }
        }
        //TODO: show more pages
        break;
    }
    */

    cosmic::app::run::<App>(Settings::default(), Flags { doc })?;
    Ok(())
}

type Transform = Transform2D<f32, UnknownUnit, UnknownUnit>;

struct Flags {
    doc: Document,
}

#[derive(Clone, Debug)]
enum Message {
    CanvasClearCache,
}

//TODO: errors
fn convert_color(color_space: &str, color: &[Object]) -> Color {
    use color_space::ToRgb;
    log::info!("convert {:?} {:?}", color_space, color);
    match color_space {
        "DeviceGray" => {
            let v = color[0].as_float().unwrap();
            Color::from_rgb(v, v, v)
        }
        "DeviceRGB" => {
            let r = color[0].as_float().unwrap();
            let g = color[1].as_float().unwrap();
            let b = color[2].as_float().unwrap();
            Color::from_rgb(r, g, b)
        }
        "DeviceCMYK" => {
            let c = color[0].as_float().unwrap();
            let m = color[1].as_float().unwrap();
            let y = color[2].as_float().unwrap();
            //TODO: why does this sometimes only have 3 components?
            let rgb = if color.len() > 3 {
                let k = color[3].as_float().unwrap();
                color_space::Cmyk::new(c.into(), m.into(), y.into(), k.into()).to_rgb()
            } else {
                color_space::Cmy::new(c.into(), m.into(), y.into()).to_rgb()
            };
            Color::from_rgb(rgb.r as f32, rgb.g as f32, rgb.b as f32)
        }
        _ => {
            log::warn!("unsupported color space {:?}", color_space);
            Color::BLACK
        }
    }
}

#[derive(Clone, Debug)]
struct GraphicsState {
    line_join_style: i64,
    line_width: f32,
    transform: Transform,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            line_join_style: 0,
            line_width: 1.0,
            transform: Transform::identity(),
        }
    }
}

fn finish_path(original: &mut canvas::path::Builder, transform: &Transform) -> canvas::Path {
    let mut builder = canvas::path::Builder::default();
    mem::swap(original, &mut builder);
    builder.build().transform(transform)
}

#[derive(Clone, Debug)]
struct TextState {
    x_line: f32,
    x_off: f32,
    y_line: f32,
    y_off: f32,
    size: f32,
    leading: f32,
    mode: i64,
    transform: Transform,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            x_line: 0.0,
            x_off: 0.0,
            y_line: 0.0,
            y_off: 0.0,
            size: 0.0,
            leading: 0.0,
            mode: 0,
            transform: Transform::identity(),
        }
    }
}

struct CanvasState {
    scale: Vector,
    translate: Vector,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            scale: Vector::new(1.0, -1.0),
            translate: Vector::new(0.0, 0.0),
        }
    }
}

struct App {
    core: Core,
    flags: Flags,
    canvas_cache: canvas::Cache,
    nav_model: Model,
}

impl canvas::Program<Message, Theme, Renderer> for App {
    type State = CanvasState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> (Status, Option<Message>) {
        match event {
            canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                location,
                modifiers,
                text,
            }) => {
                match key {
                    Key::Named(Named::Home) => {
                        *state = CanvasState::default();
                    }
                    Key::Named(Named::ArrowUp) => {
                        state.translate.y += 16.0;
                    }
                    Key::Named(Named::ArrowDown) => {
                        state.translate.y -= 16.0;
                    }
                    Key::Named(Named::ArrowLeft) => {
                        state.translate.x += 16.0;
                    }
                    Key::Named(Named::ArrowRight) => {
                        state.translate.x -= 16.0;
                    }
                    Key::Named(Named::PageUp) => {
                        state.scale.x *= 1.1;
                        state.scale.y = -state.scale.x;
                    }
                    Key::Named(Named::PageDown) => {
                        state.scale.x /= 1.1;
                        state.scale.y = -state.scale.x;
                    }
                    _ => return (Status::Ignored, None),
                }
                (Status::Captured, Some(Message::CanvasClearCache))
            }
            _ => (Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<iced_renderer::Geometry> {
        let geo = self.canvas_cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::new(0.0, 0.0), frame.size(), Color::WHITE);
            frame.translate(Vector::new(0.0, frame.size().height));
            frame.scale_nonuniform(state.scale);
            frame.translate(state.translate);

            if let Some(page_id) = self.nav_model.active_data::<ObjectId>() {
                let doc = &self.flags.doc;
                let page_dict = doc.get_object(*page_id).and_then(|obj| obj.as_dict());
                println!("{:#?}", page_dict);
                let media_box = page_dict.ok().and_then(|dict| {
                    let rect = dict.get(b"MediaBox").ok()?.as_array().ok()?;
                    Some(Rectangle::new(
                        Point::new(
                            rect.get(0)?.as_float().ok()?,
                            rect.get(1)?.as_float().ok()?
                        ),
                        Size::new(
                            rect.get(2)?.as_float().ok()?,
                            rect.get(3)?.as_float().ok()?
                        ),
                    ))
                });
                println!("{:#?}", media_box);
                let fonts = doc.get_page_fonts(*page_id);
                println!("{:#?}", fonts);
                let resources = doc.get_page_resources(*page_id);
                println!("{:#?}", resources);
                match doc.get_and_decode_page_content(*page_id) {
                    Ok(content) => {
                        let mut color_space_fill = "DeviceGray".to_string();
                        let mut color_fill = vec![Object::Real(0.0)];
                        let mut color_space_stroke = "DeviceGray".to_string();
                        let mut color_stroke = vec![Object::Real(0.0)];
                        let mut graphics_states = vec![GraphicsState::default()];
                        let mut text_states = vec![];
                        let mut p = canvas::path::Builder::new();
                        for op in content.operations.iter() {
                            //TODO: better handle errors with object conversions
                            // https://pdfa.org/wp-content/uploads/2023/08/PDF-Operators-CheatSheet.pdf
                            match op.operator.as_str() {
                                // Path construction
                                "c" => {
                                    let x1 = op.operands[0].as_float().unwrap();
                                    let y1 = op.operands[1].as_float().unwrap();
                                    let x2 = op.operands[2].as_float().unwrap();
                                    let y2 = op.operands[3].as_float().unwrap();
                                    let x3 = op.operands[4].as_float().unwrap();
                                    let y3 = op.operands[5].as_float().unwrap();
                                    log::info!(
                                        "bezier_curve_to {x1}, {y1}; {x2}, {y2}; {x3}, {y3}"
                                    );
                                    p.bezier_curve_to(
                                        Point::new(x1, y1),
                                        Point::new(x2, y2),
                                        Point::new(x3, y3),
                                    );
                                }
                                "h" => {
                                    log::info!("close");
                                    p.close();
                                }
                                "l" => {
                                    let x = op.operands[0].as_float().unwrap();
                                    let y = op.operands[1].as_float().unwrap();
                                    log::info!("line_to {x}, {y}");
                                    p.line_to(Point::new(x, y));
                                }
                                "m" => {
                                    let x = op.operands[0].as_float().unwrap();
                                    let y = op.operands[1].as_float().unwrap();
                                    log::info!("move_to {x}, {y}");
                                    p.move_to(Point::new(x, y));
                                }
                                "re" => {
                                    let x = op.operands[0].as_float().unwrap();
                                    let y = op.operands[1].as_float().unwrap();
                                    let w = op.operands[2].as_float().unwrap();
                                    let h = op.operands[3].as_float().unwrap();
                                    log::info!("rectangle {x}, {y}, {w}, {y}");
                                    p.rectangle(Point::new(x, y), Size::new(w, h));
                                }

                                // Path painting
                                "b" | "B" | "b*" | "B*" | "f" | "f*" | "n" | "s" | "S" => {
                                    let (close, fill, stroke, rule) = match op.operator.as_str() {
                                        "b" => (true, true, true, canvas::fill::Rule::NonZero),
                                        "B" => (false, true, true, canvas::fill::Rule::NonZero),
                                        "b*" => (true, true, true, canvas::fill::Rule::EvenOdd),
                                        "B*" => (false, true, true, canvas::fill::Rule::EvenOdd),
                                        "f" => (true, true, false, canvas::fill::Rule::NonZero),
                                        "f*" => (false, true, false, canvas::fill::Rule::EvenOdd),
                                        "F" => (false, true, false, canvas::fill::Rule::NonZero),
                                        "n" => (false, false, false, canvas::fill::Rule::NonZero),
                                        "s" => (true, false, true, canvas::fill::Rule::NonZero),
                                        "S" => (false, false, true, canvas::fill::Rule::NonZero),
                                        _ => panic!(
                                            "unexpected path painting operator {}",
                                            op.operator
                                        ),
                                    };
                                    log::info!(
                                        "{}{}{}end path using {:?} winding rule",
                                        if close { "close, " } else { "" },
                                        if fill { "fill, " } else { "" },
                                        if stroke { "stroke, " } else { "" },
                                        rule
                                    );
                                    if close {
                                        p.close();
                                    }
                                    let gs = graphics_states.last().unwrap();
                                    let path = finish_path(&mut p, &gs.transform);
                                    if fill {
                                        let mut f = canvas::Fill::from(convert_color(
                                            &color_space_fill,
                                            &color_fill,
                                        ));
                                        f.rule = rule;
                                        frame.fill(&path, f);
                                    }
                                    if stroke {
                                        frame.stroke(
                                            &path,
                                            canvas::Stroke::default()
                                                .with_color(convert_color(
                                                    &color_space_stroke,
                                                    &color_stroke,
                                                ))
                                                .with_line_join(match gs.line_join_style {
                                                    0 => canvas::LineJoin::Miter,
                                                    1 => canvas::LineJoin::Round,
                                                    2 => canvas::LineJoin::Bevel,
                                                    _ => canvas::LineJoin::default(),
                                                }),
                                        );
                                    }
                                }

                                // Text object
                                "BT" => {
                                    text_states.push(TextState::default());
                                }
                                "ET" => {
                                    text_states.pop();
                                }

                                // Text state
                                "Tf" => {
                                    //TODO: use font name
                                    let name = op.operands[0].as_name_str();
                                    let size = op.operands[1].as_float().unwrap();
                                    log::info!("set font {name:?} size {size}");
                                    let ts = text_states.last_mut().unwrap();
                                    ts.size = size;
                                }
                                "TL" => {
                                    let leading = op.operands[0].as_float().unwrap();
                                    log::info!("set text leading {leading}");
                                    let ts = text_states.last_mut().unwrap();
                                    ts.leading = leading;
                                }
                                "Ts" => {
                                    let rise = op.operands[0].as_float().unwrap();
                                    log::info!("set text rise {rise}");
                                    let ts = text_states.last_mut().unwrap();
                                    ts.y_off = rise;
                                }

                                // Text positioning
                                "T*" => {
                                    log::info!("move to start of next line");
                                    let ts = text_states.last_mut().unwrap();
                                    ts.x_off = 0.0;
                                    ts.y_line += ts.leading;
                                    ts.y_off = 0.0;
                                }
                                "Td" => {
                                    let x = op.operands[0].as_float().unwrap();
                                    let y = op.operands[1].as_float().unwrap();
                                    log::info!("move to start of next line {x}, {y}");
                                    let ts = text_states.last_mut().unwrap();
                                    ts.x_line += x;
                                    ts.x_off = 0.0;
                                    ts.y_line -= y;
                                    ts.y_off = 0.0;
                                }
                                "TD" => {
                                    let x = op.operands[0].as_float().unwrap();
                                    let y = op.operands[1].as_float().unwrap();
                                    log::info!(
                                        "move to start of next line {x}, {y} and set leading"
                                    );
                                    let ts = text_states.last_mut().unwrap();
                                    ts.x_line += x;
                                    ts.x_off = 0.0;
                                    ts.y_line -= y;
                                    ts.y_off = 0.0;
                                    ts.leading = -y;
                                }
                                "Tm" => {
                                    let a = op.operands[0].as_float().unwrap();
                                    let b = op.operands[1].as_float().unwrap();
                                    let c = op.operands[2].as_float().unwrap();
                                    let d = op.operands[3].as_float().unwrap();
                                    let e = op.operands[4].as_float().unwrap();
                                    let f = op.operands[5].as_float().unwrap();
                                    let ts = text_states.last_mut().unwrap();
                                    ts.transform = Transform::new(a, b, c, d, e, f);
                                    log::info!("set text transform {:?}", ts.transform);
                                }

                                // Text showing
                                "Tj" | "TJ" => {
                                    let has_adjustment = match op.operator.as_str() {
                                        "Tj" => false,
                                        "TJ" => true,
                                        _ => panic!(
                                            "uexpected text showing operator {}",
                                            op.operator
                                        ),
                                    };
                                    log::info!(
                                        "show text{} {:?}",
                                        if has_adjustment {
                                            " with adjustment"
                                        } else {
                                            ""
                                        },
                                        op.operands
                                    );
                                    //TODO: clean this up
                                    let elements = if has_adjustment {
                                        op.operands[0].as_array().unwrap()
                                    } else {
                                        &op.operands
                                    };
                                    let mut i = 0;
                                    while i < elements.len() {
                                        let content = elements[i].as_string().unwrap();
                                        i += 1;
                                        let adjustment = if has_adjustment && i < elements.len() {
                                            let adjustment = elements[i].as_float().unwrap();
                                            i += 1;
                                            adjustment
                                        } else {
                                            0.0
                                        };
                                        //TODO: fill or stroke?
                                        let stroke = false;
                                        //TODO: set all of these parameters
                                        let mut ts = text_states.last_mut().unwrap();
                                        let text = canvas::Text {
                                            content: content.to_string(),
                                            position: Point::new(
                                                ts.x_line + ts.x_off,
                                                ts.y_line + ts.y_off - ts.size,
                                            ),
                                            color: if stroke {
                                                convert_color(&color_space_stroke, &color_stroke)
                                            } else {
                                                convert_color(&color_space_fill, &color_fill)
                                            },
                                            size: Pixels(ts.size),
                                            line_height: LineHeight::Absolute(Pixels(ts.leading)),
                                            font: Font::DEFAULT,
                                            horizontal_alignment: Horizontal::Left,
                                            vertical_alignment: Vertical::Top,
                                            shaping: Shaping::Advanced,
                                        };
                                        text.draw_with(|mut path, color| {
                                            path = path
                                                .transform(&Transform::scale(1.0, -1.0))
                                                .transform(&ts.transform);
                                            if stroke {
                                                frame.stroke(
                                                    &path,
                                                    canvas::Stroke::default().with_color(color),
                                                );
                                            } else {
                                                frame.fill(&path, color);
                                            }
                                        });
                                        //TODO: more efficient way to determine size
                                        {
                                            let mut font_system = text::font_system()
                                                .write()
                                                .expect("Write font system");

                                            let mut buffer = cosmic_text::BufferLine::new(
                                                &text.content,
                                                cosmic_text::LineEnding::default(),
                                                cosmic_text::AttrsList::new(text::to_attributes(
                                                    text.font,
                                                )),
                                                text::to_shaping(text.shaping),
                                            );

                                            let layout = buffer.layout(
                                                font_system.raw(),
                                                text.size.0,
                                                f32::MAX,
                                                cosmic_text::Wrap::None,
                                                None,
                                            );

                                            let mut max_w = 0.0;
                                            for layout_line in layout {
                                                if layout_line.w > max_w {
                                                    max_w = layout_line.w;
                                                }
                                            }
                                            ts.x_off += max_w;
                                            //TODO: why does adjustment need to be inverse transformed?
                                            match ts.transform.inverse().map(|x| {
                                                x.transform_vector(Vector2D::new(adjustment, 0.0))
                                            }) {
                                                Some(v) => {
                                                    //TODO: v.y?
                                                    log::info!(
                                                        "line {} off {} adj {} trans {} max_w {} content {:?}",
                                                        ts.x_line,
                                                        ts.x_off,
                                                        adjustment,
                                                        v.x,
                                                        max_w,
                                                        content,
                                                    );
                                                    //ts.x_off -= v.x;
                                                }
                                                None => {
                                                    //TODO: is this a problem?
                                                }
                                            }
                                        }
                                    }
                                }

                                // Graphics state
                                "cm" => {
                                    let a = op.operands[0].as_float().unwrap();
                                    let b = op.operands[1].as_float().unwrap();
                                    let c = op.operands[2].as_float().unwrap();
                                    let d = op.operands[3].as_float().unwrap();
                                    let e = op.operands[4].as_float().unwrap();
                                    let f = op.operands[5].as_float().unwrap();
                                    let gs = graphics_states.last_mut().unwrap();
                                    gs.transform = Transform::new(a, b, c, d, e, f);
                                    log::info!("set graphics transform {:?}", gs.transform);
                                }
                                "j" => {
                                    let gs = graphics_states.last_mut().unwrap();
                                    gs.line_join_style = op.operands[0].as_i64().unwrap();
                                    log::info!("set line join style {}", gs.line_join_style);
                                }
                                "q" => {
                                    log::info!("save graphics state");
                                    let gs = graphics_states.last().cloned().unwrap_or_default();
                                    graphics_states.push(gs);
                                }
                                "Q" => {
                                    log::info!("restore graphics state");
                                    graphics_states.pop();
                                }
                                "w" => {
                                    let gs = graphics_states.last_mut().unwrap();
                                    gs.line_width = op.operands[0].as_float().unwrap();
                                    log::info!("set line width {}", gs.line_width);
                                }

                                // Color
                                "cs" => {
                                    color_space_fill =
                                        op.operands[0].as_name_str().unwrap().to_string();
                                    log::info!("color space (fill) {color_space_fill}");
                                }
                                "CS" => {
                                    color_space_stroke =
                                        op.operands[0].as_name_str().unwrap().to_string();
                                    log::info!("color space (stroke) {color_space_stroke}");
                                }
                                "g" => {
                                    color_space_fill = "DeviceGray".to_string();
                                    color_fill = op.operands.clone();
                                    log::info!("color (fill) {color_fill:?}");
                                }
                                "G" => {
                                    color_space_stroke = "DeviceGray".to_string();
                                    color_stroke = op.operands.clone();
                                    log::info!("color (stroke) {color_stroke:?}");
                                }
                                "k" => {
                                    color_space_fill = "DeviceCMYK".to_string();
                                    color_fill = op.operands.clone();
                                    log::info!("color (fill) {color_fill:?}");
                                }
                                "K" => {
                                    color_space_stroke = "DeviceCMYK".to_string();
                                    color_stroke = op.operands.clone();
                                    log::info!("color (stroke) {color_stroke:?}");
                                }
                                "rg" => {
                                    color_space_fill = "DeviceRGB".to_string();
                                    color_fill = op.operands.clone();
                                    log::info!("color (fill) {color_fill:?}");
                                }
                                "RG" => {
                                    color_space_stroke = "DeviceRGB".to_string();
                                    color_stroke = op.operands.clone();
                                    log::info!("color (stroke) {color_stroke:?}");
                                }
                                "scn" => {
                                    color_fill = op.operands.clone();
                                    log::info!("color (fill) {color_fill:?}");
                                }
                                "SCN" => {
                                    color_stroke = op.operands.clone();
                                    log::info!("color (stroke) {color_stroke:?}");
                                }
                                _ => {
                                    log::warn!("unknown op {:?}", op);
                                }
                            }
                        }
                    }
                    Err(_err) => {}
                }
            }
        });
        vec![geo]
    }
}

impl Application for App {
    type Executor = executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = "com.system76.CosmicReader";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Command<Message>) {
        let mut nav_model = Model::default();
        for (i, page_id) in flags.doc.page_iter().enumerate() {
            nav_model
                .insert()
                .text(format!("Page {}", i + 1))
                .data::<ObjectId>(page_id);
        }
        nav_model.activate_position(0);

        (
            Self {
                core,
                flags,
                canvas_cache: canvas::Cache::new(),
                nav_model,
            },
            Command::none(),
        )
    }

    fn nav_model(&self) -> Option<&Model> {
        Some(&self.nav_model)
    }

    fn on_nav_select(&mut self, id: widget::nav_bar::Id) -> Command<Message> {
        self.canvas_cache.clear();
        self.nav_model.activate(id);
        Command::none()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::CanvasClearCache => {
                self.canvas_cache.clear();
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<Message> {
        canvas::Canvas::new(self)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
