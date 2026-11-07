//! The Preferences window and the keyboard shortcuts window.
//!
//! The Python version used the GSettings widgets from python3-xapp. Those
//! only exist for Python, so equivalent rows are built here.

use gtk::prelude::*;

use crate::ffi::PreferencesWindow;
use crate::i18n::{gettext, n_};
use crate::note::{COLORS, FONT_SCALES};

pub const SCHEMA: &str = "org.x.sticky";

pub const START_POSITIONS: &[(&str, &str)] = &[
    ("top-left", n_("Top Left")),
    ("top-center", n_("Top Center")),
    ("top-right", n_("Top Right")),
    ("center-left", n_("Center Left")),
    ("center-center", n_("Center")),
    ("center-right", n_("Center Right")),
    ("bottom-left", n_("Bottom Left")),
    ("bottom-center", n_("Bottom Center")),
    ("bottom-right", n_("Bottom Right")),
];

const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        n_("Operations"),
        &[
            (n_("Move selection up"), "<ctrl><shift>Up"),
            (n_("Move selection down"), "<ctrl><shift>Down"),
            (n_("Undo"), "<ctrl>z"),
            (n_("Redo"), "<ctrl>y"),
            (n_("Toggle Checklist"), "<ctrl>e"),
            (n_("Toggle Bullets"), "<ctrl>l"),
        ],
    ),
    (
        n_("Formatting"),
        &[
            (n_("Bold"), "<ctrl>b"),
            (n_("Italic"), "<ctrl>i"),
            (n_("Fixed Width"), "<ctrl>f"),
            (n_("Underline"), "<ctrl>u"),
            (n_("Strikethrough"), "<ctrl>k"),
            (n_("Highlight"), "<ctrl>g"),
            (n_("Header"), "<ctrl>h"),
        ],
    ),
];

/// A settings row: a label on the left and `widget` on the right.
fn row<W: IsA<gtk::Widget>>(settings: &gio::Settings, label: &str, widget: &W, dep_key: Option<&str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 15);
    row.set_margin_start(20);
    row.set_margin_end(20);
    row.set_margin_top(6);
    row.set_margin_bottom(6);

    let label = gtk::Label::new(Some(label));
    label.set_halign(gtk::Align::Start);
    row.pack_start(&label, false, false, 0);
    row.pack_end(widget, false, false, 0);

    if let Some(dep_key) = dep_key {
        settings.bind(dep_key, &row, "sensitive").flags(gio::SettingsBindFlags::GET).build();
    }

    row
}

fn switch(settings: &gio::Settings, label: &str, key: &str, dep_key: Option<&str>) -> gtk::Box {
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    settings.bind(key, &switch, "active").build();
    row(settings, label, &switch, dep_key)
}

#[allow(clippy::too_many_arguments)]
fn spin(settings: &gio::Settings, label: &str, key: &str, min: f64, max: f64, step: f64, units: Option<&str>, tooltip: Option<&str>) -> gtk::Box {
    let spin = gtk::SpinButton::with_range(min, max, step);
    spin.set_digits(0);
    settings.bind(key, &spin, "value").build();

    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    widget.pack_start(&spin, false, false, 0);
    if let Some(units) = units {
        widget.pack_start(&gtk::Label::new(Some(units)), false, false, 0);
    }

    let row = row(settings, label, &widget, None);
    if let Some(tooltip) = tooltip {
        row.set_tooltip_text(Some(tooltip));
    }
    row
}

fn combo(settings: &gio::Settings, label: &str, key: &str, options: &[(String, String)]) -> gtk::Box {
    let combo = gtk::ComboBoxText::new();
    for (id, name) in options {
        combo.append(Some(id), name);
    }
    combo.set_row_separator_func(|model, iter| model.value(iter, 1).get::<String>().map(|id| id == "sep").unwrap_or(false));
    settings.bind(key, &combo, "active-id").build();
    combo.set_valign(gtk::Align::Center);
    row(settings, label, &combo, None)
}

fn font(settings: &gio::Settings, label: &str, key: &str) -> gtk::Box {
    let button = gtk::FontButton::new();
    button.set_level(gtk::FontChooserLevel::SIZE);
    button.set_valign(gtk::Align::Center);
    settings.bind(key, &button, "font").build();
    row(settings, label, &button, None)
}

fn page() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 0)
}

pub fn settings_window(settings: &gio::Settings) -> gtk::Window {
    let prefs = PreferencesWindow::new();
    let window = prefs.window().clone();
    window.set_skip_taskbar_hint(false);
    window.set_title(&gettext("Preferences"));

    // general settings
    let p = page();
    p.pack_start(&switch(settings, &gettext("Show notes on all desktops"), "desktop-window-state", None), false, false, 0);
    p.pack_start(&switch(settings, &gettext("Show in taskbar"), "show-in-taskbar", None), false, false, 0);
    p.pack_start(&switch(settings, &gettext("Tray icon"), "show-in-tray", None), false, false, 0);
    p.pack_start(
        &switch(settings, &gettext("Show the main window automatically"), "show-manager", Some("show-in-tray")),
        false,
        false,
        0,
    );
    prefs.add_page(&p, "general", &gettext("General"));

    // note related settings
    let p = page();
    p.pack_start(&spin(settings, &gettext("Default height"), "default-height", 50.0, 2000.0, 10.0, None, None), false, false, 0);
    p.pack_start(&spin(settings, &gettext("Default width"), "default-width", 50.0, 2000.0, 10.0, None, None), false, false, 0);
    let positions: Vec<(String, String)> = START_POSITIONS.iter().map(|(id, name)| (id.to_string(), gettext(name))).collect();
    p.pack_start(&combo(settings, &gettext("Default position"), "default-position", &positions), false, false, 0);
    let mut colors: Vec<(String, String)> = COLORS.iter().map(|(id, name, _)| (id.to_string(), gettext(name))).collect();
    colors.push(("sep".into(), String::new()));
    colors.push(("cycle".into(), gettext("Cycle Colors")));
    p.pack_start(&combo(settings, &gettext("Default color"), "default-color", &colors), false, false, 0);
    p.pack_start(&font(settings, &gettext("Font"), "font"), false, false, 0);
    p.pack_start(&switch(settings, &gettext("Show spelling mistakes"), "inline-spell-check", None), false, false, 0);
    prefs.add_page(&p, "notes", &gettext("Notes"));

    // backups
    let p = page();
    p.pack_start(&switch(settings, &gettext("Automatic backups"), "automatic-backups", None), false, false, 0);
    let hours = gettext("hours");
    p.pack_start(&spin(settings, &gettext("Time between backups"), "backup-interval", 1.0, 168.0, 1.0, Some(&hours), None), false, false, 0);
    let obm_tooltip = gettext("Set this to zero if you wish to keep all backups indefinitely");
    p.pack_start(&spin(settings, &gettext("Number to keep"), "old-backups-max", 0.0, 100.0, 1.0, None, Some(&obm_tooltip)), false, false, 0);
    prefs.add_page(&p, "backup", &gettext("Backups"));

    // autostart
    let p = page();
    p.pack_start(&switch(settings, &gettext("Start automatically"), "autostart", None), false, false, 0);
    p.pack_start(&switch(settings, &gettext("Show notes on the screen"), "autostart-notes-visible", Some("autostart")), false, false, 0);
    prefs.add_page(&p, "autostart", &gettext("Automatic start"));

    window.show_all();
    window
}

pub fn shortcuts_window() -> gtk::ShortcutsWindow {
    let window: gtk::ShortcutsWindow = glib::Object::new();

    let section: gtk::ShortcutsSection = glib::Object::builder().property("visible", true).build();

    let add_group = |title: &str, items: Vec<(String, String)>| {
        let group: gtk::ShortcutsGroup = glib::Object::builder().property("title", title).property("visible", false).build();
        section.add(&group);
        for (title, accelerator) in items {
            let shortcut: gtk::ShortcutsShortcut = glib::Object::builder()
                .property("title", title)
                .property("accelerator", accelerator)
                .property("visible", true)
                .build();
            group.add(&shortcut);
        }
    };

    for (group, items) in SHORTCUTS {
        add_group(&gettext(group), items.iter().map(|(t, a)| (gettext(t), a.to_string())).collect());
    }

    add_group(
        &gettext("Text Size"),
        FONT_SCALES.iter().enumerate().map(|(i, (_, name, _))| (gettext(name), format!("<ctrl>{}", i + 2))).collect(),
    );

    window.add(&section);
    window.show_all();
    window
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Run from `gtk_tests` in main.rs. Needs the compiled schema
    /// (GSETTINGS_SCHEMA_DIR), otherwise it is skipped.
    pub fn windows_build() {
        let has_schema = gio::SettingsSchemaSource::default().and_then(|s| s.lookup(SCHEMA, true)).is_some();
        if !has_schema {
            eprintln!("no schema, skipping the settings window check");
            return;
        }
        let settings = gio::Settings::new(SCHEMA);
        let window = settings_window(&settings);
        assert!(window.is_visible());
        let shortcuts = shortcuts_window();
        assert!(shortcuts.is_visible());
        unsafe {
            window.destroy();
            shortcuts.destroy();
        }
    }
}
