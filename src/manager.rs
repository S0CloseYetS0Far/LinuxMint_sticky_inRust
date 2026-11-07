//! The "Notes" manager window: groups on the left, note previews on the
//! right, search, backups, import/export.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::app::App;
use crate::ffi::StyleManager;
use crate::file_handler::FileHandler;
use crate::i18n::gettext;
use crate::note_buffer::NoteBuffer;
use crate::notes::NoteInfo;
use crate::util::clean_text;
use crate::widgets::HoverBox;

fn note_targets() -> Vec<gtk::TargetEntry> {
    vec![gtk::TargetEntry::new("note-entry", gtk::TargetFlags::SAME_APP, 1)]
}

pub fn data_dir() -> std::path::PathBuf {
    std::env::var_os("STICKY_DATA_DIR").map(Into::into).unwrap_or_else(|| "/usr/share/sticky".into())
}

/// A note shown in the manager.
#[derive(Clone)]
struct PreviewNote {
    info: NoteInfo,
    group_name: String,
    /// Position in its group's note list.
    index: usize,
}

impl PreviewNote {
    fn title(&self) -> String {
        match self.info.title.as_deref() {
            None | Some("") => gettext("Untitled"),
            Some(t) => t.to_string(),
        }
    }
}

/// The 150x150 preview of a note.
fn note_entry(item: &PreviewNote, settings: &gio::Settings) -> gtk::Widget {
    let entry = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let style_manager = Rc::new(StyleManager::new(&entry));

    let title_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    title_bar.set_widget_name("title-bar");
    let title = gtk::Label::new(Some(&item.title()));
    title.set_widget_name("title");
    title.set_margin_top(5);
    title.set_margin_bottom(5);
    title.set_ellipsize(pango::EllipsizeMode::End);
    title_bar.pack_start(&title, false, false, 0);
    entry.pack_start(&title_bar, false, false, 0);

    let buffer = NoteBuffer::new();
    let text = gtk::TextView::with_buffer(buffer.text_buffer());
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_populate_all(true);
    text.set_sensitive(false);
    text.set_margin_start(10);
    text.set_margin_end(10);
    text.set_margin_top(10);
    text.set_margin_bottom(10);
    buffer.set_view(&text);
    buffer.set_from_internal_markup(&item.info.text);
    entry.pack_start(&text, true, true, 0);

    // Keep the buffer state alive as long as the view.
    let keep = RefCell::new(Some(buffer));
    text.connect_destroy(move |_| {
        keep.borrow_mut().take();
    });

    let set_font = {
        let style_manager = style_manager.clone();
        move |settings: &gio::Settings| style_manager.set_from_pango_font_string(&settings.string("font"))
    };
    set_font(settings);
    let font_handler = settings.connect_changed(Some("font"), move |s, _| set_font(s));
    let settings = settings.clone();
    let font_handler = Cell::new(Some(font_handler));
    entry.connect_destroy(move |_| {
        if let Some(id) = font_handler.take() {
            settings.disconnect(id);
        }
    });

    entry.set_tooltip_text(Some(&item.title()));

    // The preview always has the same size; anything that does not fit is clipped.
    let clip = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    clip.set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
    clip.set_size_request(150, 150);
    clip.set_min_content_width(150);
    clip.set_min_content_height(150);
    clip.set_valign(gtk::Align::Start);
    clip.set_halign(gtk::Align::Center);
    clip.add(&entry);
    if let Some(viewport) = clip.child().and_then(|c| c.downcast::<gtk::Viewport>().ok()) {
        viewport.set_shadow_type(gtk::ShadowType::None);
    }
    clip.show_all();
    clip.upcast()
}

struct GroupEntry {
    row: gtk::ListBoxRow,
    name: RefCell<String>,
    file_handler: Rc<FileHandler>,
    hoverbox: HoverBox,
    menu: gtk::Menu,
    new_item: gtk::MenuItem,
    remove_item: gtk::MenuItem,
    content: RefCell<Option<gtk::Box>>,
    entry: RefCell<Option<(gtk::Entry, Vec<glib::SignalHandlerId>)>>,
    weak_self: RefCell<Weak<GroupEntry>>,
}

impl GroupEntry {
    fn new(name: &str, file_handler: &Rc<FileHandler>) -> Rc<GroupEntry> {
        let row = gtk::ListBoxRow::new();
        row.set_height_request(35);

        let hoverbox = HoverBox::new();
        row.add(&hoverbox.event_box);

        let menu = gtk::Menu::new();
        menu.set_attach_widget(Some(&row));
        let new_item = gtk::MenuItem::with_label(&gettext("New"));
        new_item.set_visible(true);
        menu.append(&new_item);
        let sep = gtk::SeparatorMenuItem::new();
        sep.set_visible(true);
        menu.append(&sep);
        let edit_item = gtk::MenuItem::with_label(&gettext("Edit"));
        edit_item.set_visible(true);
        menu.append(&edit_item);
        let remove_item = gtk::MenuItem::with_label(&gettext("Remove"));
        remove_item.set_visible(true);
        menu.append(&remove_item);

        let group = Rc::new(GroupEntry {
            row,
            name: RefCell::new(name.to_string()),
            file_handler: file_handler.clone(),
            hoverbox,
            menu,
            new_item,
            remove_item,
            content: RefCell::default(),
            entry: RefCell::default(),
            weak_self: RefCell::default(),
        });
        *group.weak_self.borrow_mut() = Rc::downgrade(&group);

        let weak = group.weak();
        edit_item.connect_activate(move |_| {
            if let Some(g) = weak.upgrade() {
                g.edit_group_name();
            }
        });
        let weak = group.weak();
        group.remove_item.connect_activate(move |_| {
            if let Some(g) = weak.upgrade() {
                g.remove_group();
            }
        });

        let weak = group.weak();
        group.row.connect_popup_menu(move |row| {
            if let Some(g) = weak.upgrade() {
                g.menu.popup_at_widget(row, gdk::Gravity::Center, gdk::Gravity::Center, None);
            }
            true
        });
        let weak = group.weak();
        group.row.connect_button_press_event(move |_, event| {
            if let Some(g) = weak.upgrade() {
                if event.button() == 3 {
                    g.menu.popup_at_pointer(Some(event));
                }
            }
            glib::Propagation::Proceed
        });
        let weak = group.weak();
        group.row.connect_key_press_event(move |_, event| {
            if event.keyval() == gdk::keys::constants::Delete {
                if let Some(g) = weak.upgrade() {
                    g.remove_group();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });

        group.generate_content();
        group
    }

    fn weak(&self) -> Weak<GroupEntry> {
        self.weak_self.borrow().clone()
    }

    fn name(&self) -> String {
        self.name.borrow().clone()
    }

    fn generate_content(&self) {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        self.hoverbox.add(&content);

        let label = gtk::Label::new(Some(&self.name()));
        label.set_halign(gtk::Align::Start);
        label.set_margin_start(5);
        content.pack_start(&label, true, true, 5);

        let image = gtk::Image::from_icon_name(Some("xsi-document-edit-symbolic"), gtk::IconSize::Button);
        let button = gtk::Button::new();
        button.set_image(Some(&image));
        button.set_relief(gtk::ReliefStyle::None);
        button.set_widget_name("manager-group-edit-button");
        content.pack_end(&button, false, false, 2);
        let weak = self.weak();
        button.connect_clicked(move |_| {
            if let Some(g) = weak.upgrade() {
                g.edit_group_name();
            }
        });
        self.hoverbox.set_child_widget(&button);

        content.show_all();
        *self.content.borrow_mut() = Some(content);
    }

    fn remove_group(&self) {
        self.file_handler.remove_group(&self.name());
    }

    fn edit_group_name(&self) {
        if let Some(content) = self.content.borrow_mut().take() {
            unsafe { content.destroy() };
        }

        let entry = gtk::Entry::new();
        entry.set_visible(true);
        entry.set_text(&self.name());
        self.hoverbox.add(&entry);

        let weak = self.weak();
        let activate_id = entry.connect_activate(move |_| {
            if let Some(g) = weak.upgrade() {
                g.maybe_done();
            }
        });
        let weak = self.weak();
        let focus_id = entry.connect_focus_out_event(move |_, _| {
            if let Some(g) = weak.upgrade() {
                g.maybe_done();
            }
            glib::Propagation::Proceed
        });
        let weak = self.weak();
        let key_id = entry.connect_key_press_event(move |_, event| {
            if event.keyval() != gdk::keys::constants::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(g) = weak.upgrade() {
                g.clean_up();
            }
            glib::Propagation::Stop
        });
        self.hoverbox.disable();

        entry.grab_focus();
        *self.entry.borrow_mut() = Some((entry, vec![activate_id, focus_id, key_id]));
    }

    fn maybe_done(&self) {
        let Some(group_name) = self.entry.borrow().as_ref().map(|(e, _)| e.text().to_string()) else { return };
        let old_name = self.name();
        if !group_name.is_empty() && group_name != old_name {
            *self.name.borrow_mut() = group_name.clone();
            self.file_handler.change_group_name(&old_name, &group_name);
        }

        self.clean_up();
    }

    fn clean_up(&self) {
        let Some((entry, handlers)) = self.entry.borrow_mut().take() else { return };
        for id in handlers {
            entry.disconnect(id);
        }
        unsafe { entry.destroy() };

        self.generate_content();
        self.hoverbox.enable();
    }

    fn set_can_remove(&self, can_remove: bool) {
        self.remove_item.set_sensitive(can_remove);
    }
}

pub struct NotesManager {
    app: Weak<App>,
    file_handler: Rc<FileHandler>,
    settings: gio::Settings,
    pub window: gtk::Window,
    group_list: gtk::ListBox,
    note_view: gtk::FlowBox,
    search_bar: gtk::SearchBar,
    search_box: gtk::SearchEntry,
    remove_note_button: gtk::Button,
    duplicate_note_button: gtk::Button,
    entry_box: gtk::Box,
    remove_group_item: gtk::MenuItem,
    groups: RefCell<Vec<Rc<GroupEntry>>>,
    previews: RefCell<Vec<(gtk::FlowBoxChild, PreviewNote)>>,
    dragged_note: RefCell<Option<PreviewNote>>,
    weak_self: RefCell<Weak<NotesManager>>,
}

impl NotesManager {
    pub fn new(app: &Rc<App>, file_handler: &Rc<FileHandler>) -> Rc<NotesManager> {
        let builder = gtk::Builder::new();
        builder.set_translation_domain(Some(crate::i18n::DOMAIN));
        if let Err(e) = builder.add_from_file(data_dir().join("manager.ui")) {
            panic!("could not load manager.ui: {e}");
        }
        let obj = |name: &str| builder.object::<glib::Object>(name).unwrap_or_else(|| panic!("manager.ui has no {name}"));

        let window: gtk::Window = obj("main_window").downcast().unwrap();
        let group_list: gtk::ListBox = obj("group_list").downcast().unwrap();
        let note_view: gtk::FlowBox = obj("note_view").downcast().unwrap();
        let search_toggle: gtk::ToggleButton = obj("search_toggle_button").downcast().unwrap();
        let search_bar: gtk::SearchBar = obj("search_bar").downcast().unwrap();
        let search_box: gtk::SearchEntry = obj("search_box").downcast().unwrap();
        let new_note_button: gtk::Button = obj("new_note").downcast().unwrap();
        let remove_note_button: gtk::Button = obj("remove_note").downcast().unwrap();
        let duplicate_note_button: gtk::Button = obj("duplicate_note").downcast().unwrap();
        let entry_box: gtk::Box = obj("group_name_entry_box").downcast().unwrap();
        let menu_button: gtk::MenuButton = obj("menu_button").downcast().unwrap();

        search_toggle.bind_property("active", &search_bar, "search-mode-enabled").bidirectional().build();

        let main_menu = gtk::Menu::new();
        let remove_group_item = gtk::MenuItem::with_label(&gettext("Remove Group"));

        let manager = Rc::new(NotesManager {
            app: Rc::downgrade(app),
            file_handler: file_handler.clone(),
            settings: app.settings().clone(),
            window: window.clone(),
            group_list: group_list.clone(),
            note_view: note_view.clone(),
            search_bar,
            search_box: search_box.clone(),
            remove_note_button: remove_note_button.clone(),
            duplicate_note_button: duplicate_note_button.clone(),
            entry_box: entry_box.clone(),
            remove_group_item: remove_group_item.clone(),
            groups: RefCell::default(),
            previews: RefCell::default(),
            dragged_note: RefCell::default(),
            weak_self: RefCell::default(),
        });
        *manager.weak_self.borrow_mut() = Rc::downgrade(&manager);

        let weak = manager.weak();
        file_handler.group_changed.connect(move |group_name| {
            if let Some(m) = weak.upgrade() {
                m.on_list_changed(group_name);
            }
        });
        let weak = manager.weak();
        file_handler.lists_changed.connect(move |_| {
            if let Some(m) = weak.upgrade() {
                m.generate_group_list();
            }
        });

        let weak = manager.weak();
        note_view.connect_child_activated(move |_, _| {
            if let Some(m) = weak.upgrade() {
                m.on_note_activated();
            }
        });
        let weak = manager.weak();
        note_view.connect_selected_children_changed(move |_| {
            if let Some(m) = weak.upgrade() {
                m.on_selected_notes_changed();
            }
        });

        let weak_app = Rc::downgrade(app);
        new_note_button.connect_clicked(move |_| {
            if let Some(app) = weak_app.upgrade() {
                app.new_note(None);
            }
        });
        let weak = manager.weak();
        remove_note_button.connect_clicked(move |_| {
            if let Some(m) = weak.upgrade() {
                m.remove_note();
            }
        });
        let weak = manager.weak();
        duplicate_note_button.connect_clicked(move |_| {
            if let Some(m) = weak.upgrade() {
                m.duplicate_note();
            }
        });

        let weak = manager.weak();
        search_box.connect_search_changed(move |_| {
            if let Some(m) = weak.upgrade() {
                m.on_search_changed();
            }
        });

        entry_box.drag_dest_set(gtk::DestDefaults::MOTION | gtk::DestDefaults::HIGHLIGHT, &note_targets(), gdk::DragAction::MOVE);
        let weak = manager.weak();
        entry_box.connect_drag_drop(move |_, context, _, _, time| {
            if let Some(m) = weak.upgrade() {
                m.handle_new_group_drop();
            }
            context.drag_finish(true, false, time);
            true
        });

        // main menu
        let item = manager.menu_item(&gettext("New Group"), |m| m.new_group());
        main_menu.append(&item);

        let weak = manager.weak();
        remove_group_item.connect_activate(move |_| {
            if let Some(m) = weak.upgrade() {
                m.remove_group();
            }
        });
        main_menu.append(&remove_group_item);

        main_menu.append(&gtk::SeparatorMenuItem::new());

        main_menu.append(&manager.menu_item(&gettext("Back Up"), |m| m.file_handler.save_backup()));
        main_menu.append(&manager.menu_item(&gettext("Restore Backups..."), |m| m.file_handler.restore_backup(&m.window)));

        main_menu.append(&gtk::SeparatorMenuItem::new());

        main_menu.append(&manager.menu_item(&gettext("Import..."), |m| m.file_handler.import_notes(&m.window)));
        main_menu.append(&manager.menu_item(&gettext("Export..."), |m| m.file_handler.export_notes(&m.window)));

        main_menu.append(&gtk::SeparatorMenuItem::new());

        main_menu.append(&manager.menu_item(&gettext("Preferences"), |m| m.with_app(|a| a.open_settings_window())));
        main_menu.append(&manager.menu_item(&gettext("Keyboard Shortcuts"), |m| m.with_app(|a| a.open_keyboard_shortcuts())));

        let about = manager.menu_item(&gettext("About"), |m| m.with_app(|a| a.open_about()));
        let accel_group = gtk::AccelGroup::new();
        window.add_accel_group(&accel_group);
        let (key, modifier) = gtk::accelerator_parse("F1");
        about.add_accelerator("activate", &accel_group, key, modifier, gtk::AccelFlags::VISIBLE);
        main_menu.append(&about);

        let (key, modifier) = gtk::accelerator_parse("<Control>f");
        let weak = manager.weak();
        accel_group.connect_accel_group(key, modifier, gtk::AccelFlags::VISIBLE, move |_, _, _, _| {
            if let Some(m) = weak.upgrade() {
                m.search_bar.set_search_mode(true);
            }
            true
        });

        main_menu.show_all();
        menu_button.set_popup(Some(&main_menu));

        manager.generate_group_list();

        window.show_all();

        manager.generate_previews();

        let weak = manager.weak();
        group_list.connect_row_selected(move |_, _| {
            if let Some(m) = weak.upgrade() {
                m.on_group_selected();
            }
        });
        let weak = manager.weak();
        group_list.connect_button_press_event(move |_, _| {
            if let Some(m) = weak.upgrade() {
                m.with_app(|a| a.present_notes(gtk::current_event_time()));
            }
            glib::Propagation::Proceed
        });
        let weak = manager.weak();
        manager.settings.connect_changed(Some("active-group"), move |settings, key| {
            if let Some(m) = weak.upgrade() {
                m.select_group(&settings.string(key));
            }
        });

        manager
    }

    fn weak(&self) -> Weak<NotesManager> {
        self.weak_self.borrow().clone()
    }

    fn with_app(&self, f: impl FnOnce(&Rc<App>)) {
        if let Some(app) = self.app.upgrade() {
            f(&app);
        }
    }

    fn menu_item(&self, label: &str, f: impl Fn(&NotesManager) + 'static) -> gtk::MenuItem {
        let item = gtk::MenuItem::with_label(label);
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(m) = weak.upgrade() {
                f(&m);
            }
        });
        item
    }

    fn on_list_changed(&self, group_name: &str) {
        if self.get_current_group().as_deref() == Some(group_name) {
            self.generate_previews();
        }
    }

    fn generate_group_list(&self) {
        let active = self.settings.string("active-group").to_string();

        let old_groups = std::mem::take(&mut *self.groups.borrow_mut());
        for group in old_groups {
            self.group_list.remove(&group.row);
        }

        let mut name = None;
        for group_name in self.file_handler.get_note_group_names() {
            if active == group_name {
                name = Some(group_name.clone());
            }
            let group = GroupEntry::new(&group_name, &self.file_handler);
            group.row.drag_dest_set(gtk::DestDefaults::MOTION | gtk::DestDefaults::HIGHLIGHT, &note_targets(), gdk::DragAction::MOVE);
            let weak = self.weak();
            let weak_group = Rc::downgrade(&group);
            group.row.connect_drag_drop(move |_, context, _, _, time| {
                if let (Some(m), Some(g)) = (weak.upgrade(), weak_group.upgrade()) {
                    m.handle_drop(&g.name());
                }
                context.drag_finish(true, false, time);
                true
            });
            let weak = self.weak();
            group.new_item.connect_activate(move |_| {
                if let Some(m) = weak.upgrade() {
                    m.new_group();
                }
            });
            self.group_list.add(&group.row);
            self.groups.borrow_mut().push(group);
        }

        self.group_list.show_all();

        if let Some(name) = name {
            self.select_group(&name);
        }

        let groups = self.groups.borrow();
        for group in groups.iter() {
            group.set_can_remove(groups.len() != 1);
        }
        self.remove_group_item.set_sensitive(groups.len() != 1);
    }

    fn select_group(&self, name: &str) {
        let row = self.groups.borrow().iter().find(|g| g.name() == name).map(|g| g.row.clone());
        if let Some(row) = row {
            self.group_list.select_row(Some(&row));
        }
    }

    fn on_group_selected(&self) {
        if let Some(group_name) = self.get_current_group() {
            if self.settings.string("active-group") != group_name.as_str() {
                let _ = self.settings.set_string("active-group", &group_name);
            }
            self.generate_previews();
        }
    }

    fn on_search_changed(&self) {
        let search_text = self.search_box.text().to_lowercase().trim().to_string();
        if search_text.is_empty() {
            self.generate_previews();
        } else {
            let mut found = Vec::new();
            for group_name in self.file_handler.get_note_group_names() {
                for (index, info) in self.file_handler.get_note_list(&group_name).into_iter().enumerate() {
                    let title = info.title.clone().unwrap_or_default().to_lowercase();
                    if title.contains(&search_text) || clean_text(&info.text).contains(&search_text) {
                        found.push(PreviewNote { info, group_name: group_name.clone(), index });
                    }
                }
            }
            self.show_previews(found);
        }
    }

    fn on_note_activated(&self) {
        let Some(activated) = self.selected_preview() else { return };
        self.select_group(&activated.group_name);
        self.with_app(|a| a.focus_note(&activated.info));
    }

    fn on_selected_notes_changed(&self) {
        let sensitive = !self.note_view.selected_children().is_empty();
        self.remove_note_button.set_sensitive(sensitive);
        self.duplicate_note_button.set_sensitive(sensitive);
    }

    fn create_note_entry(&self, item: &PreviewNote) -> gtk::FlowBoxChild {
        let widget = gtk::FlowBoxChild::new();

        let dnd_wrapper = gtk::EventBox::new();
        dnd_wrapper.set_above_child(true);
        dnd_wrapper.drag_source_set(gdk::ModifierType::BUTTON1_MASK, &note_targets(), gdk::DragAction::MOVE);
        let weak = self.weak();
        let dragged = item.clone();
        dnd_wrapper.connect_drag_begin(move |_, _| {
            if let Some(m) = weak.upgrade() {
                *m.dragged_note.borrow_mut() = Some(dragged.clone());
            }
        });
        widget.add(&dnd_wrapper);

        let outer_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        outer_box.set_margin_start(10);
        outer_box.set_margin_end(10);
        outer_box.set_margin_top(10);
        outer_box.set_margin_bottom(10);
        outer_box.set_receives_default(true);
        dnd_wrapper.add(&outer_box);

        let wrapper = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        wrapper.set_halign(gtk::Align::Center);
        let context = wrapper.style_context();
        context.add_class(item.info.color.as_deref().unwrap_or("yellow"));
        context.add_class("note-preview");
        outer_box.pack_start(&wrapper, false, false, 0);

        wrapper.pack_start(&note_entry(item, &self.settings), false, false, 0);

        widget.show_all();
        widget
    }

    fn show_previews(&self, items: Vec<PreviewNote>) {
        let old_previews = std::mem::take(&mut *self.previews.borrow_mut());
        for (child, _) in old_previews {
            self.note_view.remove(&child);
        }
        for item in items {
            let child = self.create_note_entry(&item);
            self.note_view.add(&child);
            self.previews.borrow_mut().push((child, item));
        }
        self.on_selected_notes_changed();
    }

    fn generate_previews(&self) {
        let Some(group_name) = self.get_current_group() else { return };
        let items = self
            .file_handler
            .get_note_list(&group_name)
            .into_iter()
            .enumerate()
            .map(|(index, info)| PreviewNote { info, group_name: group_name.clone(), index })
            .collect();
        self.show_previews(items);
    }

    fn get_current_group(&self) -> Option<String> {
        let row = self.group_list.selected_row()?;
        self.groups.borrow().iter().find(|g| g.row == row).map(|g| g.name())
    }

    fn selected_preview(&self) -> Option<PreviewNote> {
        let child = self.note_view.selected_children().into_iter().next()?;
        self.previews.borrow().iter().find(|(c, _)| *c == child).map(|(_, p)| p.clone())
    }

    /// Show an entry under the group list for the name of a new group.
    /// `callback(name, success)` runs when it is done.
    fn create_new_group(&self, callback: impl Fn(Option<String>, bool) + 'static) {
        let entry = gtk::Entry::new();
        entry.set_visible(true);
        self.entry_box.pack_start(&entry, false, false, 5);

        let handlers: Rc<RefCell<Vec<glib::SignalHandlerId>>> = Rc::default();
        let callback = Rc::new(callback);

        let clean_up = {
            let handlers = handlers.clone();
            let entry_box = self.entry_box.clone();
            move |entry: &gtk::Entry| {
                for id in handlers.borrow_mut().drain(..) {
                    entry.disconnect(id);
                }
                entry_box.remove(entry);
            }
        };
        let clean_up = Rc::new(clean_up);

        let maybe_done = {
            let clean_up = clean_up.clone();
            let callback = callback.clone();
            let weak = self.weak();
            move |entry: &gtk::Entry| {
                let group_name = entry.text().to_string();
                clean_up(entry);

                let Some(m) = weak.upgrade() else { return };
                let mut success = false;
                if !group_name.is_empty() {
                    success = m.file_handler.new_group(&group_name);
                }

                if success {
                    m.generate_group_list();
                }

                callback(Some(group_name), success);
            }
        };
        let maybe_done = Rc::new(maybe_done);

        entry.grab_focus();
        let md = maybe_done.clone();
        let activate_id = entry.connect_activate(move |e| md(e));
        let md = maybe_done.clone();
        let focus_id = entry.connect_focus_out_event(move |e, _| {
            md(e);
            glib::Propagation::Proceed
        });
        let key_id = entry.connect_key_press_event(move |e, event| {
            if event.keyval() != gdk::keys::constants::Escape {
                return glib::Propagation::Proceed;
            }
            clean_up(e);
            callback(None, false);
            glib::Propagation::Proceed
        });
        handlers.borrow_mut().extend([activate_id, focus_id, key_id]);
    }

    fn select_group_or(&self, group_name: Option<String>, fallback: Option<String>) {
        if let Some(name) = group_name.or(fallback) {
            self.select_group(&name);
        }
    }

    fn new_group(&self) {
        let old_group = self.get_current_group();
        let weak = self.weak();
        self.create_new_group(move |group_name, success| {
            let Some(m) = weak.upgrade() else { return };
            let name = if success { group_name } else { None };
            m.select_group_or(name, old_group.clone());
        });
    }

    /// The notes of `item`'s group with `item` taken out. Notes are found by
    /// position, not by value, so an identical copy is never removed with it.
    fn list_without(&self, item: &PreviewNote) -> Option<Vec<NoteInfo>> {
        let mut notes = self.file_handler.get_note_list(&item.group_name);
        let index = if notes.get(item.index) == Some(&item.info) {
            item.index
        } else {
            // the list changed since the previews were made
            notes.iter().position(|n| *n == item.info)?
        };
        notes.remove(index);
        Some(notes)
    }

    fn remove_note(&self) {
        let Some(selected) = self.selected_preview() else { return };
        if let Some(notes) = self.list_without(&selected) {
            self.file_handler.update_note_list(notes, &selected.group_name);
        }
    }

    /// Move a note to the end of another group.
    fn move_note(&self, item: &PreviewNote, new_group: &str) {
        if item.group_name == new_group {
            return;
        }
        let Some(old_list) = self.list_without(item) else { return };
        let mut new_list = self.file_handler.get_note_list(new_group);
        new_list.push(item.info.clone());

        self.file_handler.update_note_list(new_list, new_group);
        self.file_handler.update_note_list(old_list, &item.group_name);
    }

    fn remove_group(&self) {
        if let Some(group_name) = self.get_current_group() {
            self.file_handler.remove_group(&group_name);
        }
    }

    fn duplicate_note(&self) {
        let Some(selected) = self.selected_preview() else { return };
        let mut info = selected.info;
        info.x += 50;
        info.y += 50;
        let mut notes = self.file_handler.get_note_list(&selected.group_name);
        notes.push(info);
        self.file_handler.update_note_list(notes, &selected.group_name);
    }

    fn handle_drop(&self, new_group: &str) {
        let Some(dragged) = self.dragged_note.borrow_mut().take() else { return };
        self.move_note(&dragged, new_group);
    }

    fn handle_new_group_drop(&self) {
        let old_group = self.get_current_group();
        let weak = self.weak();
        self.create_new_group(move |group_name, success| {
            let Some(m) = weak.upgrade() else { return };
            let dragged = m.dragged_note.borrow_mut().take();
            let mut name = None;
            if success {
                if let (Some(dragged), Some(new)) = (dragged, group_name.as_deref()) {
                    m.move_note(&dragged, new);
                }
                name = group_name;
            }
            m.select_group_or(name, old_group.clone());
        });
    }
}
