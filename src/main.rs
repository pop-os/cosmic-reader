use cosmic::{
    app::{Command, Core, Settings},
    executor,
    iced::{
        keyboard::{self, key::Named, Key},
        mouse,
        mouse::Cursor,
        widget::canvas::{self, event::Status},
        Length, Rectangle,
    },
    iced_renderer,
    widget::{self, nav_bar::Model},
    Application, Element, Renderer, Theme,
};
use lopdf::{Document, ObjectId};
use std::env;

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
                        state.scale.x *= 1.1f32.powf(y / 16.0);
                        state.scale.y = -state.scale.x;
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
            if let Some(page_id) = self.nav_model.active_data::<ObjectId>() {
                match pdf::draw_page(&self.flags.doc, *page_id, frame, state) {
                    Ok(()) => {}
                    Err(err) => {
                        log::error!("failed to draw page {:?}: {}", page_id, err);
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
