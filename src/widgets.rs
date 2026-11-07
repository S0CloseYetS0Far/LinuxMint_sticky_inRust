//! Small shared widgets: the hover box and the yes/no confirmation dialog.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::i18n::gettext;

struct HoverState {
    child: Option<gtk::Widget>,
    inhibited: bool,
}

/// An event box that only shows one of its descendants (an "edit" button)
/// while the mouse pointer is over it.
#[derive(Clone)]
pub struct HoverBox {
    pub event_box: gtk::EventBox,
    state: Rc<RefCell<HoverState>>,
}

impl HoverBox {
    pub fn new() -> Self {
        let event_box = gtk::EventBox::new();
        let hover = HoverBox { event_box, state: Rc::new(RefCell::new(HoverState { child: None, inhibited: false })) };

        for enter in [true, false] {
            let h = hover.clone_weak();
            let handler = move |_: &gtk::EventBox, _: &gdk::EventCrossing| {
                if let Some(h) = h.upgrade() {
                    h.update_visibility();
                }
                glib::Propagation::Proceed
            };
            if enter {
                hover.event_box.connect_enter_notify_event(handler);
            } else {
                hover.event_box.connect_leave_notify_event(handler);
            }
        }

        hover
    }

    fn clone_weak(&self) -> WeakHoverBox {
        WeakHoverBox { event_box: self.event_box.downgrade(), state: Rc::downgrade(&self.state) }
    }

    pub fn add<W: IsA<gtk::Widget>>(&self, widget: &W) {
        self.event_box.add(widget);
    }

    pub fn set_child_widget<W: IsA<gtk::Widget>>(&self, widget: &W) {
        widget.set_no_show_all(true);
        widget.hide();
        self.state.borrow_mut().child = Some(widget.clone().upcast());
    }

    pub fn disable(&self) {
        self.state.borrow_mut().inhibited = true;
        self.update_visibility();
    }

    pub fn enable(&self) {
        self.state.borrow_mut().inhibited = false;
        self.update_visibility();
    }

    fn has_mouse(&self) -> bool {
        let Some(own) = self.event_box.window() else { return false };
        let Some(pointer) = self.event_box.display().default_seat().and_then(|s| s.pointer()) else {
            return false;
        };
        let (window, _, _) = pointer.window_at_position();
        match window {
            Some(w) => w == own || own.children().contains(&w),
            None => false,
        }
    }

    pub fn update_visibility(&self) {
        let state = self.state.borrow();
        let Some(child) = state.child.clone() else { return };
        let show = !state.inhibited && self.has_mouse();
        drop(state);
        if show {
            child.show();
        } else {
            child.hide();
        }
    }
}

struct WeakHoverBox {
    event_box: glib::WeakRef<gtk::EventBox>,
    state: std::rc::Weak<RefCell<HoverState>>,
}

impl WeakHoverBox {
    fn upgrade(&self) -> Option<HoverBox> {
        Some(HoverBox { event_box: self.event_box.upgrade()?, state: self.state.upgrade()? })
    }
}

/// Where a confirmation dialog stores "Don't ask me again".
pub struct DisableKey<'a> {
    pub settings: &'a gio::Settings,
    pub key: &'a str,
}

/// Ask a yes/no question. Returns true for "Yes".
pub fn confirm(title: &str, message: &str, window: Option<&gtk::Window>, disable: Option<DisableKey<'_>>) -> bool {
    let dialog = gtk::Dialog::new();
    dialog.set_title(title);
    dialog.set_transient_for(window);
    dialog.set_position(if window.is_none() {
        gtk::WindowPosition::Center
    } else {
        gtk::WindowPosition::CenterOnParent
    });
    dialog.add_button(&gettext("No"), gtk::ResponseType::No);
    dialog.add_button(&gettext("Yes"), gtk::ResponseType::Yes);
    dialog.set_default_response(gtk::ResponseType::Yes);

    let content = dialog.content_area();
    content.set_margin_start(20);
    content.set_margin_end(20);
    content.pack_start(&gtk::Label::new(Some(message)), false, false, 10);

    if let Some(disable) = disable {
        let checkbox = gtk::CheckButton::with_label(&gettext("Don't ask me again"));
        content.pack_start(&checkbox, false, false, 0);
        disable.settings.bind(disable.key, &checkbox, "active").build();
    }

    content.show_all();
    let response = dialog.run();
    dialog.close();
    unsafe { dialog.destroy() };

    response == gtk::ResponseType::Yes
}

/// Show an error message with a Close button.
pub fn show_error(message: &str, window: Option<&gtk::Window>) {
    let dialog = gtk::MessageDialog::new(
        window,
        gtk::DialogFlags::empty(),
        gtk::MessageType::Other,
        gtk::ButtonsType::Close,
        message,
    );
    dialog.run();
    unsafe { dialog.destroy() };
}
