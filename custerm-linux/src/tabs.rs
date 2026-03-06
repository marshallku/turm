use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::gdk;
use gtk4::glib;

use custerm_core::config::CustermConfig;

use vte4::prelude::*;

use crate::panel::Panel;
use crate::split::{CloseResult, TabContent};
use crate::terminal::TerminalPanel;

pub struct TabManager {
    pub notebook: gtk4::Notebook,
    tabs: Rc<RefCell<Vec<TabContent>>>,
    focused: Rc<RefCell<Option<Rc<TerminalPanel>>>>,
    config: Rc<RefCell<CustermConfig>>,
}

impl TabManager {
    pub fn new(config: &CustermConfig, window: &gtk4::ApplicationWindow) -> Rc<Self> {
        let notebook = gtk4::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_show_border(false);
        notebook.set_show_tabs(false);

        let manager = Rc::new(Self {
            notebook,
            tabs: Rc::new(RefCell::new(Vec::new())),
            focused: Rc::new(RefCell::new(None)),
            config: Rc::new(RefCell::new(config.clone())),
        });

        // Tab bar CSS
        let css = gtk4::CssProvider::new();
        css.load_from_string(TAB_CSS);
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 3,
        );

        // Update tab bar visibility on page remove
        let tabs_ref = manager.tabs.clone();
        manager.notebook.connect_page_removed(move |notebook, _, _| {
            notebook.set_show_tabs(tabs_ref.borrow().len() > 1);
        });

        // Focus the right panel when switching tabs
        let focused = manager.focused.clone();
        let tabs_ref = manager.tabs.clone();
        manager.notebook.connect_switch_page(move |_, _, page_num| {
            let tabs = tabs_ref.borrow();
            if let Some(tab) = tabs.get(page_num as usize) {
                let mut panels = Vec::new();
                tab.root.borrow().collect_panels(&mut panels);
                // Focus first panel in this tab, or the previously focused one if it's in this tab
                let current_focused = focused.borrow().clone();
                let should_focus = current_focused
                    .filter(|f| panels.iter().any(|p| Rc::ptr_eq(p, f)))
                    .or_else(|| panels.into_iter().next());
                if let Some(panel) = should_focus {
                    panel.grab_focus();
                }
            }
        });

        // Keyboard shortcuts
        setup_shortcuts(&manager, window);

        // First tab
        let mgr = manager.clone();
        let win = window.clone();
        mgr.add_tab(&win);

        manager
    }

    pub fn add_tab(self: &Rc<Self>, window: &gtk4::ApplicationWindow) {
        let config = self.config.borrow().clone();
        let panel = self.create_panel(&config, window);

        let tab_content = TabContent::new(panel.clone());
        let tab_label = self.make_tab_label(&panel);

        self.notebook
            .append_page(&tab_content.container, Some(&tab_label));
        self.notebook
            .set_tab_reorderable(&tab_content.container, true);
        self.tabs.borrow_mut().push(tab_content);
        self.update_tab_visibility();

        let page_num = self.notebook.n_pages() - 1;
        self.notebook.set_current_page(Some(page_num));
        *self.focused.borrow_mut() = Some(panel.clone());
        panel.grab_focus();
    }

    pub fn split_focused(
        self: &Rc<Self>,
        orientation: gtk4::Orientation,
        window: &gtk4::ApplicationWindow,
    ) {
        let focused = self.focused.borrow().clone();
        let Some(focused_panel) = focused else { return };
        let Some(tab_idx) = self.tab_index_of(&focused_panel) else {
            return;
        };

        let config = self.config.borrow().clone();
        let new_panel = self.create_panel(&config, window);

        {
            let tabs = self.tabs.borrow();
            tabs[tab_idx].split(&focused_panel, &new_panel, orientation);
        }

        *self.focused.borrow_mut() = Some(new_panel.clone());
        new_panel.grab_focus();
    }

    pub fn close_focused(self: &Rc<Self>, window: &gtk4::ApplicationWindow) {
        let focused = self.focused.borrow().clone();
        let Some(focused_panel) = focused else { return };
        let Some(tab_idx) = self.tab_index_of(&focused_panel) else {
            return;
        };

        let result = {
            let tabs = self.tabs.borrow();
            tabs[tab_idx].close_panel(&focused_panel)
        };

        match result {
            CloseResult::CloseTab => {
                self.tabs.borrow_mut().remove(tab_idx);
                self.notebook.remove_page(Some(tab_idx as u32));
                self.update_tab_visibility();

                if self.tabs.borrow().is_empty() {
                    window.close();
                    return;
                }
                self.focus_active_tab_panel();
            }
            CloseResult::Closed { focus_target } => {
                if let Some(panel) = focus_target {
                    *self.focused.borrow_mut() = Some(panel.clone());
                    panel.grab_focus();
                } else {
                    // Fallback: focus any panel in the same tab
                    let tabs = self.tabs.borrow();
                    let mut panels = Vec::new();
                    tabs[tab_idx].root.borrow().collect_panels(&mut panels);
                    if let Some(panel) = panels.first() {
                        *self.focused.borrow_mut() = Some(panel.clone());
                        panel.grab_focus();
                    }
                }
            }
        }
    }

    pub fn active_panel(&self) -> Option<Rc<TerminalPanel>> {
        self.focused.borrow().clone()
    }

    pub fn update_config(&self, config: &CustermConfig) {
        *self.config.borrow_mut() = config.clone();
        for tab in self.tabs.borrow().iter() {
            let mut panels = Vec::new();
            tab.root.borrow().collect_panels(&mut panels);
            for panel in panels {
                panel.apply_config(config);
            }
        }
    }

    /// Navigate focus between split panes
    pub fn focus_direction(&self, direction: FocusDirection) {
        let focused = self.focused.borrow().clone();
        let Some(focused_panel) = focused else { return };
        let Some(tab_idx) = self.tab_index_of(&focused_panel) else {
            return;
        };

        let tabs = self.tabs.borrow();
        let mut panels = Vec::new();
        tabs[tab_idx].root.borrow().collect_panels(&mut panels);

        if panels.len() < 2 {
            return;
        }

        // Simple: cycle through panels in order based on direction
        let current_idx = panels
            .iter()
            .position(|p| Rc::ptr_eq(p, &focused_panel))
            .unwrap_or(0);

        let next_idx = match direction {
            FocusDirection::Next => (current_idx + 1) % panels.len(),
            FocusDirection::Prev => {
                if current_idx == 0 {
                    panels.len() - 1
                } else {
                    current_idx - 1
                }
            }
        };

        let next_panel = &panels[next_idx];
        *self.focused.borrow_mut() = Some(next_panel.clone());
        next_panel.grab_focus();
    }

    // -- Private helpers --

    fn create_panel(
        self: &Rc<Self>,
        config: &CustermConfig,
        window: &gtk4::ApplicationWindow,
    ) -> Rc<TerminalPanel> {
        let mgr = Rc::downgrade(self);
        let win = window.clone();
        let widget_holder: Rc<RefCell<Option<gtk4::Widget>>> = Rc::new(RefCell::new(None));
        let widget_for_exit = widget_holder.clone();

        let panel = Rc::new(TerminalPanel::new(config, move || {
            let widget = widget_for_exit.borrow().clone();
            let mgr = mgr.clone();
            let win = win.clone();
            glib::idle_add_local_once(move || {
                let Some(mgr) = mgr.upgrade() else { return };
                if let Some(ref w) = widget {
                    mgr.handle_panel_exit(w, &win);
                }
            });
        }));

        *widget_holder.borrow_mut() = Some(panel.widget().clone());

        // Apply background
        if let Some(ref path) = config.background.image {
            let p = std::path::Path::new(path);
            if p.exists() {
                panel.set_background(p);
            }
        }

        self.track_focus(&panel);
        panel
    }

    fn track_focus(&self, panel: &Rc<TerminalPanel>) {
        let focused = self.focused.clone();
        let panel_weak = Rc::downgrade(panel);
        let controller = gtk4::EventControllerFocus::new();
        controller.connect_enter(move |_| {
            if let Some(panel) = panel_weak.upgrade() {
                *focused.borrow_mut() = Some(panel);
            }
        });
        panel.terminal.add_controller(controller);
    }

    fn handle_panel_exit(&self, panel_widget: &gtk4::Widget, window: &gtk4::ApplicationWindow) {
        let tabs = self.tabs.borrow();
        for (tab_idx, tab) in tabs.iter().enumerate() {
            let mut panels = Vec::new();
            tab.root.borrow().collect_panels(&mut panels);
            if let Some(panel) = panels.iter().find(|p| p.widget() == panel_widget) {
                let result = tab.close_panel(panel);
                match result {
                    CloseResult::CloseTab => {
                        drop(tabs);
                        self.tabs.borrow_mut().remove(tab_idx);
                        self.notebook.remove_page(Some(tab_idx as u32));
                        self.update_tab_visibility();

                        if self.tabs.borrow().is_empty() {
                            window.close();
                            return;
                        }
                        self.focus_active_tab_panel();
                    }
                    CloseResult::Closed { focus_target } => {
                        if let Some(p) = focus_target {
                            *self.focused.borrow_mut() = Some(p.clone());
                            p.grab_focus();
                        } else {
                            let mut remaining = Vec::new();
                            tab.root.borrow().collect_panels(&mut remaining);
                            if let Some(p) = remaining.first() {
                                *self.focused.borrow_mut() = Some(p.clone());
                                p.grab_focus();
                            }
                        }
                    }
                }
                return;
            }
        }
    }

    fn tab_index_of(&self, panel: &Rc<TerminalPanel>) -> Option<usize> {
        let tabs = self.tabs.borrow();
        for (i, tab) in tabs.iter().enumerate() {
            let mut panels = Vec::new();
            tab.root.borrow().collect_panels(&mut panels);
            if panels.iter().any(|p| Rc::ptr_eq(p, panel)) {
                return Some(i);
            }
        }
        None
    }

    fn update_tab_visibility(&self) {
        self.notebook
            .set_show_tabs(self.tabs.borrow().len() > 1);
    }

    fn focus_active_tab_panel(&self) {
        if let Some(page) = self.notebook.current_page() {
            let tabs = self.tabs.borrow();
            if let Some(tab) = tabs.get(page as usize) {
                let mut panels = Vec::new();
                tab.root.borrow().collect_panels(&mut panels);
                if let Some(p) = panels.first() {
                    *self.focused.borrow_mut() = Some(p.clone());
                    p.grab_focus();
                }
            }
        }
    }

    fn make_tab_label(&self, panel: &Rc<TerminalPanel>) -> gtk4::Box {
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let label = gtk4::Label::new(Some("Terminal"));
        label.set_hexpand(true);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(20);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("custerm-tab-close");
        close_btn.set_tooltip_text(Some("Close tab"));

        hbox.append(&label);
        hbox.append(&close_btn);

        let label_clone = label.clone();
        panel
            .terminal
            .connect_window_title_changed(move |term: &vte4::Terminal| {
                if let Some(title) = term.window_title() {
                    label_clone.set_text(&title);
                }
            });

        let nb = self.notebook.clone();
        let tabs = self.tabs.clone();
        let focused = self.focused.clone();
        close_btn.connect_clicked(move |btn| {
            // Find the tab page this button belongs to
            let Some(page_widget) = btn
                .ancestor(gtk4::Box::static_type())
                .and_then(|w| w.parent())
                .and_then(|w| w.parent())
            else {
                return;
            };
            // Alternative: find by notebook
            let tab_count = tabs.borrow().len();
            for i in 0..tab_count {
                if let Some(page) = nb.nth_page(Some(i as u32)) {
                    if page == page_widget {
                        tabs.borrow_mut().remove(i);
                        nb.remove_page(Some(i as u32));
                        nb.set_show_tabs(tabs.borrow().len() > 1);

                        // Update focus
                        if let Some(new_page) = nb.current_page() {
                            let tabs_ref = tabs.borrow();
                            if let Some(tab) = tabs_ref.get(new_page as usize) {
                                let mut panels = Vec::new();
                                tab.root.borrow().collect_panels(&mut panels);
                                if let Some(p) = panels.first() {
                                    *focused.borrow_mut() = Some(p.clone());
                                    p.grab_focus();
                                }
                            }
                        }
                        return;
                    }
                }
            }
        });

        hbox
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FocusDirection {
    Next,
    Prev,
}

fn setup_shortcuts(manager: &Rc<TabManager>, window: &gtk4::ApplicationWindow) {
    let controller = gtk4::EventControllerKey::new();
    let mgr = Rc::downgrade(manager);
    let win = window.clone();

    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    controller.connect_key_pressed(move |_, keyval, _, modifier| {
        let ctrl_shift = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        if !modifier.contains(ctrl_shift) {
            return glib::Propagation::Proceed;
        }

        let Some(mgr) = mgr.upgrade() else {
            return glib::Propagation::Proceed;
        };

        match keyval {
            // Ctrl+Shift+T: new tab
            gdk::Key::T => {
                mgr.add_tab(&win);
                glib::Propagation::Stop
            }
            // Ctrl+Shift+W: close focused panel (unsplit or close tab)
            gdk::Key::W => {
                mgr.close_focused(&win);
                glib::Propagation::Stop
            }
            // Ctrl+Shift+E: split horizontal
            gdk::Key::E => {
                mgr.split_focused(gtk4::Orientation::Horizontal, &win);
                glib::Propagation::Stop
            }
            // Ctrl+Shift+O: split vertical
            gdk::Key::O => {
                mgr.split_focused(gtk4::Orientation::Vertical, &win);
                glib::Propagation::Stop
            }
            // Ctrl+Shift+N / Ctrl+Shift+Right: next pane
            gdk::Key::N | gdk::Key::Right => {
                mgr.focus_direction(FocusDirection::Next);
                glib::Propagation::Stop
            }
            // Ctrl+Shift+P / Ctrl+Shift+Left: prev pane
            gdk::Key::P | gdk::Key::Left => {
                mgr.focus_direction(FocusDirection::Prev);
                glib::Propagation::Stop
            }
            // Ctrl+Shift+Tab: next tab
            gdk::Key::ISO_Left_Tab => {
                let nb = &mgr.notebook;
                if nb.n_pages() > 1 {
                    let current = nb.current_page().unwrap_or(0);
                    let next = (current + 1) % nb.n_pages();
                    nb.set_current_page(Some(next));
                }
                glib::Propagation::Stop
            }
            // Ctrl+Shift+1-9: switch to tab N
            k @ (gdk::Key::exclam
            | gdk::Key::at
            | gdk::Key::numbersign
            | gdk::Key::dollar
            | gdk::Key::percent
            | gdk::Key::asciicircum
            | gdk::Key::ampersand
            | gdk::Key::asterisk
            | gdk::Key::parenleft) => {
                let tab_num = match k {
                    gdk::Key::exclam => 0,
                    gdk::Key::at => 1,
                    gdk::Key::numbersign => 2,
                    gdk::Key::dollar => 3,
                    gdk::Key::percent => 4,
                    gdk::Key::asciicircum => 5,
                    gdk::Key::ampersand => 6,
                    gdk::Key::asterisk => 7,
                    gdk::Key::parenleft => 8,
                    _ => return glib::Propagation::Proceed,
                };
                if (tab_num as u32) < mgr.notebook.n_pages() {
                    mgr.notebook.set_current_page(Some(tab_num as u32));
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });

    window.add_controller(controller);
}

const TAB_CSS: &str = r#"
notebook header {
    background-color: #181825;
    padding: 0;
}

notebook header tabs {
    background-color: transparent;
}

notebook header tab {
    background-color: #1e1e2e;
    color: #6c7086;
    padding: 4px 8px;
    margin: 2px 1px 0;
    border-radius: 6px 6px 0 0;
    min-height: 24px;
}

notebook header tab:checked {
    background-color: #313244;
    color: #cdd6f4;
}

notebook header tab:hover:not(:checked) {
    background-color: #262637;
    color: #bac2de;
}

.custerm-tab-close {
    min-width: 16px;
    min-height: 16px;
    padding: 0;
    margin: 0;
    border-radius: 4px;
    color: #6c7086;
}

.custerm-tab-close:hover {
    background-color: #45475a;
    color: #f38ba8;
}
"#;
