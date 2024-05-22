use cosmic::{
    app::{Command, Core, Settings},
    executor,
    iced::{
        keyboard::{self, key::Named, Key},
        mouse,
        mouse::Cursor,
        widget::canvas::{self, event::Status},
        Color, Length, Point, Rectangle, Size, Vector,
    },
    iced_renderer,
    widget::{self, nav_bar::Model},
    Application, Element, Renderer, Theme,
};
use lopdf::{Document, ObjectId};
use std::{collections::HashMap, env, sync::Mutex};

mod pdf;
mod text;

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

struct Flags {
    doc: Document,
}

#[derive(Clone, Debug)]
enum Message {
    CanvasClearCache,
}

struct App {
    core: Core,
    flags: Flags,
    canvas_cache: canvas::Cache,
    nav_model: Model,
    page_cache: Mutex<HashMap<ObjectId, Vec<pdf::PageOp>>>,
}

impl canvas::Program<Message, Theme, Renderer> for App {
    type State = pdf::CanvasState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: Cursor,
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
                        *state = pdf::CanvasState::default();
                        state.modifiers = modifiers;
                    }
                    Key::Named(Named::ArrowUp) => {
                        state.translate.y -= 16.0;
                    }
                    Key::Named(Named::ArrowDown) => {
                        state.translate.y += 16.0;
                    }
                    Key::Named(Named::ArrowLeft) => {
                        state.translate.x += 16.0;
                    }
                    Key::Named(Named::ArrowRight) => {
                        state.translate.x -= 16.0;
                    }
                    Key::Named(Named::PageUp) => {
                        state.scale *= 1.1;
                    }
                    Key::Named(Named::PageDown) => {
                        state.scale /= 1.1;
                    }
                    _ => return (Status::Ignored, None),
                }
                (Status::Captured, Some(Message::CanvasClearCache))
            }
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = modifiers;
                (Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let (x, y) = match delta {
                        mouse::ScrollDelta::Lines { x, y } => {
                            //TODO: best value to translate line scroll to pixels
                            (x * 16.0, y * 16.0)
                        }
                        mouse::ScrollDelta::Pixels { x, y } => (x, y),
                    };
                    if state.modifiers.contains(keyboard::Modifiers::CTRL) {
                        state.scale *= 1.1f32.powf(y / 16.0);
                    } else {
                        state.translate.x += x;
                        state.translate.y -= y;
                    }
                    (Status::Captured, Some(Message::CanvasClearCache))
                } else {
                    (Status::Ignored, None)
                }
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
            if let Some(&page_id) = self.nav_model.active_data::<ObjectId>() {
                let doc = &self.flags.doc;
                let page_dict = doc.get_object(page_id).and_then(|obj| obj.as_dict());
                println!("{:#?}", page_dict);
                let media_box = page_dict.ok().and_then(|dict| {
                    let rect = dict.get(b"MediaBox").ok()?.as_array().ok()?;
                    Some(Rectangle::new(
                        Point::new(rect.get(0)?.as_float().ok()?, rect.get(1)?.as_float().ok()?),
                        Size::new(rect.get(2)?.as_float().ok()?, rect.get(3)?.as_float().ok()?),
                    ))
                });
                println!("{:#?}", media_box);

                // PDF's origin is the bottom left while the canvas origin is the top right, so flip it
                {
                    frame.translate(Vector::new(0.0, frame.size().height));
                    frame.scale_nonuniform(Vector::new(1.0, -1.0));
                }

                // Apply zoom and pan
                //TODO: can user's pan and zoom be applied without having to regenerate entire frame?
                {
                    // Move to center
                    frame.translate(Vector::new(
                        frame.size().width / 2.0,
                        frame.size().height / 2.0,
                    ));
                    // Zoom
                    frame.scale(state.scale);
                    // Apply pan
                    frame.translate(state.translate);
                }
                if let Some(rect) = media_box {
                    // Move back to origin
                    frame.translate(Vector::new(
                        -rect.size().width / 2.0,
                        -rect.size().height / 2.0,
                    ));
                    // Fill background
                    frame.fill_rectangle(rect.position(), rect.size(), Color::WHITE);
                }

                {
                    let mut page_cache = self.page_cache.lock().unwrap();
                    let ops = page_cache
                        .entry(page_id)
                        .or_insert_with(|| pdf::page_ops(doc, page_id));
                    for op in ops.iter() {
                        if let Some(fill) = &op.fill {
                            frame.fill(&op.path, fill.clone());
                        }
                        if let Some(stroke) = &op.stroke {
                            frame.stroke(&op.path, stroke.clone());
                        }
                    }
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
                page_cache: Mutex::new(HashMap::new()),
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
