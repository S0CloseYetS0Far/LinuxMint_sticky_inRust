//! Sticky: sticky notes for the Linux desktop.

mod app;
mod check_box;
mod ffi;
mod file_handler;
mod i18n;
mod manager;
mod note;
mod note_buffer;
mod notes;
mod settings_window;
mod signal;
mod util;
mod widgets;

fn main() -> glib::ExitCode {
    i18n::init();
    let app = app::App::new();
    app.run()
}

/// Everything that needs GTK runs in this one test, because GTK may only be
/// used from the thread that initialized it. Skipped without a display.
#[cfg(test)]
#[test]
fn gtk_tests() {
    if gtk::init().is_err() {
        eprintln!("no display, skipping the GTK tests");
        return;
    }
    note_buffer::tests::buffer_behaviour();
    settings_window::tests::windows_build();
}
