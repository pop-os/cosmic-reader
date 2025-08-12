// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    cosmic_theme::palette::{blend::Compose, WithAlpha},
    iced::{
        advanced::graphics::text::font_system,
        event::{Event, Status},
        keyboard::{Event as KeyEvent, Modifiers},
        mouse::{self, Button, Event as MouseEvent, ScrollDelta},
        Color, Element, Length, Padding, Point, Rectangle, Size, Vector,
    },
    iced_core::{
        clipboard::Clipboard,
        image,
        keyboard::{key::Named, Key},
        layout::{self, Layout},
        renderer::{self, Quad, Renderer as _},
        widget::{
            self,
            operation::{self, Operation},
            tree, Id, Widget,
        },
        Border, Radians, Shell,
    },
    theme::Theme,
    Renderer,
};
use std::{
    cell::Cell,
    cmp,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct Page {
    id: Option<Id>,
    padding: Padding,
}

impl Page {
    pub fn new() -> Self {
        Self {
            id: None,
            padding: Padding::new(0.0),
        }
    }

    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }
}

impl<Message> Widget<Message, cosmic::Theme, Renderer> for Page
where
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        //TODO
        layout::Node::new(limits.max())
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();

        mouse::Interaction::Idle
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let instant = Instant::now();

        let state = tree.state.downcast_ref::<State>();

        let duration = instant.elapsed();
        log::debug!("redraw: {:?}", duration);
    }

    fn on_event(
        &mut self,
        tree: &mut widget::Tree,
        event: Event,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle<f32>,
    ) -> Status {
        let state = tree.state.downcast_mut::<State>();

        Status::Ignored
    }
}

impl<'a, Message> From<Page> for Element<'a, Message, cosmic::Theme, Renderer>
where
    Message: Clone + 'a,
{
    fn from(page: Page) -> Self {
        Self::new(page)
    }
}

pub struct State;

impl State {
    /// Creates a new [`State`].
    pub fn new() -> State {
        State
    }
}
