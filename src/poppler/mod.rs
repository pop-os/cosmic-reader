use cosmic::{
    app::{Core, Settings, Task},
    executor,
    iced::{widget::scrollable, ContentFit, Length},
    widget::{self, nav_bar::Model},
    Application, Element,
};
use std::{env, fs, io};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let arg = env::args().nth(1).unwrap();
    let url = match url::Url::parse(&arg) {
        Ok(url) => Ok(url),
        Err(_) => match fs::canonicalize(&arg) {
            Ok(path) => {
                match url::Url::from_file_path(&path)
                    .or_else(|_| url::Url::from_directory_path(&path))
                {
                    Ok(url) => Ok(url),
                    Err(()) => {
                        log::warn!("failed to parse path {:?}", path);
                        Err(io::Error::other("Invalid URL and path"))
                    }
                }
            }
            Err(err) => {
                log::warn!("failed to parse argument {:?}: {}", arg, err);
                Err(err)
            }
        },
    }?;

    let doc = poppler::Document::from_file(url.as_str(), None).unwrap();

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
    doc: poppler::Document,
}

#[derive(Clone, Debug)]
enum Message {}

struct App {
    core: Core,
    dpi: f64,
    flags: Flags,
    nav_model: Model,
}

impl App {
    fn update_page(&mut self) -> Task<Message> {
        let entity = self.nav_model.active();

        if self
            .nav_model
            .data::<widget::image::Handle>(entity)
            .is_some()
        {
            // Already has image cached
            return Task::none();
        }

        if self.nav_model.data::<widget::svg::Handle>(entity).is_some() {
            // Already has SVG cached
            return Task::none();
        }

        let Some(index) = self.nav_model.data::<i32>(entity) else {
            return Task::none();
        };

        let Some(page) = self.flags.doc.page(*index) else {
            return Task::none();
        };

        //TODO: return errors
        //TODO: run in background (poppler::Page can't be shared with threads?)
        let svg = true;
        if svg {
            let mut data = Vec::new();
            {
                let surface = unsafe {
                    cairo::SvgSurface::for_raw_stream(page.size().0, page.size().1, &mut data)
                }
                .unwrap();
                let ctx = cairo::Context::new(surface).unwrap();
                page.render(&ctx);
            }
            let handle = widget::svg::Handle::from_memory(data);
            self.nav_model
                .data_set::<widget::svg::Handle>(entity, handle);
        } else {
            let scale = self.dpi / 72.0;
            let width: u16 = num::cast(page.size().0 * scale).unwrap();
            let height: u16 = num::cast(page.size().1 * scale).unwrap();
            println!(
                "{}x{} => {}x{}",
                page.size().0,
                page.size().1,
                width,
                height
            );
            let mut data =
                vec![0u8; usize::from(width) * usize::from(height) * 4].into_boxed_slice();
            {
                let surface = unsafe {
                    cairo::ImageSurface::create_for_data_unsafe(
                        data.as_mut_ptr(),
                        cairo::Format::ARgb32,
                        i32::from(width),
                        i32::from(height),
                        i32::from(width) * 4,
                    )
                }
                .unwrap();
                let ctx = cairo::Context::new(surface).unwrap();
                ctx.scale(scale, scale);
                page.render(&ctx);
            }
            let handle =
                widget::image::Handle::from_rgba(u32::from(width), u32::from(height), data);
            self.nav_model
                .data_set::<widget::image::Handle>(entity, handle);
        }
        Task::none()
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

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Message>) {
        let mut nav_model = Model::default();
        for index in 0..flags.doc.n_pages() {
            let Some(page) = flags.doc.page(index) else {
                log::warn!("missing page {}", index);
                continue;
            };
            let label = page
                .label()
                .map(|x| x.to_string())
                .unwrap_or_else(|| format!("Page {}", index + 1));
            nav_model.insert().text(label).data::<i32>(index);
        }
        nav_model.activate_position(0);

        let mut app = Self {
            core,
            //TODO: what is the best value to use?
            dpi: 192.0,
            flags,
            nav_model,
        };
        let task = app.update_page();
        (app, task)
    }

    fn nav_model(&self) -> Option<&Model> {
        Some(&self.nav_model)
    }

    fn on_nav_select(&mut self, id: widget::nav_bar::Id) -> Task<Message> {
        self.nav_model.activate(id);
        self.update_page()
    }

    fn update(&mut self, _message: Message) -> Task<Message> {
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        // Handle cached images
        if let Some(handle) = self.nav_model.active_data::<widget::image::Handle>() {
            let scrollbar = scrollable::Scrollbar::default();
            return scrollable::Scrollable::with_direction(
                widget::image(handle).content_fit(ContentFit::None),
                scrollable::Direction::Both {
                    vertical: scrollbar,
                    horizontal: scrollbar,
                },
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        }

        // Handle cached SVGs
        if let Some(handle) = self.nav_model.active_data::<widget::svg::Handle>() {
            return widget::svg(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }

        widget::text("No page image").into()
    }
}
