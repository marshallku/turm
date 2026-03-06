use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::gdk;
use gtk4::glib;

use custerm_core::config::CustermConfig;

use vte4::prelude::*;

use crate::panel::Panel;
use crate::terminal::TerminalPanel;

pub struct TabManager {
    pub notebook: gtk4::Notebook,
    panels: Rc<RefCell<Vec<Rc<TerminalPanel>>>>,
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
            panels: Rc::new(RefCell::new(Vec::new())),
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

        // Update tab bar visibility
        let panels_ref = manager.panels.clone();
        manager.notebook.connect_page_removed(move |notebook, _, _| {
            notebook.set_show_tabs(panels_ref.borrow().len() > 1);
        });

        // Focus terminal when switching tabs
        let panels_ref = manager.panels.clone();
        manager.notebook.connect_switch_page(move |_, _, page_num| {
            let panels = panels_ref.borrow();
            if let Some(panel) = panels.get(page_num as usize) {
                panel.grab_focus();
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
        let mgr = Rc::downgrade(self);
        let win = window.clone();

        // Each panel gets a weak widget ref so on_exit can find its page
        let overlay_holder: Rc<RefCell<Option<gtk4::Widget>>> = Rc::new(RefCell::new(None));
        let overlay_for_exit = overlay_holder.clone();

        let panel = Rc::new(TerminalPanel::new(&config, move || {
            let overlay = overlay_for_exit.borrow().clone();
            let mgr = mgr.clone();
            let win = win.clone();
            glib::idle_add_local_once(move || {
                let Some(mgr) = mgr.upgrade() else { return };
                let Some(ref widget) = overlay else { return };
                if let Some(page_num) = mgr.notebook.page_num(widget) {
                    mgr.panels.borrow_mut().remove(page_num as usize);
                    mgr.notebook.remove_page(Some(page_num as u32));
                    mgr.notebook.set_show_tabs(mgr.panels.borrow().len() > 1);
                }
                if mgr.panels.borrow().is_empty() {
                    win.close();
                }
            });
        }));

        // Store the widget ref for the exit callback
        *overlay_holder.borrow_mut() = Some(panel.widget().clone());

        // Apply initial background
        if let Some(ref path) = config.background.image {
            let p = std::path::Path::new(path);
            if p.exists() {
                panel.set_background(p);
            }
        }

        let tab_label = make_tab_label(&panel, &self.notebook, &self.panels);

        self.notebook.append_page(panel.widget(), Some(&tab_label));
        self.notebook.set_tab_reorderable(panel.widget(), true);
        self.panels.borrow_mut().push(panel.clone());
        self.notebook.set_show_tabs(self.panels.borrow().len() > 1);

        let page_num = self.notebook.n_pages() - 1;
        self.notebook.set_current_page(Some(page_num));
        panel.grab_focus();
    }

    pub fn close_tab(self: &Rc<Self>, window: &gtk4::ApplicationWindow) {
        if let Some(page_num) = self.notebook.current_page() {
            self.panels.borrow_mut().remove(page_num as usize);
            self.notebook.remove_page(Some(page_num));
            self.notebook.set_show_tabs(self.panels.borrow().len() > 1);

            if self.panels.borrow().is_empty() {
                window.close();
            }
        }
    }

    pub fn active_panel(&self) -> Option<Rc<TerminalPanel>> {
        let idx = self.notebook.current_page()? as usize;
        self.panels.borrow().get(idx).cloned()
    }

    pub fn update_config(&self, config: &CustermConfig) {
        *self.config.borrow_mut() = config.clone();
        for panel in self.panels.borrow().iter() {
            panel.apply_config(config);
        }
    }
}

fn make_tab_label(
    panel: &Rc<TerminalPanel>,
    notebook: &gtk4::Notebook,
    panels: &Rc<RefCell<Vec<Rc<TerminalPanel>>>>,
) -> gtk4::Box {
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

    // Update label on title change
    let label_clone = label.clone();
    panel.terminal.connect_window_title_changed(move |term: &vte4::Terminal| {
        if let Some(title) = term.window_title() {
            label_clone.set_text(&title);
        }
    });

    // Close button
    let widget = panel.widget().clone();
    let nb = notebook.clone();
    let panels_ref = panels.clone();
    close_btn.connect_clicked(move |_| {
        if let Some(page_num) = nb.page_num(&widget) {
            panels_ref.borrow_mut().remove(page_num as usize);
            nb.remove_page(Some(page_num as u32));
            nb.set_show_tabs(panels_ref.borrow().len() > 1);
        }
    });

    hbox
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
            // Ctrl+Shift+W: close current tab
            gdk::Key::W => {
                mgr.close_tab(&win);
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
