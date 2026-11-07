//! The application: command line, D-Bus interface, tray icon and the list of
//! note windows for the active group.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::ffi::StatusIcon;
use crate::file_handler::FileHandler;
use crate::i18n::gettext;
use crate::manager::{data_dir, NotesManager};
use crate::note::Note;
use crate::notes::NoteInfo;
use crate::settings_window::{settings_window, shortcuts_window, SCHEMA};
use crate::signal::HandlerId;
use crate::util::gnote_to_internal_format;
use crate::widgets::confirm;

pub const APPLICATION_ID: &str = "org.x.sticky";
const DBUS_PATH: &str = "/org/x/sticky";
const DBUS_INTERFACE_XML: &str = r#"
<node>
  <interface name='org.x.sticky'>
    <method name='ShowNotes'>
    </method>
    <method name='NewNote'>
      <arg type='s' name='text' direction='in'/>
    </method>
    <method name='NewNoteBlank'>
    </method>
    <method name='ReloadNotesFromFile'>
    </method>
    <signal name='NotesChanged'>
    </signal>
  </interface>
</node>
"#;

thread_local! {
    static DBUS_APP: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
}

pub struct App {
    gtk_app: gtk::Application,
    settings: OnceCell<gio::Settings>,
    dummy_window: OnceCell<gtk::Window>,
    file_handler: OnceCell<Rc<FileHandler>>,

    status_icon: RefCell<Option<StatusIcon>>,
    has_activated: Cell<bool>,
    notes: RefCell<Vec<Rc<Note>>>,
    settings_window: RefCell<Option<gtk::Window>>,
    keyboard_shortcuts: RefCell<Option<gtk::ShortcutsWindow>>,
    // There's no use creating the manager if a user is never going to use it, so we don't until it's asked for.
    // Therefore, we should never assume that the manager already exists, and handle the situation gracefully if it
    // is none.
    manager: RefCell<Option<Rc<NotesManager>>>,
    notes_hidden: Cell<bool>,
    autostart_mode: Cell<bool>,
    dbus_register_id: RefCell<Option<gio::RegistrationId>>,
    group_buttons: RefCell<Vec<(String, gtk::RadioMenuItem)>>,
    note_group: RefCell<String>,
    group_update_id: Cell<Option<HandlerId>>,
    active_group_changed_id: RefCell<Option<glib::SignalHandlerId>>,

    weak_self: RefCell<Weak<App>>,
}

impl App {
    pub fn new() -> Rc<App> {
        let gtk_app = gtk::Application::new(Some(APPLICATION_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);

        let app = Rc::new(App {
            gtk_app: gtk_app.clone(),
            settings: OnceCell::new(),
            dummy_window: OnceCell::new(),
            file_handler: OnceCell::new(),
            status_icon: RefCell::default(),
            has_activated: Cell::new(false),
            notes: RefCell::default(),
            settings_window: RefCell::default(),
            keyboard_shortcuts: RefCell::default(),
            manager: RefCell::default(),
            notes_hidden: Cell::new(false),
            autostart_mode: Cell::new(false),
            dbus_register_id: RefCell::default(),
            group_buttons: RefCell::default(),
            note_group: RefCell::default(),
            group_update_id: Cell::new(None),
            active_group_changed_id: RefCell::default(),
            weak_self: RefCell::default(),
        });
        *app.weak_self.borrow_mut() = Rc::downgrade(&app);

        gtk_app.add_main_option("autostart", glib::Char::from(b'a'), glib::OptionFlags::NONE, glib::OptionArg::None, "Autostart mode", None);
        gtk_app.add_main_option("new", glib::Char::from(b'n'), glib::OptionFlags::NONE, glib::OptionArg::None, "Create a new note", None);
        gtk_app.add_main_option("toggle", glib::Char::from(b't'), glib::OptionFlags::NONE, glib::OptionArg::None, "Toggle note visibility", None);

        let weak = app.weak();
        gtk_app.connect_command_line(move |_, command_line| match weak.upgrade() {
            Some(app) => app.do_command_line(command_line),
            None => 1,
        });
        let weak = app.weak();
        gtk_app.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.do_activate();
            }
        });
        let weak = app.weak();
        gtk_app.connect_startup(move |_| {
            if let Some(app) = weak.upgrade() {
                app.dbus_register();
            }
        });
        let weak = app.weak();
        gtk_app.connect_shutdown(move |_| {
            if let Some(app) = weak.upgrade() {
                app.dbus_unregister();
            }
        });

        app
    }

    pub fn run(&self) -> glib::ExitCode {
        self.gtk_app.run()
    }

    fn weak(&self) -> Weak<App> {
        self.weak_self.borrow().clone()
    }

    fn rc(&self) -> Rc<App> {
        self.weak().upgrade().expect("App is alive")
    }

    pub fn settings(&self) -> &gio::Settings {
        self.settings.get().expect("settings are created on activate")
    }

    fn file_handler(&self) -> &Rc<FileHandler> {
        self.file_handler.get().expect("file handler is created on activate")
    }

    fn dummy_window(&self) -> &gtk::Window {
        self.dummy_window.get().expect("dummy window is created on activate")
    }

    fn notes(&self) -> Vec<Rc<Note>> {
        self.notes.borrow().clone()
    }

    fn do_command_line(&self, command_line: &gio::ApplicationCommandLine) -> i32 {
        let options = command_line.options_dict();

        if options.contains("autostart") {
            let settings = gio::Settings::new(SCHEMA);
            if !settings.boolean("autostart") {
                std::process::exit(0);
            } else {
                self.autostart_mode.set(true);
            }
        }

        if !self.has_activated.get() {
            self.gtk_app.activate();
        }

        if options.contains("new") {
            self.new_note(None);
        }

        if options.contains("toggle") {
            if self.notes_hidden.get() {
                self.activate_notes(gtk::current_event_time());
            } else {
                self.hide_notes();
            }
        }

        0
    }

    fn do_activate(&self) {
        if self.has_activated.get() {
            for note in self.notes() {
                note.restore(0);
            }
            self.open_manager(0);
            return;
        }

        glib::set_prgname(Some("sticky"));
        gtk::Window::set_default_icon_name("sticky");

        let settings = gio::Settings::new(SCHEMA);
        let _ = self.settings.set(settings.clone());

        let dummy_window = gtk::Window::builder()
            .title(gettext("Notes"))
            .default_height(1)
            .default_width(1)
            .decorated(false)
            .deletable(false)
            .name("dummy-window")
            .build();
        dummy_window.show();
        let _ = self.dummy_window.set(dummy_window.clone());

        let file_handler = FileHandler::new(&settings, &dummy_window);
        let _ = self.file_handler.set(file_handler.clone());

        if settings.boolean("first-run") {
            self.first_run();
        }

        // Backwards compatibility
        // - Update random color option to cycle
        if settings.string("default-color") == "random" {
            let _ = settings.set_string("default-color", "cycle");
        }

        let weak = self.weak();
        file_handler.lists_changed.connect(move |_| {
            if let Some(app) = weak.upgrade() {
                app.on_lists_changed();
            }
        });
        let weak = self.weak();
        let id = file_handler.group_changed.connect(move |group_name| {
            if let Some(app) = weak.upgrade() {
                app.on_group_changed(group_name);
            }
        });
        self.group_update_id.set(Some(id));
        let weak = self.weak();
        file_handler.group_name_changed.connect(move |(old_name, new_name)| {
            if let Some(app) = weak.upgrade() {
                app.on_group_name_changed(old_name, new_name);
            }
        });
        let weak = self.weak();
        file_handler.saved.connect(move |_| {
            if let Some(app) = weak.upgrade() {
                app.on_save();
            }
        });

        if settings.boolean("show-in-tray") {
            self.create_status_icon();
        }

        let weak = self.weak();
        settings.connect_changed(Some("show-in-tray"), move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.update_tray_icon();
            }
        });
        let weak = self.weak();
        settings.connect_changed(Some("show-in-taskbar"), move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.update_dummy_window();
            }
        });
        let weak = self.weak();
        let id = settings.connect_changed(Some("active-group"), move |settings, _| {
            if let Some(app) = weak.upgrade() {
                app.on_active_group_changed(settings);
            }
        });
        *self.active_group_changed_id.borrow_mut() = Some(id);
        self.update_dummy_window();

        *self.note_group.borrow_mut() = settings.string("active-group").to_string();
        let group_names = file_handler.get_note_group_names();
        let note_group = self.note_group.borrow().clone();
        if !group_names.contains(&note_group) {
            if let Some(first) = group_names.first() {
                *self.note_group.borrow_mut() = first.clone();
                let _ = settings.set_string("active-group", first);
            } else {
                file_handler.new_group(&note_group);
            }
        }

        let provider = gtk::CssProvider::new();
        if let Err(e) = provider.load_from_path(&data_dir().join("sticky.css").to_string_lossy()) {
            eprintln!("sticky: could not load the style sheet: {e}");
        }
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(&screen, &provider, 600);
        }

        self.load_notes();

        // Keep running when no window is open.
        std::mem::forget(self.gtk_app.hold());

        if self.autostart_mode.get() {
            self.autostart_mode.set(false);
            if !settings.boolean("autostart-notes-visible") {
                self.hide_notes();
            }
        } else if settings.boolean("show-manager") {
            self.open_manager(0);
        }

        if !settings.boolean("show-in-tray") {
            self.open_manager(0);
        }

        self.has_activated.set(true);
    }

    fn dbus_register(&self) {
        let (Some(connection), Some(object_path)) = (self.gtk_app.dbus_connection(), self.gtk_app.dbus_object_path()) else {
            return;
        };
        let node = match gio::DBusNodeInfo::for_xml(DBUS_INTERFACE_XML) {
            Ok(node) => node,
            Err(e) => {
                eprintln!("sticky: bad D-Bus interface: {e}");
                return;
            }
        };
        let Some(interface) = node.lookup_interface(APPLICATION_ID) else { return };

        // The callbacks must be Send + Sync, but GApplication always calls them on the main thread, so the
        // application is found through a thread local instead of being captured.
        DBUS_APP.with(|a| *a.borrow_mut() = self.weak());

        let result = connection.register_object(
            &object_path,
            &interface,
            move |_, _, _, _, method_name, params, _invocation| {
                if let Some(app) = DBUS_APP.with(|a| a.borrow().upgrade()) {
                    app.dbus_method_call(method_name, &params);
                }
            },
            |_, _, _, _, _| ().to_variant(),
            |_, _, _, _, _, _| false,
        );

        match result {
            Ok(id) => *self.dbus_register_id.borrow_mut() = Some(id),
            Err(e) => eprintln!("sticky: could not register the D-Bus interface: {e}"),
        }
    }

    fn dbus_unregister(&self) {
        if let (Some(id), Some(connection)) = (self.dbus_register_id.borrow_mut().take(), self.gtk_app.dbus_connection()) {
            let _ = connection.unregister_object(id);
        }
    }

    fn dbus_method_call(&self, method_name: &str, params: &glib::Variant) {
        if !self.has_activated.get() {
            return;
        }
        match method_name {
            "ShowNotes" => self.activate_notes(0),
            "NewNote" => {
                let text = params.child_value(0).str().unwrap_or("").to_string();
                let (x, y, direction) = self.get_direction(None);
                let (x, y) = self.find_note_location(x, y, direction);
                self.add_note(&NoteInfo { text, x, y, ..Default::default() });
            }
            "NewNoteBlank" => self.new_note(None),
            "ReloadNotesFromFile" => self.reload_notes_from_file(),
            _ => {}
        }
    }

    fn first_run(&self) {
        let file_handler = self.file_handler();
        let gnote_dir = glib::user_data_dir().join("gnote");

        if gnote_dir.exists() {
            let mut import_notes: Vec<std::path::PathBuf> = std::fs::read_dir(&gnote_dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".note"))
                        .collect()
                })
                .unwrap_or_default();
            import_notes.sort();

            if !import_notes.is_empty()
                && confirm(
                    &gettext("Notes"),
                    &gettext("Would you like to import your notes from Gnote? This will not change your Gnote notes in any way."),
                    Some(self.dummy_window()),
                    None,
                )
            {
                let mut coordinates = 40;
                for file in import_notes {
                    let (group_name, mut info, is_template) = match gnote_to_internal_format(&file) {
                        Ok(result) => result,
                        Err(e) => {
                            eprintln!("sticky: could not import {}: {e}", file.display());
                            continue;
                        }
                    };
                    if !is_template {
                        info.color = Some("yellow".into());
                        info.x = coordinates;
                        info.y = coordinates;

                        if !file_handler.has_group(&group_name) {
                            file_handler.new_group(&group_name);
                        }

                        let mut group_list = file_handler.get_note_list(&group_name);
                        group_list.push(info);
                        file_handler.update_note_list(group_list, &group_name);

                        coordinates += 20;
                    }
                }
            }
        }

        // Create a default group
        if file_handler.get_note_group_names().is_empty() {
            let note = NoteInfo { text: String::new(), color: Some("yellow".into()), x: 20, y: 20, ..Default::default() };
            file_handler.update_note_list(vec![note], &gettext("Group 1"));
        }

        let _ = self.settings().set_boolean("first-run", false);
    }

    fn create_status_icon(&self) {
        let icon = StatusIcon::new();
        icon.set_name("sticky");
        icon.set_icon_name("sticky-symbolic");
        icon.set_tooltip_text(&format!(
            "{}\n<i>{}</i>\n<i>{}</i>",
            gettext("Notes"),
            gettext("Left click to toggle notes"),
            gettext("Middle click to toggle the manager")
        ));
        icon.set_visible(true);
        let weak = self.weak();
        icon.connect_button_press(move |button, time| {
            if let Some(app) = weak.upgrade() {
                if button == 1 {
                    app.activate_notes(time);
                } else if button == 2 {
                    app.toggle_manager(time);
                }
            }
        });
        *self.status_icon.borrow_mut() = Some(icon);

        self.rebuild_tray_context_menu();
    }

    fn rebuild_tray_context_menu(&self) {
        let Some(icon) = self.status_icon.borrow().clone() else { return };

        let mut group_buttons = Vec::new();

        let menu = gtk::Menu::new();
        let item = gtk::MenuItem::with_label(&gettext("New Note"));
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.new_note(None);
            }
        });
        menu.append(&item);

        let item = gtk::MenuItem::with_label(&gettext("Manage Notes"));
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_manager(0);
            }
        });
        menu.append(&item);

        menu.append(&gtk::SeparatorMenuItem::new());

        let active = self.settings().string("active-group");
        let mut last_item: Option<gtk::RadioMenuItem> = None;
        for group in self.file_handler().get_note_group_names() {
            let item = gtk::RadioMenuItem::with_label(&group);

            if let Some(last) = &last_item {
                item.join_group(Some(last));
            }

            if group == active.as_str() {
                item.set_active(true);
            }

            last_item = Some(item.clone());

            let weak = self.weak();
            let name = group.clone();
            item.connect_activate(move |_| {
                if let Some(app) = weak.upgrade() {
                    let _ = app.settings().set_string("active-group", &name);
                }
            });
            menu.append(&item);
            group_buttons.push((group, item));
        }

        menu.append(&gtk::SeparatorMenuItem::new());

        let item = gtk::MenuItem::with_label(&gettext("Preferences"));
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_settings_window();
            }
        });
        menu.append(&item);

        menu.append(&gtk::SeparatorMenuItem::new());

        let item = gtk::MenuItem::with_label(&gettext("Quit"));
        let weak = self.weak();
        item.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.quit_app();
            }
        });
        menu.append(&item);

        menu.show_all();

        *self.group_buttons.borrow_mut() = group_buttons;
        icon.set_secondary_menu(&menu);
    }

    fn destroy_status_icon(&self) {
        if let Some(icon) = self.status_icon.borrow_mut().take() {
            icon.set_visible(false);
        }
    }

    fn update_tray_icon(&self) {
        if self.settings().boolean("show-in-tray") {
            self.create_status_icon();
        } else {
            self.open_manager(0);
            self.destroy_status_icon();
        }
    }

    fn on_active_group_changed(&self, settings: &gio::Settings) {
        let group = settings.string("active-group").to_string();
        if self.status_icon.borrow().is_some() {
            let button = self.group_buttons.borrow().iter().find(|(name, _)| *name == group).map(|(_, b)| b.clone());
            if let Some(button) = button.filter(|b| !b.is_active()) {
                let id = self.active_group_changed_id.borrow();
                if let Some(id) = id.as_ref() {
                    settings.block_signal(id);
                }
                button.set_active(true);
                if let Some(id) = id.as_ref() {
                    settings.unblock_signal(id);
                }
            }
        }

        self.change_visible_note_group(None);
    }

    fn update_dummy_window(&self) {
        let settings = self.settings();
        let dummy = self.dummy_window();
        if settings.boolean("show-in-taskbar") && !self.notes_hidden.get() {
            dummy.set_skip_taskbar_hint(false);
        } else {
            dummy.set_skip_taskbar_hint(true);
        }

        dummy.move_(-2, -2);

        if settings.boolean("desktop-window-state") {
            dummy.stick();
        }
    }

    fn activate_notes(&self, time: u32) {
        let notes = self.notes();
        if notes.iter().any(|n| n.is_active()) {
            self.hide_notes();
            return;
        }

        self.dummy_window().present_with_time(time);

        for note in &notes {
            note.restore(time);
        }

        if notes.is_empty() {
            self.new_note(None);
        }

        self.notes_hidden.set(false);
        self.update_dummy_window();
    }

    fn hide_notes(&self) {
        for note in self.notes() {
            note.hide();
        }

        self.notes_hidden.set(true);
        self.update_dummy_window();
    }

    /// Called by the manager when its group list is clicked.
    pub fn present_notes(&self, time: u32) {
        for note in self.notes() {
            note.present_with_time(time);
        }
    }

    fn workarea(monitor: Option<gdk::Monitor>) -> gdk::Rectangle {
        monitor.map(|m| m.workarea()).unwrap_or_else(|| gdk::Rectangle::new(0, 0, 1024, 768))
    }

    fn find_note_location(&self, mut x: i32, mut y: i32, direction: (i32, i32)) -> (i32, i32) {
        let notes = self.file_handler().get_note_list(&self.note_group.borrow());
        while notes.iter().any(|n| n.x == x && n.y == y) {
            x += 20 * direction.0;
            y += 60 * direction.1;
        }

        // make sure the note doesn't get placed off-screen or over a panel
        let display = gdk::Display::default();
        let workarea = Self::workarea(display.and_then(|d| d.monitor_at_point(x, y)));
        let min_x = workarea.x();
        let max_x = workarea.x() + workarea.width();
        let min_y = workarea.y();
        let max_y = workarea.y() + workarea.height();

        let settings = self.settings();
        let x_end = x + settings.uint("default-height") as i32;
        let y_end = y + settings.uint("default-width") as i32;

        if x < min_x || x_end > max_x || y < min_y || y_end > max_y {
            // the calculated location wont work, so put the new note near the top left corner
            x = workarea.x() + 20;
            y = workarea.y() + 20;
        }

        (x, y)
    }

    fn get_direction(&self, parent: Option<&Note>) -> (i32, i32, (i32, i32)) {
        let display = gdk::Display::default();
        // if user has no monitor configured, or uses Wayland, for now guess monitor 0 (better than a crash)
        let monitor = display.as_ref().and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)));
        let workarea = Self::workarea(monitor);
        let settings = self.settings();
        let default_width = settings.uint("default-width") as i32;
        let default_height = settings.uint("default-height") as i32;

        let mut direction = (1, 1);
        let (x, y);
        if let Some(parent) = parent {
            let info = parent.get_info();
            if info.x > workarea.width() / 2 {
                x = info.x - 20;
                direction.0 = -1;
            } else {
                x = info.x + 20;
            }
            if info.y > workarea.height() / 2 {
                y = info.y - 60;
                direction.1 = -1;
            } else {
                y = info.y + 60;
            }
        } else {
            let position = settings.string("default-position");
            let (pos_y, pos_x) = position.split_once('-').unwrap_or(("top", "left"));
            x = match pos_x {
                "right" => {
                    direction.0 = -1;
                    workarea.width() - default_width - 20
                }
                "center" => workarea.width() / 2 - default_width / 2,
                _ => 20,
            };
            y = match pos_y {
                "bottom" => {
                    direction.1 = -1;
                    workarea.height() - default_height - 60
                }
                "center" => workarea.height() / 2 - default_height / 2,
                _ => 20,
            };
        }

        (x, y, direction)
    }

    pub fn new_note(&self, parent: Option<&Note>) {
        let (x, y, direction) = self.get_direction(parent);
        let (x, y) = self.find_note_location(x, y, direction);
        self.add_note(&NoteInfo { x, y, ..Default::default() });
    }

    fn add_note(&self, info: &NoteInfo) {
        let note = self.generate_note(info);
        note.present_with_time(gtk::current_event_time());

        // Note is Gdk.WindowType.UTILITY - these don't get raised automatically
        // (see muffin: window.c:window_state_on_map)
        if !note.window.is_realized() {
            note.window.realize();
        }
        if let Some(window) = note.window.window() {
            window.raise();
        }

        self.on_update();
    }

    fn generate_note(&self, info: &NoteInfo) -> Rc<Note> {
        let note = Note::new(self.settings(), self.dummy_window(), info);

        let weak = self.weak();
        note.update.connect(move |_| {
            if let Some(app) = weak.upgrade() {
                app.on_update();
            }
        });
        let weak = self.weak();
        let weak_note = Rc::downgrade(&note);
        note.removed.connect(move |_| {
            if let (Some(app), Some(note)) = (weak.upgrade(), weak_note.upgrade()) {
                app.on_removed(&note);
            }
        });
        let weak = self.weak();
        let weak_note = Rc::downgrade(&note);
        note.new_note_requested.connect(move |_| {
            if let (Some(app), Some(note)) = (weak.upgrade(), weak_note.upgrade()) {
                app.new_note(Some(&note));
            }
        });
        let weak = self.weak();
        let weak_note = Rc::downgrade(&note);
        note.duplicate_requested.connect(move |_| {
            if let (Some(app), Some(note)) = (weak.upgrade(), weak_note.upgrade()) {
                app.duplicate_note(&note);
            }
        });

        self.notes.borrow_mut().push(note.clone());

        note
    }

    fn load_notes(&self) {
        let old = std::mem::take(&mut *self.notes.borrow_mut());
        for note in old {
            note.destroy();
        }

        let group = self.note_group.borrow().clone();
        for info in self.file_handler().get_note_list(&group) {
            self.generate_note(&info);
        }
    }

    fn reload_notes_from_file(&self) {
        self.file_handler().load_notes();
        self.load_notes();
    }

    fn duplicate_note(&self, note: &Note) {
        let mut info = note.get_info();
        info.x += 50;
        info.y += 50;
        self.add_note(&info);
    }

    pub fn focus_note(&self, info: &NoteInfo) {
        for note in self.notes() {
            if note.get_info() == *info {
                note.present_with_time(0);
            }
        }
    }

    fn on_lists_changed(&self) {
        let group = self.note_group.borrow().clone();
        if !self.file_handler().has_group(&group) {
            self.change_visible_note_group(None);
        } else {
            self.load_notes();
        }

        self.rebuild_tray_context_menu();
    }

    fn on_group_changed(&self, group_name: &str) {
        if *self.note_group.borrow() == group_name {
            self.load_notes();
        }
    }

    fn on_group_name_changed(&self, old_name: &str, new_name: &str) {
        if *self.note_group.borrow() == old_name {
            self.change_visible_note_group(Some(new_name));
        }

        if self.settings().string("active-group") == old_name {
            let _ = self.settings().set_string("active-group", new_name);
        }

        self.rebuild_tray_context_menu();
    }

    fn on_save(&self) {
        if let Some(connection) = self.gtk_app.dbus_connection() {
            let _ = connection.emit_signal(None, DBUS_PATH, APPLICATION_ID, "NotesChanged", None);
        }
    }

    fn change_visible_note_group(&self, group: Option<&str>) {
        let default = self.settings().string("active-group").to_string();
        *self.note_group.borrow_mut() = group.map(str::to_string).unwrap_or_else(|| default.clone());

        let group_names = self.file_handler().get_note_group_names();
        let current = self.note_group.borrow().clone();
        if !group_names.contains(&current) {
            if let Some(first) = group_names.first() {
                *self.note_group.borrow_mut() = first.clone();
            } else {
                self.file_handler().new_group(&default);
                *self.note_group.borrow_mut() = default;
            }
        }

        self.load_notes();
    }

    pub fn open_manager(&self, time: u32) {
        if let Some(manager) = self.manager.borrow().as_ref() {
            let time = if time == 0 { gtk::current_event_time() } else { time };
            manager.window.present_with_time(time);
            return;
        }

        let manager = NotesManager::new(&self.rc(), self.file_handler());
        let weak = self.weak();
        manager.window.connect_delete_event(move |_, _| match weak.upgrade() {
            Some(app) => app.manager_closed(),
            None => glib::Propagation::Proceed,
        });
        *self.manager.borrow_mut() = Some(manager);
    }

    fn toggle_manager(&self, time: u32) {
        let window = self.manager.borrow().as_ref().map(|m| m.window.clone());
        match window {
            Some(w) if w.is_active() && w.is_visible() => w.hide(),
            _ => self.open_manager(time),
        }
    }

    fn manager_closed(&self) -> glib::Propagation {
        if self.status_icon.borrow().is_none() {
            self.quit_app();
        }

        if let Some(manager) = self.manager.borrow().as_ref() {
            manager.window.hide();
        }

        glib::Propagation::Stop
    }

    pub fn open_settings_window(&self) {
        if let Some(window) = self.settings_window.borrow().as_ref() {
            window.present_with_time(gtk::current_event_time());
            return;
        }

        let window = settings_window(self.settings());
        let weak = self.weak();
        window.connect_destroy(move |_| {
            if let Some(app) = weak.upgrade() {
                app.settings_window.borrow_mut().take();
            }
        });
        window.show_all();
        *self.settings_window.borrow_mut() = Some(window);
    }

    pub fn open_about(&self) {
        let dialog = gtk::AboutDialog::new();
        if let Some(manager) = self.manager.borrow().as_ref() {
            dialog.set_transient_for(Some(&manager.window));
        }
        dialog.set_modal(true);
        dialog.set_title(&gettext("About"));
        dialog.set_program_name(&gettext("Notes"));
        dialog.set_comments(Some(&gettext("Take notes and stay organized")));
        dialog.set_license_type(gtk::License::Gpl20);
        dialog.set_version(Some(env!("CARGO_PKG_VERSION")));
        dialog.set_icon_name(Some("sticky"));
        dialog.set_logo_icon_name(Some("sticky"));
        dialog.set_website(Some("https://www.github.com/linuxmint/sticky"));
        dialog.connect_response(|dialog, response| {
            if response == gtk::ResponseType::Cancel || response == gtk::ResponseType::DeleteEvent {
                unsafe { dialog.destroy() };
            }
        });
        dialog.show();
    }

    pub fn open_keyboard_shortcuts(&self) {
        if let Some(window) = self.keyboard_shortcuts.borrow().as_ref() {
            window.present_with_time(gtk::current_event_time());
            return;
        }

        let window = shortcuts_window();
        let weak = self.weak();
        window.connect_destroy(move |_| {
            if let Some(app) = weak.upgrade() {
                app.keyboard_shortcuts.borrow_mut().take();
            }
        });
        window.show_all();
        *self.keyboard_shortcuts.borrow_mut() = Some(window);
    }

    fn on_update(&self) {
        let info: Vec<NoteInfo> = self.notes().iter().map(|n| n.get_info()).collect();

        let file_handler = self.file_handler();
        let id = self.group_update_id.get();
        if let Some(id) = id {
            file_handler.group_changed.block(id);
        }
        file_handler.update_note_list(info, &self.note_group.borrow().clone());
        if let Some(id) = id {
            file_handler.group_changed.unblock(id);
        }
    }

    fn on_removed(&self, note: &Rc<Note>) {
        self.notes.borrow_mut().retain(|n| !Rc::ptr_eq(n, note));
        self.on_update();
    }

    fn quit_app(&self) {
        self.file_handler().flush();

        let notes = std::mem::take(&mut *self.notes.borrow_mut());
        for note in notes {
            note.destroy();
        }

        self.gtk_app.quit();
    }
}
