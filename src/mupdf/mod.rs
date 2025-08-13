use cosmic::{
    action,
    app::{Core, Settings, Task},
    cosmic_theme, executor,
    iced::{futures::SinkExt, stream, Length, Subscription},
    theme,
    widget::{self, nav_bar::Model, segmented_button::Entity},
    Application, Element,
};
use rayon::prelude::*;
use std::{any::TypeId, env, fs, io, sync::Arc};

const THUMBNAIL_WIDTH: u16 = 128;

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
fn display_list_to_handle(display_list: &mupdf::DisplayList, scale: f32) -> widget::image::Handle {
    let matrix = mupdf::Matrix::new_scale(scale, scale);
    let pixmap = display_list
        .to_pixmap(&matrix, &mupdf::Colorspace::device_rgb(), false)
        .unwrap();
    let mut data = Vec::new();
    //TODO: store raw image data?
    pixmap.write_to(&mut data, mupdf::ImageFormat::PNG).unwrap();
    widget::image::Handle::from_bytes(data)
}

struct Flags {
    url: url::Url,
}

#[derive(Clone, Debug)]
struct Page {
    index: i32,
    bounds: mupdf::Rect,
    display_list: Option<Arc<mupdf::DisplayList>>,
    icon_handle: Option<widget::image::Handle>,
    image_handle: Option<widget::image::Handle>,
}

#[derive(Clone, Debug)]
enum Message {
    DisplayList(i32, Arc<mupdf::DisplayList>),
    Image(Entity, widget::image::Handle),
    Pages(Vec<Page>),
    NavSelect(Entity),
    SearchActivate,
    SearchClear,
    SearchInput(String),
    SearchResults(Entity, Vec<mupdf::Quad>),
    Thumbnail(Entity, widget::image::Handle),
}

struct App {
    core: Core,
    dpi: f32,
    flags: Flags,
    nav_model: Model,
    search_active: bool,
    search_id: widget::Id,
    search_term: String,
}

impl App {
    fn entity_by_index(&self, index: i32) -> Option<Entity> {
        for entity in self.nav_model.iter() {
            if let Some(page) = self.nav_model.data::<Page>(entity) {
                if page.index == index {
                    return Some(entity);
                }
            }
        }
        None
    }

    fn update_page(&mut self) -> Task<Message> {
        let entity = self.nav_model.active();

        let Some(page) = self.nav_model.data::<Page>(entity) else {
            return Task::none();
        };
        if page.image_handle.is_some() {
            // Already has image cached
            return Task::none();
        }
        let Some(display_list) = page.display_list.clone() else {
            return Task::none();
        };

        let dpi = self.dpi;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let scale = dpi / 72.0;
                    Message::Image(entity, display_list_to_handle(&display_list, scale))
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

    fn header_end(&self) -> Vec<Element<Message>> {
        let cosmic_theme::Spacing { space_xxs, .. } = theme::spacing();

        let mut elements = Vec::with_capacity(1);

        if self.search_active {
            elements.push(
                widget::text_input::search_input("", &self.search_term)
                    .width(Length::Fixed(240.0))
                    .id(self.search_id.clone())
                    .on_clear(Message::SearchClear)
                    .on_input(Message::SearchInput)
                    .into(),
            );
        } else {
            elements.push(
                widget::button::icon(widget::icon::from_name("system-search-symbolic"))
                    .on_press(Message::SearchActivate)
                    .padding(space_xxs)
                    .into(),
            );
        }

        elements
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Message>) {
        let mut app = Self {
            core,
            //TODO: what is the best value to use?
            dpi: 192.0,
            flags,
            nav_model: Model::default(),
            search_active: false,
            search_id: widget::Id::unique(),
            search_term: String::new(),
        };
        let task = app.update_page();
        (app, task)
    }

    fn nav_bar(&self) -> Option<Element<action::Action<Message>>> {
        if !self.core.nav_bar_active() {
            return None;
        }

        let cosmic_theme::Spacing { space_xxs, .. } = theme::spacing();

        let mut column = widget::column::with_capacity(self.nav_model.len())
            .padding(space_xxs)
            .spacing(space_xxs);
        for entity in self.nav_model.iter() {
            if let Some(page) = self.nav_model.data::<Page>(entity) {
                if let Some(handle) = &page.icon_handle {
                    column = column.push(
                        widget::button::image(handle)
                            .width(THUMBNAIL_WIDTH)
                            .on_press(action::app(Message::NavSelect(entity))),
                    );
                } else {
                    column = column.push(
                        widget::button::custom_image_button(
                            widget::Space::with_height(Length::Fixed(
                                page.bounds.height() * (THUMBNAIL_WIDTH as f32)
                                    / page.bounds.width(),
                            )),
                            None,
                        )
                        .width(THUMBNAIL_WIDTH)
                        .on_press(action::app(Message::NavSelect(entity))),
                    );
                }
            }
        }

        let mut nav = widget::container(widget::scrollable(column).width(Length::Fixed(
            (THUMBNAIL_WIDTH as f32) + (space_xxs as f32) * 2.0,
        )));
        if !self.core.is_condensed() {
            nav = nav.max_width(280);
        }
        Some(nav.into())
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
                if let Some(entity) = self.entity_by_index(index) {
                    let mut tasks = Vec::with_capacity(2);
                    if let Some(page) = self.nav_model.data_mut::<Page>(entity) {
                        page.display_list = Some(display_list.clone());
                    }
                    if entity == self.nav_model.active() {
                        tasks.push(self.update_page());
                    }
                    tasks.push(Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                let scale =
                                    (THUMBNAIL_WIDTH as f32) / display_list.bounds().width();
                                Message::Thumbnail(
                                    entity,
                                    display_list_to_handle(&display_list, scale),
                                )
                            })
                            .await
                            .unwrap()
                        },
                        |x| action::app(x),
                    ));
                    return Task::batch(tasks);
                }
            }
            Message::Image(entity, handle) => {
                if let Some(page) = self.nav_model.data_mut::<Page>(entity) {
                    page.image_handle = Some(handle);
                }
            }
            Message::Pages(pages) => {
                self.nav_model.clear();
                for page in pages {
                    self.nav_model.insert().data::<Page>(page);
                }
                self.nav_model.activate_position(0);
                return self.update_page();
            }
            Message::NavSelect(entity) => {
                return self.on_nav_select(entity);
            }
            Message::SearchActivate => {
                self.search_active = true;
                return widget::text_input::focus(self.search_id.clone());
            }
            Message::SearchClear => {
                self.search_active = false;
            }
            Message::SearchInput(term) => {
                self.search_term = term.clone();
            }
            Message::SearchResults(entity, quads) => {
                //TODO
            }
            Message::Thumbnail(entity, handle) => {
                if let Some(page) = self.nav_model.data_mut::<Page>(entity) {
                    page.icon_handle = Some(handle);
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<Message> {
        let entity = self.nav_model.active();

        // Handle cached images
        if let Some(page) = self.nav_model.data::<Page>(entity) {
            if let Some(handle) = &page.image_handle {
                return widget::image::viewer(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into();
            }
        }

        widget::text("Page loading...").into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(2);

        struct LoaderSubscription;
        let url = self.flags.url.clone();
        subscriptions.push(Subscription::run_with_id(
            (TypeId::of::<LoaderSubscription>(), url.clone()),
            stream::channel(16, |mut output| async move {
                //TODO: send errors to UI
                let handle = tokio::runtime::Handle::current();
                tokio::task::spawn_blocking(move || {
                    let Ok(path) = url.to_file_path() else { return };
                    let doc = mupdf::Document::open(path.as_os_str()).unwrap();
                    let page_count = doc.page_count().unwrap();
                    //TODO: use outline for document tree view eprintln!("{:#?}", doc.outlines());

                    // Generate the table of contents
                    let mut pages = Vec::with_capacity(usize::try_from(page_count).unwrap());
                    for index in 0..page_count {
                        let page = doc.load_page(index).unwrap();
                        //TODO: get label?
                        let bounds = page.bounds().unwrap();
                        pages.push(Page {
                            index,
                            bounds,
                            display_list: None,
                            icon_handle: None,
                            image_handle: None,
                        });
                    }
                    handle
                        .block_on(async { output.send(Message::Pages(pages)).await })
                        .unwrap();

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
        ));

        if self.search_active && !self.search_term.is_empty() {
            //TODO: efficiently cache this somehow
            let mut display_lists = Vec::with_capacity(self.nav_model.len());
            for entity in self.nav_model.iter() {
                if let Some(page) = self.nav_model.data::<Page>(entity) {
                    if let Some(display_list) = page.display_list.clone() {
                        display_lists.push((entity, display_list));
                    }
                }
            }

            struct SearchSubscription;
            let term = self.search_term.clone();
            subscriptions.push(Subscription::run_with_id(
                (TypeId::of::<SearchSubscription>(), term.clone()),
                stream::channel(16, |output| async move {
                    let output = Arc::new(tokio::sync::Mutex::new(output));
                    let handle = tokio::runtime::Handle::current();
                    tokio::task::spawn_blocking(move || {
                        let timer = std::time::Instant::now();
                        display_lists.par_iter().for_each(|(entity, display_list)| {
                            let quads = display_list.search(&term, 100).unwrap();
                            if !quads.is_empty() {
                                eprintln!("{:?}: {:?} results", entity, quads.len(),);
                                let quads_vec: Vec<mupdf::Quad> = quads.into_iter().collect();
                                let output = output.clone();
                                handle
                                    .block_on(async move {
                                        output
                                            .lock()
                                            .await
                                            .send(Message::SearchResults(*entity, quads_vec))
                                            .await
                                    })
                                    .unwrap();
                            }
                        });
                        eprintln!("searched for {:?} in {:?}", term, timer.elapsed());
                    })
                    .await
                    .unwrap();
                    std::future::pending().await
                }),
            ));
        }

        Subscription::batch(subscriptions)
    }
}
