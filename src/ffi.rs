//! Hand-written bindings for the handful of libxapp and gspell functions
//! Sticky needs. Neither library has maintained Rust bindings.

use std::ffi::{c_char, c_void, CString};

use glib::prelude::*;
use glib::translate::*;

#[link(name = "xapp")]
extern "C" {
    fn xapp_status_icon_new() -> *mut glib::gobject_ffi::GObject;
    fn xapp_status_icon_set_name(icon: *mut c_void, name: *const c_char);
    fn xapp_status_icon_set_icon_name(icon: *mut c_void, icon_name: *const c_char);
    fn xapp_status_icon_set_tooltip_text(icon: *mut c_void, text: *const c_char);
    fn xapp_status_icon_set_visible(icon: *mut c_void, visible: glib::ffi::gboolean);
    fn xapp_status_icon_set_secondary_menu(icon: *mut c_void, menu: *mut c_void);

    fn xapp_style_manager_new() -> *mut glib::gobject_ffi::GObject;
    fn xapp_style_manager_set_widget(manager: *mut c_void, widget: *mut c_void);
    fn xapp_style_manager_set_from_pango_font_string(manager: *mut c_void, font: *const c_char);

    fn xapp_preferences_window_new() -> *mut gtk::ffi::GtkWidget;
    fn xapp_preferences_window_add_page(
        window: *mut c_void,
        widget: *mut c_void,
        name: *const c_char,
        title: *const c_char,
    );
}

#[link(name = "gspell-1")]
extern "C" {
    fn gspell_text_view_get_from_gtk_text_view(view: *mut c_void) -> *mut glib::gobject_ffi::GObject;
    fn gspell_text_view_basic_setup(view: *mut c_void);
}

fn cstr(s: &str) -> CString {
    // Strings coming from the UI never contain NUL bytes; strip any just in case.
    CString::new(s.replace('\0', "")).unwrap()
}

fn raw<T: IsA<glib::Object>>(obj: &T) -> *mut c_void {
    obj.as_ref().as_ptr() as *mut c_void
}

/// `XAppStatusIcon`: a tray icon that works on every Mint desktop.
#[derive(Clone)]
pub struct StatusIcon(glib::Object);

impl StatusIcon {
    pub fn new() -> Self {
        unsafe { StatusIcon(from_glib_full(xapp_status_icon_new())) }
    }

    pub fn set_name(&self, name: &str) {
        unsafe { xapp_status_icon_set_name(raw(&self.0), cstr(name).as_ptr()) }
    }

    pub fn set_icon_name(&self, name: &str) {
        unsafe { xapp_status_icon_set_icon_name(raw(&self.0), cstr(name).as_ptr()) }
    }

    pub fn set_tooltip_text(&self, text: &str) {
        unsafe { xapp_status_icon_set_tooltip_text(raw(&self.0), cstr(text).as_ptr()) }
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe { xapp_status_icon_set_visible(raw(&self.0), visible.into_glib()) }
    }

    pub fn set_secondary_menu(&self, menu: &gtk::Menu) {
        unsafe { xapp_status_icon_set_secondary_menu(raw(&self.0), raw(menu)) }
    }

    /// `button-press-event (x, y, button, time, panel_position)`.
    /// The callback gets the button and the event time.
    pub fn connect_button_press<F: Fn(u32, u32) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.0.connect_local("button-press-event", false, move |values| {
            let button = values.get(3).and_then(|v| v.get::<u32>().ok()).unwrap_or(0);
            let time = values.get(4).and_then(|v| v.get::<u32>().ok()).unwrap_or(0);
            f(button, time);
            None
        })
    }
}

/// `XAppStyleManager`: applies a Pango font description to a widget via CSS.
pub struct StyleManager(glib::Object);

impl StyleManager {
    pub fn new<W: IsA<gtk::Widget> + IsA<glib::Object>>(widget: &W) -> Self {
        unsafe {
            let manager: glib::Object = from_glib_full(xapp_style_manager_new());
            xapp_style_manager_set_widget(raw(&manager), raw(widget));
            StyleManager(manager)
        }
    }

    pub fn set_from_pango_font_string(&self, font: &str) {
        unsafe { xapp_style_manager_set_from_pango_font_string(raw(&self.0), cstr(font).as_ptr()) }
    }
}

/// `XAppPreferencesWindow`: a window with a sidebar of settings pages.
pub struct PreferencesWindow(gtk::Window);

impl PreferencesWindow {
    pub fn new() -> Self {
        unsafe {
            let widget: gtk::Widget = from_glib_none(xapp_preferences_window_new());
            PreferencesWindow(widget.unsafe_cast())
        }
    }

    pub fn window(&self) -> &gtk::Window {
        &self.0
    }

    pub fn add_page<W: IsA<gtk::Widget> + IsA<glib::Object>>(&self, page: &W, name: &str, title: &str) {
        unsafe {
            xapp_preferences_window_add_page(
                raw(&self.0),
                raw(page),
                cstr(name).as_ptr(),
                cstr(title).as_ptr(),
            )
        }
    }
}

/// Turn on gspell for a text view and return the `GspellTextView` object so
/// that its `inline-spell-checking` property can be bound to a setting.
pub fn setup_spell_checking(view: &gtk::TextView) -> glib::Object {
    unsafe {
        let spell: glib::Object = from_glib_none(gspell_text_view_get_from_gtk_text_view(raw(view)));
        gspell_text_view_basic_setup(raw(&spell));
        spell
    }
}

