use cosmic::{
    Application, Element, action,
    app::{Core, Settings, Task},
    cosmic_theme, executor,
    iced::{
        Alignment, Color, ContentFit, Length, Rectangle, Subscription,
        core::SmolStr,
        event::{self, Event},
        futures::SinkExt,
        keyboard::{Event as KeyEvent, Key, Modifiers, key::Named},
        mouse::ScrollDelta,
        stream,
        widget::scrollable,
        window,
    },
    theme,
    widget::{self, nav_bar::Model, segmented_button::Entity},
};
use rayon::prelude::*;
use std::{any::TypeId, cell::Cell, fmt, process, sync::Arc};

use crate::fl;

const THUMBNAIL_WIDTH: u16 = 128;

mod argparse;
mod thumbnail;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args = argparse::parse();

    if let Some(output) = args.thumbnail_opt {
        let Some(input) = args.url_opt else {
            log::error!("thumbnailer can only handle exactly one URL");
            process::exit(1);
        };

        match thumbnail::main(&input, &output, args.size_opt) {
            Ok(()) => process::exit(0),
            Err(err) => {
                log::error!("failed to thumbnail '{}': {}", input, err);
                process::exit(1);
            }
        }
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    match fork::daemon(true, true) {
        Ok(fork::Fork::Child) => (),
        Ok(fork::Fork::Parent(_child_pid)) => process::exit(0),
        Err(err) => {
            eprintln!("failed to daemonize: {:?}", err);
            process::exit(1);
        }
    }

    crate::localize::localize();

    cosmic::app::run::<App>(
        Settings::default(),
        Flags {
            url_opt: args.url_opt,
        },
    )?;
    Ok(())
}

//TODO: return errors
fn display_list_to_image(display_list: &mupdf::DisplayList, scale: f32) -> widget::image::Handle {
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
    url_opt: Option<url::Url>,
}

#[derive(Clone, Debug)]
struct Page {
    index: i32,
    bounds: mupdf::Rect,
    display_list: Option<Arc<mupdf::DisplayList>>,
    icon_bounds: Cell<Option<Rectangle>>,
    icon_handle: Option<widget::image::Handle>,
    svg_handle: Option<widget::svg::Handle>,
}

#[derive(Clone, Debug)]
enum Message {
    DisplayList(i32, Arc<mupdf::DisplayList>),
    FileLoad(url::Url),
    FileOpen,
    Fullscreen,
    Key(Modifiers, Key, Option<SmolStr>),
    ModifiersChanged(Modifiers),
    NavScroll(scrollable::Viewport),
    NavSelect(Entity),
    Pages(Vec<Page>),
    SearchActivate,
    SearchClear,
    SearchInput(String),
    SearchResults(Entity, Vec<mupdf::Quad>),
    Svg(Entity, widget::svg::Handle),
    Thumbnail(Entity, widget::image::Handle),
    ZoomDropdown(usize),
    ZoomScroll(ScrollDelta),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Zoom {
    FitBoth,
    FitHeight,
    FitWidth,
    Percent(i16),
}

impl Zoom {
    fn all() -> &'static [Self] {
        &[
            Zoom::FitBoth,
            Zoom::FitHeight,
            Zoom::FitWidth,
            Zoom::Percent(25),
            Zoom::Percent(50),
            Zoom::Percent(75),
            Zoom::Percent(100),
            Zoom::Percent(125),
            Zoom::Percent(150),
            Zoom::Percent(175),
            Zoom::Percent(200),
            Zoom::Percent(225),
            Zoom::Percent(250),
            Zoom::Percent(275),
            Zoom::Percent(300),
            Zoom::Percent(325),
            Zoom::Percent(350),
            Zoom::Percent(375),
            Zoom::Percent(400),
            Zoom::Percent(425),
            Zoom::Percent(450),
            Zoom::Percent(475),
            Zoom::Percent(500),
        ]
    }
}

impl fmt::Display for Zoom {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        //TODO: translate?
        match self {
            Zoom::FitBoth => write!(f, "Fit width and height"),
            Zoom::FitHeight => write!(f, "Fit height"),
            Zoom::FitWidth => write!(f, "Fit width"),
            Zoom::Percent(percent) => write!(f, "{}%", percent),
        }
    }
}

struct App {
    core: Core,
    flags: Flags,
    fullscreen: bool,
    modifiers: Modifiers,
    nav_model: Model,
    nav_scroll_id: widget::Id,
    nav_viewport: Option<scrollable::Viewport>,
    search_active: bool,
    search_id: widget::Id,
    search_term: String,
    view_ratio: Cell<f32>,
    zoom: Zoom,
    zoom_names: Vec<String>,
    zoom_scroll: f32,
}

impl App {
    fn entity_by_index(&self, index: i32) -> Option<Entity> {
        for entity in self.nav_model.iter() {
            if let Some(page) = self.nav_model.data::<Page>(entity)
                && page.index == index
            {
                return Some(entity);
            }
        }
        None
    }

    fn update_page(&mut self) -> Task<Message> {
        let entity = self.nav_model.active();
        let Some(page) = self.nav_model.data::<Page>(entity) else {
            return Task::none();
        };
        let mut tasks = Vec::with_capacity(2);
        if let Some(viewport) = &self.nav_viewport {
            let mut bounds = viewport.bounds();
            // Adjust bounds to match scroll offset
            let offset = viewport.absolute_offset();
            bounds.x = offset.x;
            bounds.y = offset.y;
            if let Some(icon_bounds) = page.icon_bounds.get() {
                if bounds.y > icon_bounds.y {
                    // Scroll up if necessary
                    tasks.push(scrollable::scroll_to(
                        self.nav_scroll_id.clone(),
                        scrollable::AbsoluteOffset {
                            x: 0.0,
                            y: icon_bounds.y,
                        },
                    ));
                } else if bounds.y + bounds.height < icon_bounds.y + icon_bounds.height {
                    // Scroll down if necessary
                    tasks.push(scrollable::scroll_to(
                        self.nav_scroll_id.clone(),
                        scrollable::AbsoluteOffset {
                            x: 0.0,
                            y: icon_bounds.y + icon_bounds.height - bounds.height,
                        },
                    ));
                }
            }
        }
        if page.svg_handle.is_none()
            && let Some(display_list) = page.display_list.clone()
        {
            tasks.push(Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let svg = display_list.to_svg(&mupdf::Matrix::IDENTITY).unwrap();
                        Message::Svg(entity, widget::svg::Handle::from_memory(svg.into_bytes()))
                    })
                    .await
                    .unwrap()
                },
                action::app,
            ));
        }
        Task::batch(tasks)
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

    fn header_start(&self) -> Vec<Element<'_, Message>> {
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

    fn header_end(&self) -> Vec<Element<'_, Message>> {
        vec![
            widget::dropdown(
                &self.zoom_names,
                Zoom::all().iter().position(|zoom| zoom == &self.zoom),
                Message::ZoomDropdown,
            )
            .into(),
        ]
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Message>) {
        let mut zoom_names = Vec::new();
        for zoom in Zoom::all() {
            zoom_names.push(zoom.to_string());
        }

        let mut app = Self {
            core,
            //TODO: what is the best value to use?
            flags,
            fullscreen: false,
            modifiers: Modifiers::default(),
            nav_model: Model::default(),
            nav_scroll_id: widget::Id::unique(),
            nav_viewport: None,
            search_active: false,
            search_id: widget::Id::unique(),
            search_term: String::new(),
            view_ratio: Cell::new(1.0),
            zoom: Zoom::FitBoth,
            zoom_names,
            zoom_scroll: 0.0,
        };
        let task = app.update_page();
        (app, task)
    }

    fn nav_bar(&self) -> Option<Element<'_, action::Action<Message>>> {
        if !self.core.nav_bar_active() || self.fullscreen {
            return None;
        }

        let cosmic_theme::Spacing { space_xxs, .. } = theme::spacing();

        let mut column = widget::column::with_capacity(self.nav_model.len())
            .padding(space_xxs)
            .spacing(space_xxs);
        let x = space_xxs as f32;
        let mut y = space_xxs as f32;
        let mut count = 0;
        for entity in self.nav_model.iter() {
            if let Some(page) = self.nav_model.data::<Page>(entity) {
                if count > 0 {
                    y += space_xxs as f32;
                }
                //TODO: cache sizes during icon generation?
                let width = THUMBNAIL_WIDTH as f32;
                let height = page.bounds.height() * width / page.bounds.width();
                page.icon_bounds.set(Some(Rectangle {
                    x,
                    y,
                    width,
                    height,
                }));
                if let Some(handle) = &page.icon_handle {
                    column = column.push(
                        widget::button::image(handle)
                            .width(width)
                            .height(height)
                            .on_press(action::app(Message::NavSelect(entity)))
                            .selected(entity == self.nav_model.active()),
                    );
                } else {
                    column = column.push(
                        widget::button::custom_image_button(
                            widget::Space::with_height(Length::Fixed(height)),
                            None,
                        )
                        .width(width)
                        .height(height)
                        .on_press(action::app(Message::NavSelect(entity)))
                        .selected(entity == self.nav_model.active()),
                    );
                }
                y += height;
                count += 1;
            }
        }

        let mut nav = widget::container(
            scrollable(column)
                .id(self.nav_scroll_id.clone())
                .on_scroll(|x| action::app(Message::NavScroll(x)))
                .width(Length::Fixed(
                    (THUMBNAIL_WIDTH as f32) + (space_xxs as f32) * 2.0,
                )),
        );
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
                                    display_list_to_image(&display_list, scale),
                                )
                            })
                            .await
                            .unwrap()
                        },
                        action::app,
                    ));
                    return Task::batch(tasks);
                }
            }
            Message::FileLoad(url) => {
                self.nav_model.clear();
                self.flags.url_opt = Some(url);
            }
            Message::FileOpen => {
                #[cfg(feature = "xdg-portal")]
                return Task::perform(
                    async move {
                        let dialog = cosmic::dialog::file_chooser::open::Dialog::new()
                            .title(fl!("open-file"));
                        match dialog.open_file().await {
                            Ok(response) => {
                                action::app(Message::FileLoad(response.url().to_owned()))
                            }
                            Err(err) => {
                                log::warn!("failed to open file: {}", err);
                                action::none()
                            }
                        }
                    },
                    |x| x,
                );
            }
            Message::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                self.core.window.show_headerbar = !self.fullscreen;
                if let Some(window_id) = self.core.main_window_id() {
                    return window::change_mode(
                        window_id,
                        if self.fullscreen {
                            window::Mode::Fullscreen
                        } else {
                            window::Mode::Windowed
                        },
                    );
                }
            }
            //TODO: move to key binds and set up menu
            Message::Key(_modifiers, key, _text) => match &key {
                Key::Named(Named::ArrowUp | Named::ArrowLeft | Named::PageUp) => {
                    let pos = self
                        .nav_model
                        .position(self.nav_model.active())
                        .unwrap_or(0);
                    if let Some(new_pos) = pos.checked_sub(1) {
                        self.nav_model.activate_position(new_pos);
                    }
                    return self.update_page();
                }
                Key::Named(Named::ArrowDown | Named::ArrowRight | Named::PageDown) => {
                    let pos = self
                        .nav_model
                        .position(self.nav_model.active())
                        .unwrap_or(0);
                    if let Some(new_pos) = pos.checked_add(1) {
                        self.nav_model.activate_position(new_pos);
                    }
                    return self.update_page();
                }
                Key::Named(Named::Enter) => {
                    return self.update(Message::Fullscreen);
                }
                Key::Named(Named::Escape) => {
                    self.search_active = false;
                }
                Key::Character(c) => match c.as_str() {
                    "0" => {
                        self.zoom = Zoom::Percent(100);
                        println!("{:?}", self.zoom)
                    }
                    "-" => {
                        let percent = match self.zoom {
                            Zoom::Percent(percent) => percent,
                            _ => ((self.view_ratio.get() * 4.0).round() as i16) * 25,
                        };
                        self.zoom = Zoom::Percent((percent - 25).clamp(25, 500));
                        println!("{:?}", self.zoom)
                    }
                    "=" => {
                        let percent = match self.zoom {
                            Zoom::Percent(percent) => percent,
                            _ => ((self.view_ratio.get() * 4.0).round() as i16) * 25,
                        };
                        self.zoom = Zoom::Percent((percent + 25).clamp(25, 500));
                        println!("{:?}", self.zoom)
                    }
                    "f" => {
                        self.zoom = Zoom::FitBoth;
                    }
                    "h" => {
                        self.zoom = Zoom::FitHeight;
                    }
                    "w" => {
                        self.zoom = Zoom::FitWidth;
                    }
                    "s" | "/" => {
                        self.search_active = true;
                        return widget::text_input::focus(self.search_id.clone());
                    }
                    _ => {}
                },
                _ => {}
            },
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
            }
            Message::NavScroll(viewport) => {
                self.nav_viewport = Some(viewport);
            }
            Message::NavSelect(entity) => {
                return self.on_nav_select(entity);
            }
            Message::Pages(pages) => {
                self.nav_model.clear();
                for page in pages {
                    self.nav_model.insert().data::<Page>(page);
                }
                self.nav_model.activate_position(0);
                return self.update_page();
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
            Message::Svg(entity, handle) => {
                if let Some(page) = self.nav_model.data_mut::<Page>(entity) {
                    page.svg_handle = Some(handle);
                }
            }
            Message::Thumbnail(entity, handle) => {
                if let Some(page) = self.nav_model.data_mut::<Page>(entity) {
                    page.icon_handle = Some(handle);
                }
            }
            Message::ZoomDropdown(index) => {
                if let Some(zoom) = Zoom::all().get(index) {
                    self.zoom = *zoom;
                }
            }
            Message::ZoomScroll(delta) => {
                self.zoom_scroll += match delta {
                    ScrollDelta::Lines { y, .. } => y,
                    //TODO: best pixel to line conversion ratio?
                    ScrollDelta::Pixels { y, .. } => y / 20.0,
                };
                let mut percent = match self.zoom {
                    Zoom::Percent(percent) => percent,
                    _ => ((self.view_ratio.get() * 4.0).round() as i16) * 25,
                };
                while self.zoom_scroll >= 1.0 {
                    percent += 25;
                    self.zoom_scroll -= 1.0;
                }
                while self.zoom_scroll <= -1.0 {
                    percent -= 25;
                    self.zoom_scroll += 1.0;
                }
                self.zoom = Zoom::Percent(percent.clamp(25, 500));
                println!("{}", self.zoom);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let entity = self.nav_model.active();

        // Handle cached images
        if let Some(page) = self.nav_model.data::<Page>(entity) {
            return widget::responsive(move |size| {
                let ratio = match self.zoom {
                    Zoom::FitHeight => size.height / page.bounds.height(),
                    Zoom::FitWidth => size.width / page.bounds.width(),
                    Zoom::FitBoth => {
                        (size.width / page.bounds.width()).min(size.height / page.bounds.height())
                    }
                    //TODO: adjust ratio by DPI
                    Zoom::Percent(percent) => (percent as f32) / 100.0,
                };
                self.view_ratio.set(ratio);
                let width = page.bounds.width() * ratio;
                let height = page.bounds.height() * ratio;
                let mut container = widget::container(
                    widget::container(if let Some(handle) = &page.svg_handle {
                        Element::from(
                            widget::svg(handle.clone())
                                .content_fit(ContentFit::Fill)
                                .width(width)
                                .height(height),
                        )
                    } else {
                        Element::from(widget::Space::new(width, height))
                    })
                    .style(|_theme| widget::container::background(Color::WHITE)),
                );
                if size.width > width {
                    container = container.center_x(size.width);
                }
                if size.height > height {
                    container = container.center_y(size.height);
                }
                let mut mouse_area =
                    widget::mouse_area(container).on_double_press(Message::Fullscreen);
                if self.modifiers.contains(Modifiers::CTRL) {
                    mouse_area = mouse_area.on_scroll(Message::ZoomScroll);
                }
                scrollable(mouse_area)
                    .direction(scrollable::Direction::Both {
                        vertical: Default::default(),
                        horizontal: Default::default(),
                    })
                    .into()
            })
            .into();
        }

        if self.flags.url_opt.is_none() {
            //TODO: use space variables
            let column = widget::column::with_capacity(4)
                .align_x(Alignment::Center)
                .spacing(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .push(widget::vertical_space())
                .push(
                    widget::column::with_capacity(2)
                        .align_x(Alignment::Center)
                        .spacing(8)
                        .push(widget::icon::from_name("folder-symbolic").size(64))
                        .push(widget::text::body(fl!("no-file-open"))),
                )
                .push(widget::button::suggested(fl!("open-file")).on_press(Message::FileOpen))
                .push(widget::vertical_space());

            return column.into();
        }

        widget::horizontal_space().into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(3);

        subscriptions.push(event::listen_with(
            |event, status, _window_id| match event {
                Event::Keyboard(KeyEvent::KeyPressed {
                    key,
                    modifiers,
                    text,
                    ..
                }) => match status {
                    event::Status::Ignored => Some(Message::Key(modifiers, key, text)),
                    event::Status::Captured => None,
                },
                Event::Keyboard(KeyEvent::ModifiersChanged(modifiers)) => {
                    Some(Message::ModifiersChanged(modifiers))
                }
                _ => None,
            },
        ));

        struct LoaderSubscription;
        if let Some(url) = self.flags.url_opt.clone() {
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
                                icon_bounds: Cell::new(None),
                                icon_handle: None,
                                svg_handle: None,
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
        }

        if self.search_active && !self.search_term.is_empty() {
            //TODO: efficiently cache this somehow
            let mut display_lists = Vec::with_capacity(self.nav_model.len());
            for entity in self.nav_model.iter() {
                if let Some(page) = self.nav_model.data::<Page>(entity)
                    && let Some(display_list) = page.display_list.clone()
                {
                    display_lists.push((entity, display_list));
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
