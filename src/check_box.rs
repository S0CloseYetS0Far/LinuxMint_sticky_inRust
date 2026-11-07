//! The check button used inside notes. It gets an extra input-only GdkWindow
//! so the mouse pointer shows the normal arrow over it instead of the text
//! cursor of the surrounding text view.
//!
//! The Python version did this by overriding the realize/map/unmap and
//! size_allocate virtual methods; gtk-rs cannot subclass GtkCheckButton, so
//! the same work is done from the matching signals, which run right after
//! the default handlers.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

pub fn new(checked: bool) -> gtk::CheckButton {
    let check = gtk::CheckButton::new();
    check.set_active(checked);
    check.set_visible(true);
    check.set_margin_end(5);
    check.set_margin_top(5);

    let event_window: Rc<RefCell<Option<gdk::Window>>> = Rc::default();

    let ew = event_window.clone();
    check.connect_realize(move |widget| {
        let Some(parent_window) = widget.parent_window() else { return };

        let allocation = widget.allocation();
        let attributes = gdk::WindowAttr {
            window_type: gdk::WindowType::Child,
            x: Some(allocation.x()),
            y: Some(allocation.y()),
            width: allocation.width(),
            height: allocation.height(),
            wclass: gdk::WindowWindowClass::InputOnly,
            event_mask: widget.events()
                | gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::TOUCH_MASK
                | gdk::EventMask::ENTER_NOTIFY_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK
                | gdk::EventMask::POINTER_MOTION_MASK,
            cursor: gdk::Cursor::from_name(&widget.display(), "default"),
            ..Default::default()
        };

        let window = gdk::Window::new(Some(&parent_window), &attributes);
        widget.register_window(&window);
        *ew.borrow_mut() = Some(window);
    });

    let ew = event_window.clone();
    check.connect_unrealize(move |widget| {
        if let Some(window) = ew.borrow_mut().take() {
            widget.unregister_window(&window);
            window.destroy();
        }
    });

    let ew = event_window.clone();
    check.connect_map(move |_| {
        if let Some(window) = ew.borrow().as_ref() {
            window.show();
        }
    });

    let ew = event_window.clone();
    check.connect_unmap(move |_| {
        if let Some(window) = ew.borrow().as_ref() {
            window.hide();
        }
    });

    check.connect_size_allocate(move |_, allocation| {
        if let Some(window) = event_window.borrow().as_ref() {
            window.move_resize(allocation.x(), allocation.y(), allocation.width(), allocation.height());
        }
    });

    check
}
