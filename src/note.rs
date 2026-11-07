//! A single sticky note window.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::ffi::{self, StyleManager};
use crate::i18n::{gettext, n_};
use crate::note_buffer::NoteBuffer;
use crate::notes::NoteInfo;
use crate::signal::Signal;
use crate::widgets::{confirm, DisableKey, HoverBox};

const UPDATE_DELAY: u32 = 1;

pub const FONT_SCALES: &[(&str, &str, &str)] = &[
    ("small", n_("Small Text"), "small"),
    ("normal", n_("Normal Text"), "medium"),
    ("large", n_("Large Text"), "large"),
    ("larger", n_("Larger Text"), "x-large"),
];

/// (id, untranslated name, color code). Order matters for "cycle".
pub const COLORS: &[(&str, &str, &str)] = &[
    ("red", n_("Red"), "#ff5561"),
    ("green", n_("Green"), "#67ff67"),
    ("blue", n_("Blue"), "#3d9bff"),
    ("yellow", n_("Yellow"), "#f6f907"),
    ("purple", n_("Purple"), "#a553ff"),
    ("teal", n_("Teal"), "#41ffed"),
    ("orange", n_("Orange"), "#ffa939"),
    ("magenta", n_("Magenta"), "#ff7ff7"),
];

/// The color that follows `last` when cycling colors.
pub fn next_cycle_color(last: &str) -> &'static str {
    match COLORS.iter().position(|(id, _, _)| *id == last) {
        Some(i) if i + 1 < COLORS.len() => COLORS[i + 1].0,
        _ => COLORS[0].0,
    }
}

pub struct Note {
    pub window: gtk::Window,
    settings: gio::Settings,
    buffer: NoteBuffer,
    view: gtk::TextView,
    title_box: gtk::Box,
    title_hover: HoverBox,
    title: RefCell<gtk::Widget>,
    title_text: RefCell<String>,
    title_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    style_managers: RefCell<Option<(StyleManager, StyleManager)>>,
    font_handler: RefCell<Option<glib::SignalHandlerId>>,

    showing: Cell<bool>,
    is_pinned: Cell<bool>,
    is_stuck: Cell<bool>,
    changed_timer_id: RefCell<Option<glib::SourceId>>,
    invalid_cache: Cell<bool>,

    x: Cell<i32>,
    y: Cell<i32>,
    height: Cell<i32>,
    width: Cell<i32>,
    color: RefCell<String>,
    cached_text: RefCell<String>,
    extra: serde_json::Map<String, serde_json::Value>,

    weak_self: RefCell<Weak<Note>>,

    /// The note's contents, position or size changed.
    pub update: Signal<()>,
    /// The note was deleted by the user.
    pub removed: Signal<()>,
    /// "New Note" was clicked in this note's title bar.
    pub new_note_requested: Signal<()>,
    /// "Duplicate Note" was picked from this note's menu.
    pub duplicate_requested: Signal<()>,
}

fn icon_button(icon: &str, tooltip: &str, menu: bool) -> gtk::Button {
    let image = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::Button);
    let button: gtk::Button = if menu { gtk::MenuButton::new().upcast() } else { gtk::Button::new() };
    button.set_image(Some(&image));
    button.set_relief(gtk::ReliefStyle::None);
    button.set_widget_name("window-button");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    button
}

impl Note {
    pub fn new(settings: &gio::Settings, parent: &gtk::Window, info: &NoteInfo) -> Rc<Note> {
        let height = info.height.unwrap_or(settings.uint("default-height") as i32);
        let width = info.width.unwrap_or(settings.uint("default-width") as i32);
        let mut color = info.color.clone().unwrap_or_else(|| settings.string("default-color").to_string());

        let window = gtk::Window::builder()
            .skip_taskbar_hint(true)
            .transient_for(parent)
            .type_hint(gdk::WindowTypeHint::Utility)
            .default_height(height)
            .default_width(width)
            .resizable(true)
            .deletable(false)
            .name("sticky-note")
            .build();

        if color == "cycle" {
            let last = settings.string("last-color");
            let last = if last.is_empty() { color.clone() } else { last.to_string() };
            color = next_cycle_color(&last).to_string();
            let _ = settings.set_string("last-color", &color);
        }

        window.style_context().add_class(&color);

        if settings.boolean("desktop-window-state") {
            window.stick();
        }

        // title bar
        let title_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_bar.set_height_request(30);
        title_bar.set_widget_name("title-bar");

        let color_button = icon_button("sticky-color", &gettext("Note Color"), true);
        title_bar.pack_start(&color_button, false, false, 0);

        // used to show the edit title icon when the title is hovered
        let title_hover = HoverBox::new();
        title_bar.pack_start(&title_hover.event_box, true, true, 4);

        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_hover.add(&title_box);
        let title_text = info.title.clone().unwrap_or_default();
        let title = gtk::Label::new(Some(&title_text));
        title.set_margin_top(4);
        title.set_widget_name("title");
        title_box.pack_start(&title, false, false, 0);
        let title_style_manager = StyleManager::new(&title_box);

        let edit_title_button = icon_button("sticky-edit", &gettext("Rename"), false);
        title_box.pack_start(&edit_title_button, false, false, 0);
        title_hover.set_child_widget(&edit_title_button);

        let close_button = icon_button("sticky-delete", &gettext("Delete Note"), false);
        title_bar.pack_end(&close_button, false, false, 0);

        let add_button = icon_button("sticky-add", &gettext("New Note"), false);
        title_bar.pack_end(&add_button, false, false, 0);

        let text_button = icon_button("sticky-text", &gettext("Format"), true);
        title_bar.pack_end(&text_button, false, false, 20);

        window.set_titlebar(Some(&title_bar));

        // buffer
        let buffer = NoteBuffer::new();

        // text view
        let view = gtk::TextView::with_buffer(buffer.text_buffer());
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_populate_all(true);
        buffer.set_view(&view);
        let spell_checker = ffi::setup_spell_checking(&view);
        settings.bind("inline-spell-check", &spell_checker, "inline-spell-checking").flags(gio::SettingsBindFlags::GET).build();
        view.set_left_margin(10);
        view.set_right_margin(10);
        view.set_top_margin(10);
        view.set_bottom_margin(10);
        let view_style_manager = StyleManager::new(&view);

        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        window.add(&scroll);
        scroll.add(&view);

        let note = Rc::new(Note {
            window: window.clone(),
            settings: settings.clone(),
            buffer: buffer.clone(),
            view: view.clone(),
            title_box,
            title_hover,
            title: RefCell::new(title.upcast()),
            title_text: RefCell::new(title_text),
            title_handlers: RefCell::default(),
            style_managers: RefCell::new(Some((title_style_manager, view_style_manager))),
            font_handler: RefCell::default(),
            showing: Cell::new(false),
            is_pinned: Cell::new(false),
            is_stuck: Cell::new(false),
            changed_timer_id: RefCell::default(),
            invalid_cache: Cell::new(false),
            x: Cell::new(info.x),
            y: Cell::new(info.y),
            height: Cell::new(height),
            width: Cell::new(width),
            color: RefCell::new(color),
            cached_text: RefCell::new(info.text.clone()),
            extra: info.extra.clone(),
            weak_self: RefCell::default(),
            update: Signal::default(),
            removed: Signal::default(),
            new_note_requested: Signal::default(),
            duplicate_requested: Signal::default(),
        });
        *note.weak_self.borrow_mut() = Rc::downgrade(&note);

        // Right click anywhere on the title bar shows the note menu.
        let title_widgets: [gtk::Widget; 5] = [
            title_bar.clone().upcast(),
            color_button.clone().upcast(),
            edit_title_button.clone().upcast(),
            close_button.clone().upcast(),
            add_button.clone().upcast(),
        ];
        for widget in title_widgets.iter().chain(std::iter::once(text_button.upcast_ref())) {
            let weak = note.weak();
            widget.connect_button_press_event(move |_, event| match weak.upgrade() {
                Some(n) => n.on_title_click(event),
                None => glib::Propagation::Proceed,
            });
        }

        let weak = note.weak();
        edit_title_button.connect_clicked(move |_| {
            if let Some(n) = weak.upgrade() {
                n.set_title();
            }
        });
        let weak = note.weak();
        close_button.connect_clicked(move |_| {
            if let Some(n) = weak.upgrade() {
                n.remove();
            }
        });
        let weak = note.weak();
        add_button.connect_clicked(move |_| {
            if let Some(n) = weak.upgrade() {
                n.new_note_requested.emit(&());
            }
        });

        let weak = note.weak();
        view.connect_populate_popup(move |_, popup| {
            if let (Some(n), Ok(menu)) = (weak.upgrade(), popup.clone().downcast::<gtk::Menu>()) {
                n.add_context_menu_items(&menu, false);
            }
        });
        let weak = note.weak();
        view.connect_key_press_event(move |_, event| match weak.upgrade() {
            Some(n) => n.on_key_press(event),
            None => glib::Propagation::Proceed,
        });

        buffer.set_from_internal_markup(&note.cached_text.borrow());
        let weak = note.weak();
        buffer.connect_content_changed(move || {
            if let Some(n) = weak.upgrade() {
                n.queue_update(true);
            }
        });

        let weak = note.weak();
        let font_handler = settings.connect_changed(Some("font"), move |_, _| {
            if let Some(n) = weak.upgrade() {
                n.set_font();
            }
        });
        *note.font_handler.borrow_mut() = Some(font_handler);
        note.set_font();

        note.create_format_menu(color_button.downcast_ref().unwrap(), text_button.downcast_ref().unwrap());

        let weak = note.weak();
        window.connect_configure_event(move |_, _| {
            if let Some(n) = weak.upgrade() {
                n.on_size_position_changed();
            }
            false
        });
        let weak = note.weak();
        window.connect_show(move |_| {
            if let Some(n) = weak.upgrade() {
                n.showing.set(true);
            }
        });
        let weak = note.weak();
        window.connect_window_state_event(move |_, event| {
            if let Some(n) = weak.upgrade() {
                n.is_stuck.set(event.new_window_state().contains(gdk::WindowState::STICKY));
                // for some reason, the ABOVE flag is never actually being set, even when it should be
            }
            glib::Propagation::Proceed
        });

        let weak = note.weak();
        window.connect_destroy(move |_| {
            if let Some(n) = weak.upgrade() {
                n.on_destroy();
            }
        });

        window.move_(note.x.get(), note.y.get());
        window.show_all();

        note
    }

    fn weak(&self) -> Weak<Note> {
        self.weak_self.borrow().clone()
    }

    fn on_destroy(&self) {
        // The style managers must go while their widgets still exist.
        self.style_managers.borrow_mut().take();
        if let Some(id) = self.font_handler.borrow_mut().take() {
            self.settings.disconnect(id);
        }
        if let Some(id) = self.changed_timer_id.borrow_mut().take() {
            id.remove();
        }
    }

    pub fn destroy(&self) {
        unsafe { self.window.destroy() };
    }

    fn on_size_position_changed(&self) {
        if self.showing.get() {
            self.showing.set(false);
            return;
        }

        let (new_x, new_y) = self.window.position();
        let (new_width, new_height) = self.window.size();
        if self.x.get() == new_x && self.y.get() == new_y && self.height.get() == new_height && self.width.get() == new_width {
            return;
        }

        self.x.set(new_x);
        self.y.set(new_y);
        self.height.set(new_height);
        self.width.set(new_width);

        self.queue_update(false);
    }

    fn on_key_press(&self, event: &gdk::EventKey) -> glib::Propagation {
        use gdk::keys::constants as key;

        let state = event.state();
        let keyval = event.keyval();
        if state.contains(gdk::ModifierType::CONTROL_MASK) {
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                if keyval == key::Up {
                    self.buffer.shift(true);
                    return glib::Propagation::Stop;
                } else if keyval == key::Down {
                    self.buffer.shift(false);
                    return glib::Propagation::Stop;
                }
            }

            let tag = match keyval {
                k if k == key::z => {
                    self.buffer.undo();
                    return glib::Propagation::Stop;
                }
                k if k == key::y => {
                    self.buffer.redo();
                    return glib::Propagation::Stop;
                }
                k if k == key::e => {
                    self.buffer.toggle_checklist();
                    return glib::Propagation::Stop;
                }
                k if k == key::l => {
                    self.buffer.toggle_bullets();
                    return glib::Propagation::Stop;
                }
                k if k == key::b => "bold",
                k if k == key::i => "italic",
                k if k == key::f => "monospace",
                k if k == key::u => "underline",
                k if k == key::h => "header",
                k if k == key::k => "strikethrough",
                k if k == key::g => "highlight",
                k if k == key::_2 => "small",
                k if k == key::_3 => "normal",
                k if k == key::_4 => "large",
                k if k == key::_5 => "larger",
                _ => return glib::Propagation::Proceed,
            };
            self.buffer.tag_selection(tag);
            return glib::Propagation::Stop;
        } else if keyval == key::Return || keyval == key::ISO_Enter || keyval == key::KP_Enter {
            return self.buffer.on_return();
        }

        glib::Propagation::Proceed
    }

    fn on_title_click(&self, event: &gdk::EventButton) -> glib::Propagation {
        if event.button() == 3 {
            let menu = gtk::Menu::new();
            menu.set_attach_widget(Some(&self.window));
            self.add_context_menu_items(&menu, true);
            menu.popup_at_pointer(Some(event));
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    }

    pub fn restore(&self, time: u32) {
        let time = if time == 0 { gtk::current_event_time() } else { time };
        self.window.show();
        self.window.present_with_time(time);
        self.window.move_(self.x.get(), self.y.get());
    }

    pub fn hide(&self) {
        self.window.hide();
    }

    pub fn is_active(&self) -> bool {
        self.window.is_active()
    }

    fn queue_update(&self, invalidate_cache: bool) {
        // A pending text change must survive a later move or resize, so only ever set the flag here;
        // get_info() clears it once the text has been read back.
        if invalidate_cache {
            self.invalid_cache.set(true);
        }

        if let Some(id) = self.changed_timer_id.borrow_mut().take() {
            id.remove();
        }

        let weak = self.weak();
        let id = glib::timeout_add_seconds_local(UPDATE_DELAY, move || {
            if let Some(n) = weak.upgrade() {
                n.changed_timer_id.borrow_mut().take();
                n.update.emit(&());
            }
            glib::ControlFlow::Break
        });
        *self.changed_timer_id.borrow_mut() = Some(id);
    }

    fn title_string(&self) -> String {
        let title = self.title.borrow();
        if let Some(entry) = title.downcast_ref::<gtk::Entry>() {
            entry.text().to_string()
        } else if let Some(label) = title.downcast_ref::<gtk::Label>() {
            label.text().to_string()
        } else {
            String::new()
        }
    }

    pub fn get_info(&self) -> NoteInfo {
        if self.invalid_cache.get() {
            *self.cached_text.borrow_mut() = self.buffer.get_internal_markup();
            self.invalid_cache.set(false);
        }

        NoteInfo {
            x: self.x.get(),
            y: self.y.get(),
            height: Some(self.height.get()),
            width: Some(self.width.get()),
            color: Some(self.color.borrow().clone()),
            title: Some(self.title_string()),
            text: self.cached_text.borrow().clone(),
            extra: self.extra.clone(),
        }
    }

    fn menu_item(&self, label: &str, f: impl Fn(&Note) + 'static) -> gtk::MenuItem {
        let item = gtk::MenuItem::with_label(label);
        item.set_visible(true);
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(n) = weak.upgrade() {
                f(&n);
            }
        });
        item
    }

    fn separator() -> gtk::SeparatorMenuItem {
        let sep = gtk::SeparatorMenuItem::new();
        sep.set_visible(true);
        sep
    }

    fn add_context_menu_items(&self, popup: &gtk::Menu, is_title: bool) {
        if !is_title {
            popup.append(&Self::separator());

            let undo_item = self.menu_item(&gettext("undo"), |n| n.buffer.undo());
            undo_item.set_sensitive(self.buffer.can_undo());
            popup.append(&undo_item);

            let redo_item = self.menu_item(&gettext("redo"), |n| n.buffer.redo());
            redo_item.set_sensitive(self.buffer.can_redo());
            popup.append(&redo_item);

            popup.append(&Self::separator());
        }

        let label = if self.title_string().is_empty() { gettext("Set Title") } else { gettext("Edit Title") };
        popup.append(&self.menu_item(&label, |n| n.set_title()));
        popup.append(&self.menu_item(&gettext("Duplicate Note"), |n| n.duplicate_requested.emit(&())));
        popup.append(&self.menu_item(&gettext("Delete Note"), |n| n.remove()));

        if is_title {
            popup.append(&Self::separator());

            let stick_item = if self.is_stuck.get() {
                self.menu_item(&gettext("Only on This Workspace"), |n| n.window.unstick())
            } else {
                self.menu_item(&gettext("Always on Visible Workspace"), |n| n.window.stick())
            };
            popup.append(&stick_item);

            let pin_item = gtk::CheckMenuItem::with_label(&gettext("Always on Top"));
            pin_item.set_active(self.is_pinned.get());
            pin_item.set_visible(true);
            let weak = self.weak();
            pin_item.connect_activate(move |_| {
                if let Some(n) = weak.upgrade() {
                    n.window.set_keep_above(!n.is_pinned.get());
                    n.is_pinned.set(!n.is_pinned.get());
                }
            });
            popup.append(&pin_item);
        }
    }

    fn markup_item(&self, label: &str, markup: &str, f: impl Fn(&Note) + 'static) -> gtk::MenuItem {
        let item = self.menu_item(label, f);
        if let Some(child) = item.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            child.set_markup(markup);
        }
        item
    }

    fn create_format_menu(&self, color_button: &gtk::MenuButton, text_button: &gtk::MenuButton) {
        let menu = gtk::Menu::new();

        let mut colors: Vec<(&str, String, &str)> =
            COLORS.iter().map(|(id, name, code)| (*id, gettext(name), *code)).collect();
        colors.sort_by(|a, b| a.1.cmp(&b.1));
        for (color, name, code) in colors {
            let markup = format!("<span foreground='{code}'>\u{25A6}</span>  {}", glib::markup_escape_text(&name));
            let color = color.to_string();
            menu.append(&self.markup_item(&name, &markup, move |n| n.set_color(&color)));
        }

        color_button.set_popup(Some(&menu));

        let menu = gtk::Menu::new();

        let formats: [(&str, &str, &str); 7] = [
            (n_("Bold"), "<b>%s</b>", "bold"),
            (n_("Italic"), "<i>%s</i>", "italic"),
            (n_("Fixed Width"), "<tt>%s</tt>", "monospace"),
            (n_("Underline"), "<u>%s</u>", "underline"),
            (n_("Strikethrough"), "<s>%s</s>", "strikethrough"),
            (n_("Highlight"), "<span background='yellow' foreground='black'>%s</span>", "highlight"),
            (n_("Header"), "<span size='large'>%s</span>", "header"),
        ];
        for (label, markup, tag) in formats {
            let label = gettext(label);
            let markup = markup.replace("%s", &glib::markup_escape_text(&label));
            menu.append(&self.markup_item(&label, &markup, move |n| n.buffer.tag_selection(tag)));
        }

        menu.append(&Self::separator());

        for (scale_id, scale_name, scale_value) in FONT_SCALES {
            let name = gettext(scale_name);
            let markup = format!("<span size='{scale_value}'>{}</span>", glib::markup_escape_text(&name));
            menu.append(&self.markup_item(&name, &markup, move |n| n.buffer.tag_selection(scale_id)));
        }

        menu.append(&Self::separator());

        menu.append(&self.menu_item(&format!("\u{25A2} {}", gettext("Toggle Checklist")), |n| n.buffer.toggle_checklist()));
        menu.append(&self.menu_item(&format!("\u{25CF} {}", gettext("Toggle Bullets")), |n| n.buffer.toggle_bullets()));

        text_button.set_popup(Some(&menu));
    }

    fn set_color(&self, color: &str) {
        if *self.color.borrow() == color {
            return;
        }

        let context = self.window.style_context();
        context.remove_class(&self.color.borrow());
        context.add_class(color);
        *self.color.borrow_mut() = color.to_string();

        self.update.emit(&());
    }

    fn set_font(&self) {
        let font = self.settings.string("font");
        if let Some((title, view)) = self.style_managers.borrow().as_ref() {
            title.set_from_pango_font_string(&font);
            view.set_from_pango_font_string(&font);
        }
    }

    pub fn remove(&self) {
        // this is ugly but I'm not sure how to make it look better :)
        if self.settings.boolean("disable-delete-confirm")
            || (self.title_string().is_empty() && self.buffer.char_count() == 0)
            || confirm(
                &gettext("Delete Note"),
                &gettext("Are you sure you want to remove this note?"),
                Some(&self.window),
                Some(DisableKey { settings: &self.settings, key: "disable-delete-confirm" }),
            )
        {
            self.removed.emit(&());
            self.destroy();
        }
    }

    fn set_title(&self) {
        let text = self.title_string();
        *self.title_text.borrow_mut() = text.clone();
        self.title_box.remove(&*self.title.borrow());

        let entry = gtk::Entry::new();
        entry.set_text(&text);
        entry.set_visible(true);
        entry.set_widget_name("title");
        self.title_box.pack_start(&entry, false, false, 0);

        let weak = self.weak();
        let key_id = entry.connect_key_press_event(move |_, event| match weak.upgrade() {
            Some(n) => n.save_title(Some(event)),
            None => glib::Propagation::Proceed,
        });
        let weak = self.weak();
        let focus_id = entry.connect_focus_out_event(move |_, _| match weak.upgrade() {
            Some(n) => n.save_title(None),
            None => glib::Propagation::Proceed,
        });
        *self.title_handlers.borrow_mut() = vec![key_id, focus_id];

        self.title_box.reorder_child(&entry, 0);
        self.title_hover.disable();

        entry.grab_focus();
        *self.title.borrow_mut() = entry.upcast();
    }

    /// `event` is None for focus-out.
    fn save_title(&self, event: Option<&gdk::EventKey>) -> glib::Propagation {
        use gdk::keys::constants as key;

        let save = match event {
            None => true,
            Some(e) => {
                let k = e.keyval();
                if k == key::Return || k == key::ISO_Enter || k == key::KP_Enter {
                    true
                } else if k != key::Escape {
                    return glib::Propagation::Proceed;
                } else {
                    false
                }
            }
        };

        let old = self.title.borrow().clone();
        for id in self.title_handlers.borrow_mut().drain(..) {
            old.disconnect(id);
        }

        if save {
            *self.title_text.borrow_mut() = self.title_string();
        }

        self.view.grab_focus();

        self.title_box.remove(&old);

        let label = gtk::Label::new(Some(&self.title_text.borrow()));
        label.set_visible(true);
        label.set_widget_name("title");
        self.title_box.pack_start(&label, false, false, 0);
        self.title_box.reorder_child(&label, 0);
        *self.title.borrow_mut() = label.upcast();
        self.title_hover.enable();

        if save {
            self.update.emit(&());
        }

        glib::Propagation::Stop
    }

    pub fn present_with_time(&self, time: u32) {
        self.window.present_with_time(time);
    }
}
