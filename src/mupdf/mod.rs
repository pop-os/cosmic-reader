use cosmic::{
    action,
    app::{Core, Settings, Task},
    executor,
    iced::{futures::SinkExt, stream, widget::scrollable, ContentFit, Length, Subscription},
    widget::{self, nav_bar::Model, segmented_button::Entity},
    Application, Element,
};
use std::{any::TypeId, env, fs, io, sync::Arc};

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

    cosmic::app::run::<App>(Settings::default(), Flags { url })?;
    Ok(())
}

//TODO: return errors
fn display_list_to_png(display_list: &mupdf::DisplayList, scale: f32) -> Vec<u8> {
    let matrix = mupdf::Matrix::new_scale(scale, scale);
    let pixmap = display_list
        .to_pixmap(&matrix, &mupdf::Colorspace::device_rgb(), true)
        .unwrap();
    eprintln!(
        "{}x{} @ {} => {}x{}",
        display_list.bounds().width(),
        display_list.bounds().height(),
        scale,
        pixmap.width(),
        pixmap.height(),
    );
    let mut data = Vec::new();
    //TODO: store raw image data?
    pixmap.write_to(&mut data, mupdf::ImageFormat::PNG).unwrap();
    data
}

struct Flags {
    url: url::Url,
}

#[derive(Clone, Debug)]
enum Message {
    DisplayList(i32, Arc<mupdf::DisplayList>),
    Image(i32, Vec<u8>),
    NavItem(i32, String),
    Thumbnail(i32, Vec<u8>),
}

struct App {
    core: Core,
    dpi: f32,
    flags: Flags,
    nav_model: Model,
}

impl App {
    fn entity_by_index(&self, index: i32) -> Option<Entity> {
        for entity in self.nav_model.iter() {
            if self.nav_model.data::<i32>(entity) == Some(&index) {
                return Some(entity);
            }
        }
        None
    }

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

        let Some(index) = self.nav_model.data::<i32>(entity).copied() else {
            return Task::none();
        };

        let Some(display_list) = self
            .nav_model
            .data::<Arc<mupdf::DisplayList>>(entity)
            .cloned()
        else {
            return Task::none();
        };

        let dpi = self.dpi;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let scale = dpi / 72.0;
                    let data = display_list_to_png(&display_list, scale);
                    Message::Image(index, data)
                })
                .await
                .unwrap()
            },
            |x| action::app(x),
        )
    }
}

impl Application for App {
    type Executor = executor::multi::Executor;
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
        let mut app = Self {
            core,
            //TODO: what is the best value to use?
            dpi: 192.0,
            flags,
            nav_model: Model::default(),
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

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DisplayList(index, display_list) => {
                let mut tasks = Vec::with_capacity(2);
                if let Some(entity) = self.entity_by_index(index) {
                    self.nav_model
                        .data_set::<Arc<mupdf::DisplayList>>(entity, display_list.clone());
                    if entity == self.nav_model.active() {
                        tasks.push(self.update_page());
                    }
                }
                tasks.push(Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let bounds = display_list.bounds();
                            let scale = 128.0 / bounds.width().max(bounds.height());
                            let data = display_list_to_png(&display_list, scale);
                            Message::Thumbnail(index, data)
                        })
                        .await
                        .unwrap()
                    },
                    |x| action::app(x),
                ));
                return Task::batch(tasks);
            }
            Message::Image(index, data) => {
                if let Some(entity) = self.entity_by_index(index) {
                    let handle = widget::image::Handle::from_bytes(data);
                    self.nav_model
                        .data_set::<widget::image::Handle>(entity, handle);
                }
            }
            Message::NavItem(index, label) => {
                let activate = self.nav_model.len() == 0;
                let entity = self.nav_model.insert().data::<i32>(index).text(label);
                if activate {
                    entity.activate();
                }
            }
            Message::Thumbnail(index, data) => {
                if let Some(entity) = self.entity_by_index(index) {
                    self.nav_model.icon_set(
                        entity,
                        widget::icon(widget::icon::from_raster_bytes(data)).size(32),
                    );
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        let entity = self.nav_model.active();

        // Handle cached images
        if let Some(handle) = self.nav_model.data::<widget::image::Handle>(entity) {
            return widget::image::viewer(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }

        widget::text("Page loading...").into()
    }

    fn subscription(&self) -> Subscription<Message> {
        struct LoaderSubscription;
        let url = self.flags.url.clone();
        Subscription::run_with_id(
            TypeId::of::<LoaderSubscription>(),
            stream::channel(16, |mut output| async move {
                //TODO: send errors to UI
                let handle = tokio::runtime::Handle::current();
                tokio::task::spawn_blocking(move || {
                    let Ok(path) = url.to_file_path() else { return };
                    let doc = mupdf::Document::open(path.as_os_str()).unwrap();
                    let page_count = doc.page_count().unwrap();

                    // Generate the table of contents
                    for index in 0..page_count {
                        //TODO: get from doc.outlines?
                        let label = format!("Page {}", index + 1);
                        handle
                            .block_on(async { output.send(Message::NavItem(index, label)).await })
                            .unwrap();
                    }

                    // Generate display lists (cannot be threaded)
                    for index in 0..page_count {
                        let page = doc.load_page(index).unwrap();
                        let display_list = page.to_display_list(false).unwrap();
                        handle
                            .block_on(async {
                                output
                                    .send(Message::DisplayList(index, Arc::new(display_list)))
                                    .await
                            })
                            .unwrap();
                    }
                })
                .await
                .unwrap();
                std::future::pending().await
            }),
        )
    }
}
