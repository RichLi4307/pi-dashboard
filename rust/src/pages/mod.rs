//! Page abstraction and manager.
//!
//! Mirrors `panel.py`'s mode framework. All page registration, touch routing
//! and IPC `switch_mode` go through `PageManager`; `main.rs` never contains
//! page-specific logic.

use std::collections::HashMap;

use anyhow::Result;

use crate::fb::Framebuffer;
use crate::metrics::MetricsSnapshot;
use crate::touch::TouchEvent;

pub mod monitor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageAction {
    None,
    /// Switch to the page with the given id.
    Switch(&'static str),
}

pub trait Page {
    fn id(&self) -> &'static str;

    /// Draw the page onto the framebuffer. Implementations must mark dirty
    /// regions for anything they change.
    fn render(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()>;

    /// Handle a touch event. Return a page action if the event should cause
    /// a page switch or other global effect.
    fn on_touch(&mut self, ev: TouchEvent) -> PageAction;

    /// Scroll the container list, returning `(offset, total)` if this page
    /// supports it. Default returns `None`.
    fn scroll_containers(&mut self, _total: usize) -> Option<(usize, usize)> {
        None
    }

    /// Called when this page becomes active. Default implementation marks the
    /// whole framebuffer dirty so the next render refreshes everything.
    fn on_enter(&mut self, fb: &mut Framebuffer) {
        fb.mark_full_dirty();
    }
}

pub struct PageManager {
    pages: HashMap<&'static str, Box<dyn Page>>,
    active: &'static str,
}

impl PageManager {
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            active: "",
        }
    }

    pub fn register(&mut self, page: Box<dyn Page>) {
        let id = page.id();
        if self.active.is_empty() {
            self.active = id;
        }
        self.pages.insert(id, page);
    }

    pub fn switch(&mut self, id: &'static str, fb: &mut Framebuffer) -> bool {
        if id == self.active {
            return true;
        }
        if let Some(page) = self.pages.get_mut(id) {
            self.active = id;
            page.on_enter(fb);
            true
        } else {
            false
        }
    }

    pub fn active_id(&self) -> &'static str {
        self.active
    }

    pub fn active(&mut self) -> Option<&mut Box<dyn Page>> {
        self.pages.get_mut(self.active)
    }

    pub fn route_touch(&mut self, ev: TouchEvent) -> PageAction {
        if let Some(page) = self.pages.get_mut(self.active) {
            page.on_touch(ev)
        } else {
            PageAction::None
        }
    }

    pub fn render_active(&mut self, fb: &mut Framebuffer, snapshot: &MetricsSnapshot) -> Result<()> {
        if let Some(page) = self.pages.get_mut(self.active) {
            page.render(fb, snapshot)?;
        }
        Ok(())
    }

    pub fn scroll_containers(&mut self, total: usize) -> Option<(usize, usize)> {
        if let Some(page) = self.pages.get_mut(self.active) {
            page.scroll_containers(total)
        } else {
            None
        }
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.pages.contains_key(id)
    }
}
