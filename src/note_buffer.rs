//! The text buffer behind every note: formatting tags, checkboxes, bullets,
//! links, undo/redo and conversion to and from the internal markup format.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use glib::translate::{from_glib_none, IntoGlib, ToGlibPtr};

use gtk::prelude::*;
use crate::signal::Signal;

use crate::check_box;
use crate::util::{get_url_start, parse_markup, Token};

/// (name, apply attributes)
const TAG_NAMES: &[&str] = &[
    "bold",
    "italic",
    "monospace",
    "underline",
    "strikethrough",
    "highlight",
    "link",
    "header",
    "small",
    "large",
    "larger",
];

pub const FONT_SCALES: &[&str] = &["small", "normal", "large", "larger"];

const LINK_ENDERS: &[&str] = &["\n", "\t", " ", ".", ",", ";", ":"];

fn char_len(s: &str) -> i32 {
    s.chars().count() as i32
}

fn create_tag(buffer: &gtk::TextBuffer, name: &str) -> gtk::TextTag {
    let tag = gtk::TextTag::new(Some(name));
    buffer.tag_table().expect("tag table").add(&tag);
    match name {
        "bold" => tag.set_weight(pango::Weight::Bold.into_glib()),
        "italic" => tag.set_style(pango::Style::Italic),
        "monospace" => tag.set_family(Some("Monospace")),
        "underline" => tag.set_underline(pango::Underline::Single),
        "strikethrough" => tag.set_strikethrough(true),
        "highlight" => tag.set_background(Some("yellow")),
        "link" => {
            tag.set_underline(pango::Underline::Single);
            tag.set_foreground(Some("blue"));
        }
        "header" => {
            tag.set_scale(1.5);
            tag.set_weight(500);
            tag.set_pixels_above_lines(14);
            tag.set_pixels_below_lines(4);
        }
        "small" => tag.set_scale(0.8333333333333),
        "large" => tag.set_scale(1.2),
        "larger" => tag.set_scale(1.44),
        _ => {}
    }
    tag
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorType {
    Check,
    Bullet,
}

fn anchor_type(anchor: &gtk::TextChildAnchor) -> Option<AnchorType> {
    let widget = anchor.widgets().into_iter().next()?;
    if widget.is::<gtk::CheckButton>() {
        Some(AnchorType::Check)
    } else if widget.is::<gtk::Image>() {
        Some(AnchorType::Bullet)
    } else {
        None
    }
}

fn anchor_checked(anchor: &gtk::TextChildAnchor) -> bool {
    anchor
        .widgets()
        .into_iter()
        .next()
        .and_then(|w| w.downcast::<gtk::ToggleButton>().ok())
        .map(|b| b.is_active())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionType {
    Selection,
    Backward,
    Forward,
    Other,
}

/// One undoable change.
#[derive(Debug, Clone)]
pub enum Action {
    /// Plain text added to the buffer.
    Addition { text: String, position: i32 },
    /// Text removed from the buffer.
    Deletion { text: String, position: i32, kind: DeletionType },
    /// A checkbox or bullet inserted (or removed) at an anchor point.
    Object { position: i32, anchor_type: AnchorType, is_addition: bool, checked: bool },
    /// A formatting tag set (or removed) on some ranges.
    Tag { name: String, is_addition: bool, ranges: Vec<(i32, i32)> },
    /// Lines moved up or down.
    Shift { start: i32, end: i32, is_up: bool },
    /// Several actions undone and redone together, in order.
    Composite(Vec<Action>),
}

impl Action {

    fn deletion(buffer: &NoteBuffer, start: &gtk::TextIter, end: &gtk::TextIter) -> Action {
        let text = buffer.slice(start, end, true).map(|s| s.to_string()).unwrap_or_default();
        let insert = buffer.iter_at_mark(&buffer.get_insert().unwrap());
        let kind = if let Some((bs, be)) = buffer.selection_bounds() {
            if bs == *start && be != *end {
                DeletionType::Selection
            } else {
                DeletionType::Other
            }
        } else if insert == *end {
            DeletionType::Backward
        } else if insert == *start {
            DeletionType::Forward
        } else {
            DeletionType::Other
        };
        Action::Deletion { text, position: start.offset(), kind }
    }

    fn object(buffer: &NoteBuffer, anchor: &gtk::TextChildAnchor, is_addition: bool) -> Option<Action> {
        Some(Action::Object {
            position: buffer.iter_at_child_anchor(anchor).offset(),
            anchor_type: anchor_type(anchor)?,
            is_addition,
            checked: false,
        })
    }

    fn tag(buffer: &NoteBuffer, name: &str, start: &gtk::TextIter, end: &gtk::TextIter, is_addition: bool) -> Action {
        // there may be text between `start` and `end` that already has the tag applied (or doesn't, in the case of a
        // removal), and we need to find those ranges (if they exist) so we can restore that state properly when
        // undoing
        let tag = buffer.lookup(name);
        let mut current = *start;
        let mut range_start: Option<i32> = None;
        let mut ranges = Vec::new();
        while current < *end {
            let has_tag = tag.as_ref().map(|t| current.has_tag(t)).unwrap_or(false);
            // if we're adding a tag, we only want to change text that isn't already tagged
            // if it's a removal, we only want to change text that is already tagged
            if range_start.is_none() && has_tag != is_addition {
                range_start = Some(current.offset());
            } else if let Some(rs) = range_start.filter(|_| has_tag == is_addition) {
                ranges.push((rs, current.offset()));
                range_start = None;
            }
            if !current.forward_char() {
                break;
            }
        }
        if let Some(rs) = range_start {
            ranges.push((rs, end.offset()));
        }
        Action::Tag { name: name.to_string(), is_addition, ranges }
    }

    fn tag_ranges(&self) -> usize {
        match self {
            Action::Tag { ranges, .. } => ranges.len(),
            _ => 0,
        }
    }

    pub fn undo(&mut self, buffer: &NoteBuffer) {
        match self {
            Action::Addition { text, position } => {
                let mut start = buffer.iter_at_offset(*position);
                let mut end = buffer.iter_at_offset(*position + char_len(text));
                buffer.delete(&mut start, &mut end);

                // we generally want to put the cursor where the text was added/removed
                let pos = buffer.iter_at_offset(*position);
                buffer.select_range(&pos, &pos);
            }
            Action::Deletion { text, position, kind } => {
                buffer.insert(&mut buffer.iter_at_offset(*position), text);

                // depending on the type of deletion, we want to put the cursor (and selection) in different places
                match kind {
                    DeletionType::Forward => {
                        let pos = buffer.iter_at_offset(*position);
                        buffer.select_range(&pos, &pos);
                    }
                    DeletionType::Backward => {
                        let pos = buffer.iter_at_offset(*position + char_len(text));
                        buffer.select_range(&pos, &pos);
                    }
                    _ => {
                        // any other type of deletion, we probably want to just select the whole thing
                        let a = buffer.iter_at_offset(*position);
                        let b = buffer.iter_at_offset(*position + char_len(text));
                        buffer.select_range(&a, &b);
                    }
                }
            }
            Action::Object { is_addition, .. } => {
                if *is_addition {
                    self.object_remove(buffer)
                } else {
                    self.object_add(buffer)
                }
            }
            Action::Tag { is_addition, .. } => {
                if *is_addition {
                    self.tag_remove(buffer)
                } else {
                    self.tag_add(buffer)
                }
            }
            Action::Shift { is_up, .. } => {
                if *is_up {
                    self.shift_down(buffer)
                } else {
                    self.shift_up(buffer)
                }
            }
            Action::Composite(actions) => {
                for action in actions.iter_mut().rev() {
                    action.undo(buffer);
                }
            }
        }
    }

    pub fn redo(&mut self, buffer: &NoteBuffer) {
        match self {
            Action::Addition { text, position } => {
                buffer.insert(&mut buffer.iter_at_offset(*position), text);

                // we generally want to put the cursor at the end of the re-added text
                let pos = buffer.iter_at_offset(*position + char_len(text));
                buffer.select_range(&pos, &pos);
            }
            Action::Deletion { text, position, .. } => {
                let mut start = buffer.iter_at_offset(*position);
                let mut end = buffer.iter_at_offset(*position + char_len(text));
                buffer.delete(&mut start, &mut end);

                // we usually want to put the cursor at the point of deletion
                let pos = buffer.iter_at_offset(*position);
                buffer.select_range(&pos, &pos);
            }
            Action::Object { is_addition, .. } => {
                if *is_addition {
                    self.object_add(buffer)
                } else {
                    self.object_remove(buffer)
                }
            }
            Action::Tag { is_addition, .. } => {
                if *is_addition {
                    self.tag_add(buffer)
                } else {
                    self.tag_remove(buffer)
                }
            }
            Action::Shift { is_up, .. } => {
                if *is_up {
                    self.shift_up(buffer)
                } else {
                    self.shift_down(buffer)
                }
            }
            Action::Composite(actions) => {
                for action in actions.iter_mut() {
                    action.redo(buffer);
                }
            }
        }
    }

    pub fn maybe_join(&mut self, new_action: &Action) -> bool {
        match (self, new_action) {
            (Action::Addition { text, position }, Action::Addition { text: new_text, position: new_position }) => {
                if new_text == "\n" {
                    return false;
                }
                if *new_position == *position + char_len(text) {
                    text.push_str(new_text);
                    return true;
                }
                false
            }
            (
                Action::Deletion { text, position, kind },
                Action::Deletion { text: new_text, position: new_position, kind: new_kind },
            ) => {
                if kind != new_kind {
                    return false;
                }
                if *kind == DeletionType::Forward && *new_position == *position {
                    text.push_str(new_text);
                    true
                } else if *kind == DeletionType::Backward && *new_position == *position - 1 {
                    *text = format!("{new_text}{text}");
                    *position = *new_position;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    // ObjectInsertAction

    fn object_remove(&mut self, buffer: &NoteBuffer) {
        let Action::Object { position, anchor_type, checked, .. } = self else { return };
        let mut start = buffer.iter_at_offset(*position);
        let mut end = buffer.iter_at_offset(*position + 1);
        if *anchor_type == AnchorType::Check {
            if let Some(anchor) = start.child_anchor() {
                *checked = anchor_checked(&anchor);
            }
        }
        buffer.delete(&mut start, &mut end);
    }

    fn object_add(&mut self, buffer: &NoteBuffer) {
        let Action::Object { position, anchor_type, checked, .. } = self else { return };
        let mut iter = buffer.iter_at_offset(*position);
        match anchor_type {
            AnchorType::Check => {
                buffer.add_check_button(&mut iter, *checked);
            }
            AnchorType::Bullet => {
                buffer.add_bullet(&mut iter);
            }
        }
    }

    // TagAction

    fn tag_remove(&self, buffer: &NoteBuffer) {
        let Action::Tag { name, ranges, .. } = self else { return };
        for (start, end) in ranges {
            buffer.remove_tag_by_name(name, &buffer.iter_at_offset(*start), &buffer.iter_at_offset(*end));
        }
    }

    fn tag_add(&self, buffer: &NoteBuffer) {
        let Action::Tag { name, ranges, .. } = self else { return };
        for (start, end) in ranges {
            buffer.apply_tag_by_name(name, &buffer.iter_at_offset(*start), &buffer.iter_at_offset(*end));
        }
    }

    // ShiftAction

    fn copy_anchor(buffer: &NoteBuffer, from: &gtk::TextMark, to: &gtk::TextMark) {
        if let Some(anchor) = buffer.iter_at_mark(from).child_anchor() {
            match anchor_type(&anchor) {
                Some(AnchorType::Check) => {
                    let checked = anchor_checked(&anchor);
                    buffer.add_check_button(&mut buffer.iter_at_mark(to), checked);
                }
                Some(AnchorType::Bullet) => {
                    buffer.add_bullet(&mut buffer.iter_at_mark(to));
                }
                None => {}
            }
        }
    }

    fn shift_up(&mut self, buffer: &NoteBuffer) {
        let Action::Shift { start, end, .. } = self else { return };
        let start_iter = buffer.iter_at_line(*start);
        let mut end_iter = buffer.iter_at_line(*end);
        end_iter.forward_to_line_end();
        let start_mark = buffer.create_mark(None, &start_iter, false).unwrap();
        let end_mark = buffer.create_mark(None, &end_iter, true).unwrap();

        let mut move_start = start_iter;
        move_start.backward_line();
        let mut move_end = move_start;
        move_end.forward_to_line_end();

        let move_start_mark = buffer.create_mark(None, &move_start, false).unwrap();
        let move_end_mark = buffer.create_mark(None, &move_end, true).unwrap();

        buffer.insert_range(&mut end_iter, &move_start, &move_end);
        Self::copy_anchor(buffer, &move_start_mark, &end_mark);

        buffer.insert(&mut buffer.iter_at_mark(&end_mark), "\n");
        let mut delete_start = buffer.iter_at_mark(&move_start_mark);
        let mut delete_end = buffer.iter_at_mark(&move_end_mark);
        delete_end.forward_char();
        buffer.delete(&mut delete_start, &mut delete_end);

        // since it gets really messy trying to restore the previous cursor position, just select the lines that are
        // moving
        buffer.select_range(&buffer.iter_at_mark(&start_mark), &buffer.iter_at_mark(&end_mark));

        for mark in [&move_start_mark, &move_end_mark, &start_mark, &end_mark] {
            buffer.delete_mark(mark);
        }

        *start -= 1;
        *end -= 1;
    }

    fn shift_down(&mut self, buffer: &NoteBuffer) {
        let Action::Shift { start, end, .. } = self else { return };
        let mut start_iter = buffer.iter_at_line(*start);
        let mut end_iter = buffer.iter_at_line(*end);
        end_iter.forward_to_line_end();
        let begin_mark = buffer.create_mark(None, &start_iter, true).unwrap();
        let start_mark = buffer.create_mark(None, &start_iter, false).unwrap();
        let end_mark = buffer.create_mark(None, &end_iter, true).unwrap();

        let mut move_start = end_iter;
        move_start.forward_line();
        let mut move_end = move_start;
        move_end.forward_to_line_end();

        let move_start_mark = buffer.create_mark(None, &move_start, false).unwrap();
        let move_end_mark = buffer.create_mark(None, &move_end, true).unwrap();

        buffer.insert_range(&mut start_iter, &move_start, &move_end);
        buffer.insert(&mut buffer.iter_at_mark(&start_mark), "\n");
        Self::copy_anchor(buffer, &move_start_mark, &begin_mark);

        let mut delete_start = buffer.iter_at_mark(&move_start_mark);
        delete_start.backward_char();
        let mut delete_end = buffer.iter_at_mark(&move_end_mark);
        buffer.delete(&mut delete_start, &mut delete_end);

        // since it gets really messy trying to restore the previous cursor position, just select the lines that are
        // moving
        buffer.select_range(&buffer.iter_at_mark(&start_mark), &buffer.iter_at_mark(&end_mark));

        for mark in [&move_start_mark, &move_end_mark, &start_mark, &end_mark, &begin_mark] {
            buffer.delete_mark(mark);
        }

        *start += 1;
        *end += 1;
    }
}

fn composite(a: Action, b: Action) -> Action {
    Action::Composite(vec![a, b])
}

fn composite_of(mut actions: Vec<Action>) -> Action {
    if actions.len() == 1 {
        actions.pop().unwrap()
    } else {
        Action::Composite(actions)
    }
}

/// Pointer to the `GtkTextIter` carried by a signal argument, so a handler can
/// update it in place like a C handler would.
fn iter_ptr(value: &glib::Value) -> *mut gtk::ffi::GtkTextIter {
    unsafe { glib::gobject_ffi::g_value_get_boxed(value.to_glib_none().0) as *mut gtk::ffi::GtkTextIter }
}

fn read_iter(ptr: *mut gtk::ffi::GtkTextIter) -> gtk::TextIter {
    unsafe { from_glib_none(ptr as *const gtk::ffi::GtkTextIter) }
}

fn write_iter(ptr: *mut gtk::ffi::GtkTextIter, iter: &gtk::TextIter) {
    let src: *const gtk::ffi::GtkTextIter = iter.to_glib_none().0;
    unsafe { *ptr = *src }
}

/// The text of an `insert-text` emission, which is not necessarily NUL
/// terminated, so its byte length argument must be used.
fn insert_text_arg(text: &glib::Value, len: &glib::Value) -> String {
    let len = len.get::<i32>().unwrap_or(-1);
    unsafe {
        let ptr = glib::gobject_ffi::g_value_get_string(text.to_glib_none().0);
        if ptr.is_null() {
            return String::new();
        }
        let bytes = if len < 0 {
            std::ffi::CStr::from_ptr(ptr).to_bytes()
        } else {
            std::slice::from_raw_parts(ptr as *const u8, len as usize)
        };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub struct Inner {
    buffer: gtk::TextBuffer,
    /// Keeps "content-changed" from firing while the buffer performs several actions. Use
    /// `NoteBuffer::internal_action()` instead of modifying it directly.
    internal_action_count: Cell<u32>,
    in_composite: Cell<u32>,
    composite_actions: RefCell<Vec<Action>>,
    undo_actions: RefCell<Vec<Action>>,
    redo_actions: RefCell<Vec<Action>>,
    /// Tags toggled while nothing is selected, applied to the next typed character.
    tag_toggles: RefCell<Vec<String>>,
    tags: Vec<gtk::TextTag>,
    view: glib::WeakRef<gtk::TextView>,
    content_changed: Signal<()>,
}

/// A `GtkTextBuffer` plus the state Sticky keeps for it. Derefs to the
/// buffer, so all `TextBufferExt` methods can be called on it directly.
#[derive(Clone)]
pub struct NoteBuffer(Rc<Inner>);

#[derive(Clone)]
pub struct WeakNoteBuffer(Weak<Inner>);

impl WeakNoteBuffer {
    pub fn upgrade(&self) -> Option<NoteBuffer> {
        self.0.upgrade().map(NoteBuffer)
    }
}

impl std::ops::Deref for NoteBuffer {
    type Target = gtk::TextBuffer;
    fn deref(&self) -> &gtk::TextBuffer {
        &self.0.buffer
    }
}

/// Decrements the internal action counter when dropped.
pub struct InternalAction<'a> {
    buffer: &'a NoteBuffer,
    trigger_on_complete: bool,
}

impl Drop for InternalAction<'_> {
    fn drop(&mut self) {
        let inner = &self.buffer.0;
        inner.internal_action_count.set(inner.internal_action_count.get() - 1);
        if inner.internal_action_count.get() == 0 && self.trigger_on_complete {
            let weak = self.buffer.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(b) = weak.upgrade() {
                    b.trigger_changed();
                }
            });
        }
    }
}

impl NoteBuffer {
    pub fn new() -> Self {
        let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
        let tags = TAG_NAMES.iter().map(|name| create_tag(&buffer, name)).collect();
        let note_buffer = NoteBuffer(Rc::new(Inner {
            buffer: buffer.clone(),
            internal_action_count: Cell::new(0),
            in_composite: Cell::new(0),
            composite_actions: RefCell::default(),
            undo_actions: RefCell::default(),
            redo_actions: RefCell::default(),
            tag_toggles: RefCell::default(),
            tags,
            view: glib::WeakRef::new(),
            content_changed: Signal::default(),
        }));

        // Runs after the default handler has inserted the text.
        let weak = note_buffer.downgrade();
        buffer.connect_local("insert-text", true, move |values| {
            if let Some(b) = weak.upgrade() {
                let ptr = iter_ptr(&values[1]);
                let text = insert_text_arg(&values[2], &values[3]);
                let mut location = read_iter(ptr);
                b.do_insert_text(&mut location, &text);
                write_iter(ptr, &location);
            }
            None
        });

        // Runs before the default handler deletes the range.
        let weak = note_buffer.downgrade();
        buffer.connect_local("delete-range", false, move |values| {
            if let Some(b) = weak.upgrade() {
                let (start_ptr, end_ptr) = (iter_ptr(&values[1]), iter_ptr(&values[2]));
                let (mut start, mut end) = (read_iter(start_ptr), read_iter(end_ptr));
                if b.on_delete(&mut start, &mut end) {
                    write_iter(start_ptr, &start);
                    write_iter(end_ptr, &end);
                }
            }
            None
        });

        let weak = note_buffer.downgrade();
        buffer.connect_begin_user_action(move |_| {
            if let Some(b) = weak.upgrade() {
                b.0.in_composite.set(b.0.in_composite.get() + 1);
            }
        });
        let weak = note_buffer.downgrade();
        buffer.connect_end_user_action(move |_| {
            if let Some(b) = weak.upgrade() {
                b.end_composite_action();
            }
        });
        let weak = note_buffer.downgrade();
        buffer.connect_mark_set(move |_, _, mark| {
            if mark.name().as_deref() == Some("insert") {
                if let Some(b) = weak.upgrade() {
                    b.0.tag_toggles.borrow_mut().clear();
                }
            }
        });

        note_buffer
    }

    pub fn downgrade(&self) -> WeakNoteBuffer {
        WeakNoteBuffer(Rc::downgrade(&self.0))
    }

    pub fn text_buffer(&self) -> &gtk::TextBuffer {
        &self.0.buffer
    }

    pub fn set_view(&self, view: &gtk::TextView) {
        self.0.view.set(Some(view));

        view.add_events(gdk::EventMask::POINTER_MOTION_MASK);

        let weak = self.downgrade();
        view.connect_motion_notify_event(move |view, event| {
            let Some(buffer) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !view.modifier_mask(gdk::ModifierIntent::ExtendSelection).is_empty() {
                return glib::Propagation::Proceed;
            }
            let (x, y) = event.position();
            let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Text, x as i32, y as i32);
            if let (Some(iter), Some(link)) = (view.iter_at_location(bx, by), buffer.lookup("link")) {
                if iter.has_tag(&link) {
                    if let Some(window) = TextViewExt::window(view, gtk::TextWindowType::Text) {
                        window.set_cursor(gdk::Cursor::from_name(&view.display(), "pointer").as_ref());
                    }
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });

        let weak = self.downgrade();
        view.connect_button_press_event(move |view, event| {
            let Some(buffer) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !event.state().contains(gdk::ModifierType::CONTROL_MASK) || event.button() != 1 {
                return glib::Propagation::Proceed;
            }
            let Some(link) = buffer.lookup("link") else { return glib::Propagation::Proceed };
            let (x, y) = event.position();
            let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Text, x as i32, y as i32);
            let Some(iter) = view.iter_at_location(bx, by) else { return glib::Propagation::Proceed };
            if !iter.has_tag(&link) {
                return glib::Propagation::Proceed;
            }

            let mut start = iter;
            start.backward_to_tag_toggle(Some(&link));
            let mut end = iter;
            end.forward_to_tag_toggle(Some(&link));

            let url = buffer.slice(&start, &end, false).unwrap_or_default();
            let window = view.toplevel().and_then(|t| t.downcast::<gtk::Window>().ok());
            if let Err(e) = gtk::show_uri_on_window(window.as_ref(), &url, event.time()) {
                eprintln!("sticky: could not open {url}: {e}");
            }
            glib::Propagation::Stop
        });
    }

    fn view(&self) -> Option<gtk::TextView> {
        self.0.view.upgrade()
    }

    pub fn connect_content_changed<F: Fn() + 'static>(&self, f: F) -> crate::signal::HandlerId {
        self.0.content_changed.connect(move |_| f())
    }

    pub fn trigger_changed(&self) {
        if self.0.internal_action_count.get() == 0 {
            self.0.content_changed.emit(&());
        }
    }

    pub fn internal_action(&self, trigger_on_complete: bool) -> InternalAction<'_> {
        let imp = &self.0;
        imp.internal_action_count.set(imp.internal_action_count.get() + 1);
        InternalAction { buffer: self, trigger_on_complete }
    }

    pub fn can_undo(&self) -> bool {
        !self.0.undo_actions.borrow().is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.0.redo_actions.borrow().is_empty()
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<gtk::TextTag> {
        self.tag_table().and_then(|t| t.lookup(name))
    }

    fn has_named_tag(&self, iter: &gtk::TextIter, name: &str) -> bool {
        self.lookup(name).map(|t| iter.has_tag(&t)).unwrap_or(false)
    }

    pub fn get_internal_markup(&self) -> String {
        let tags = self.0.tags.clone();
        let mut on_tags: Vec<gtk::TextTag> = Vec::new();
        let mut off_tags = tags;
        let mut text = String::new();
        let tag_name = |t: &gtk::TextTag| t.name().map(|n| n.to_string()).unwrap_or_default();

        let mut current = self.iter_at_offset(0);
        loop {
            // first we close any open tags that don't continue on to the current character
            for tag in on_tags.clone() {
                if !current.has_tag(&tag) {
                    text.push_str(&format!("#tag:{}:", tag_name(&tag)));
                    on_tags.retain(|t| t != &tag);
                    off_tags.push(tag);
                }
            }

            // next we open any tags that start with the current character
            for tag in off_tags.clone() {
                if current.has_tag(&tag) {
                    text.push_str(&format!("#tag:{}:", tag_name(&tag)));
                    off_tags.retain(|t| t != &tag);
                    on_tags.push(tag);
                }
            }

            if let Some(anchor) = current.child_anchor() {
                // object insertions (bullets and checkboxes)
                match anchor_type(&anchor) {
                    Some(AnchorType::Check) => {
                        text.push_str(&format!("#check:{}", anchor_checked(&anchor) as i32));
                    }
                    Some(AnchorType::Bullet) => text.push_str("#bullet:"),
                    None => {}
                }
            } else if let Some(c) = current.char().filter(|_| !current.is_end()) {
                if c == '#' {
                    // we need to escape '#' characters to avoid misinterpretation when we parse it later
                    text.push_str("##");
                } else {
                    text.push(c);
                }
            }

            if !current.forward_char() {
                break;
            }
        }

        // If there are any open tags at this point, it means they go to the end of the text, so close them
        for tag in on_tags {
            text.push_str(&format!("#tag:{}:", tag_name(&tag)));
        }

        text
    }

    pub fn set_from_internal_markup(&self, text: &str) {
        let _guard = self.internal_action(false);
        self.set_text("");

        let mut open_tags: Vec<(String, gtk::TextMark)> = Vec::new();
        for token in parse_markup(text) {
            match token {
                Token::Text(t) => self.insert(&mut self.end_iter(), &t),
                Token::Check(checked) => {
                    self.add_check_button(&mut self.end_iter(), checked);
                }
                Token::Bullet => {
                    self.add_bullet(&mut self.end_iter());
                }
                Token::Tag(name) => {
                    if let Some(pos) = open_tags.iter().position(|(n, _)| *n == name) {
                        let (_, mark) = open_tags.remove(pos);
                        let start = self.iter_at_mark(&mark);
                        if self.lookup(&name).is_some() {
                            self.apply_tag_by_name(&name, &start, &self.end_iter());
                        }
                        self.delete_mark(&mark);
                    } else {
                        let mark = self.create_mark(None, &self.end_iter(), true).unwrap();
                        open_tags.push((name, mark));
                    }
                }
            }
        }
        for (_, mark) in open_tags {
            self.delete_mark(&mark);
        }
    }

    pub fn undo(&self) {
        let Some(mut action) = self.0.undo_actions.borrow_mut().pop() else { return };
        {
            let _guard = self.internal_action(true);
            action.undo(self);
        }
        self.0.redo_actions.borrow_mut().push(action);
    }

    pub fn redo(&self) {
        let Some(mut action) = self.0.redo_actions.borrow_mut().pop() else { return };
        {
            let _guard = self.internal_action(true);
            action.redo(self);
        }
        self.0.undo_actions.borrow_mut().push(action);
    }

    fn end_composite_action(&self) {
        let imp = &self.0;
        imp.in_composite.set(imp.in_composite.get().saturating_sub(1));

        // if there are no actions that happen during the composite, there's nothing we need to do
        if imp.in_composite.get() > 0 || imp.composite_actions.borrow().is_empty() {
            return;
        }

        // some times we get actions tagged as composite when they really shouldn't be, so if there's just one action
        // we don't want to put it inside a composite action as that will break joining actions if applicable
        let actions = std::mem::take(&mut *imp.composite_actions.borrow_mut());
        self.add_undo_action(composite_of(actions));
    }

    pub fn add_undo_action(&self, action: Action) {
        let imp = &self.0;
        if imp.in_composite.get() > 0 {
            imp.composite_actions.borrow_mut().push(action);
        } else {
            imp.undo_actions.borrow_mut().push(action);
            imp.redo_actions.borrow_mut().clear();
        }
    }

    /// Join `action` onto the last undo action if possible, otherwise add it.
    fn join_or_add(&self, action: Action) {
        let joined = self.0.undo_actions.borrow_mut().last_mut().map(|last| last.maybe_join(&action)).unwrap_or(false);
        if !joined {
            self.add_undo_action(action);
        }
    }

    /// Called after the default `insert-text` handler, so `location` points at
    /// the end of the inserted text.
    fn do_insert_text(&self, location: &mut gtk::TextIter, text: &str) {
        if self.0.internal_action_count.get() > 0 {
            return;
        }

        let position = location.offset() - char_len(text);
        let mut action = Action::Addition { text: text.to_string(), position };
        let mut end_offset = position + char_len(text);

        {
            let _guard = self.internal_action(true);
            if text == "\n" {
                // if the previous line starts with a checkbox or bullet, repeat
                let mut next_line = self.iter_at_offset(position + 1);
                let mut prev_line = next_line;
                prev_line.backward_line();

                if let Some(anchor) = prev_line.child_anchor() {
                    let obj_action = match anchor_type(&anchor) {
                        Some(AnchorType::Check) => Some(self.add_check_button(&mut next_line, false)),
                        Some(AnchorType::Bullet) => Some(self.add_bullet(&mut next_line)),
                        None => None,
                    };
                    if let Some(obj_action) = obj_action.flatten() {
                        action = composite(action, obj_action);
                    }

                    // `location` becomes invalidated by adding a check/bullet, but it is expected to be there when
                    // the function ends, so we need to reinitialize it. The default handler places it at the end of
                    // the inserted text, so that's what we'll do too
                    end_offset = position + 2;
                }
            }

            // 'location' has usually been shifted by this point, and since it may be used by something else later
            // that expects the new location, we need to use a different iter
            let start = self.iter_at_offset(position);
            let mut next_char = start;
            next_char.forward_char();

            // check tag toggles
            if text.chars().count() == 1 {
                let mut font_scale_changed = false;
                let toggles = self.0.tag_toggles.borrow().clone();
                for tag_name in &toggles {
                    // if the tag is a font scale, remove all other font scales
                    if FONT_SCALES.contains(&tag_name.as_str()) {
                        font_scale_changed = true;
                        for scale in FONT_SCALES {
                            if *scale != "normal" {
                                action = composite(action, self.strip_tag(scale, &start, &next_char));
                            }
                        }
                    }

                    let mut prev = start;
                    if tag_name == "normal" {
                        // the 'normal' tag doesn't really exist but we still use it for removing other font scale tags
                    } else if prev.backward_char() && self.has_named_tag(&prev, tag_name) {
                        action = composite(action, self.strip_tag(tag_name, &start, &next_char));
                    } else {
                        action = composite(action, self.add_tag(tag_name, &start, &next_char));
                    }
                }

                for tag in start.toggled_tags(false) {
                    let Some(tag_name) = tag.name().map(|n| n.to_string()) else { continue };

                    // don't continue header tag onto new line
                    if text == "\n" && tag_name == "header" {
                        continue;
                    }

                    // don't continue links across link-ending characters
                    if tag_name == "link" && LINK_ENDERS.contains(&text) {
                        continue;
                    }

                    // don't continue the tag if it was toggled off
                    // also don't continue if it's a font scale tag and another was just applied
                    if !toggles.contains(&tag_name) && (!FONT_SCALES.contains(&tag_name.as_str()) || !font_scale_changed) {
                        action = composite(action, self.add_tag(&tag_name, &start, &next_char));
                    }
                }

                self.0.tag_toggles.borrow_mut().clear();
            }

            self.join_or_add(action);
        }

        // check for a link if text is a link-ending character
        if LINK_ENDERS.contains(&text) {
            let start = self.iter_at_offset(position);
            let pre_text = self.slice(&self.start_iter(), &start, true).unwrap_or_default();
            if let Some(url_start) = get_url_start(&pre_text) {
                let link_start = self.iter_at_offset(url_start as i32);
                let action = self.add_tag("link", &link_start, &start);
                self.add_undo_action(action);
            }
        }

        // Every change above invalidated `location`; hand back a valid iter at the end of the insertion.
        *location = self.iter_at_offset(end_offset);
    }

    fn on_delete(&self, start: &mut gtk::TextIter, end: &mut gtk::TextIter) -> bool {
        if self.0.internal_action_count.get() > 0 {
            return false;
        }

        // if there were tags, bullets or checkboxes here, we need to handle those first so that we can undo later
        let mut actions = Vec::new();
        let _guard = self.internal_action(true);
        let mut current = *start;

        // if there's a bullet or checkbox for the next 'character', remove that too
        if end.child_anchor().is_some() {
            end.forward_char();
        }

        let start_mark = self.create_mark(None, start, false).unwrap();
        let end_mark = self.create_mark(None, end, false).unwrap();

        let mut open_tags: Vec<(String, i32)> = Vec::new();
        while current < *end {
            if let Some(anchor) = current.child_anchor() {
                let offset = current.offset();
                if let Some(mut action) = Action::object(self, &anchor, false) {
                    action.object_remove(self);
                    actions.push(action);
                }

                current = self.iter_at_offset(offset);
                *end = self.iter_at_mark(&end_mark);
                *start = self.iter_at_mark(&start_mark);
            }

            for tag in current.toggled_tags(true) {
                // ignore tags that don't have one of our names (i.e. spell checker)
                if let Some(name) = tag.name().filter(|n| TAG_NAMES.contains(&n.as_str())) {
                    let name = name.to_string();
                    open_tags.retain(|(n, _)| *n != name);
                    open_tags.push((name, current.offset()));
                }
            }

            for tag in current.toggled_tags(false) {
                // ignore tags that don't have one of our names (i.e. spell checker)
                let Some(name) = tag.name().filter(|n| TAG_NAMES.contains(&n.as_str())) else { continue };
                let name = name.to_string();

                if let Some(pos) = open_tags.iter().position(|(n, _)| *n == name) {
                    let (_, offset) = open_tags.remove(pos);
                    actions.push(Action::tag(self, &name, &self.iter_at_offset(offset), &current, false));
                } else {
                    actions.push(Action::tag(self, &name, start, &current, false));
                }
            }

            if !current.forward_char() {
                break;
            }
        }

        for (name, offset) in open_tags {
            actions.push(Action::tag(self, &name, &self.iter_at_offset(offset), end, false));
        }

        self.delete_mark(&start_mark);
        self.delete_mark(&end_mark);

        // if it's just an object deletion, there's nothing left to remove, so there's no need to create an undo
        // action
        if *start != *end {
            actions.push(Action::deletion(self, start, end));
        }

        if actions.is_empty() {
            return true;
        }

        self.join_or_add(composite_of(actions));
        true
    }

    pub fn tag_selection(&self, tag_name: &str) {
        if let Some((start, end)) = self.selection_bounds() {
            let mut remove_tag = true;
            if FONT_SCALES.contains(&tag_name) {
                remove_tag = false;
            } else {
                let tag = self.lookup(tag_name);
                let mut current = start;
                while current < end {
                    if !tag.as_ref().map(|t| current.has_tag(t)).unwrap_or(false) {
                        remove_tag = false;
                        break;
                    }
                    current.forward_char();
                }
            }

            // remove the tag if the whole selection already has it (unless it's a font scale tag)
            let action = if remove_tag { self.strip_tag(tag_name, &start, &end) } else { self.add_tag(tag_name, &start, &end) };
            self.add_undo_action(action);
        } else {
            let mut toggles = self.0.tag_toggles.borrow_mut();
            // if it's a font scale, we want to remove any existing font scale first
            // this has the side effect that font scales don't toggle like everything else, which is exactly what we
            // want
            if FONT_SCALES.contains(&tag_name) {
                toggles.retain(|n| !FONT_SCALES.contains(&n.as_str()));
            }

            if let Some(pos) = toggles.iter().position(|n| n == tag_name) {
                toggles.remove(pos);
            } else {
                toggles.push(tag_name.to_string());
            }
        }
    }

    pub fn add_tag(&self, tag_name: &str, start: &gtk::TextIter, end: &gtk::TextIter) -> Action {
        let mut actions = Vec::new();
        if FONT_SCALES.contains(&tag_name) {
            for scale in FONT_SCALES {
                if *scale == tag_name || *scale == "normal" {
                    continue;
                }
                let action = self.strip_tag(scale, start, end);
                if action.tag_ranges() > 0 {
                    actions.push(action);
                }
            }
        }

        if tag_name != "normal" {
            actions.push(Action::tag(self, tag_name, start, end, true));
            self.apply_tag_by_name(tag_name, start, end);
        }
        self.trigger_changed();
        composite_of(actions)
    }

    pub fn strip_tag(&self, tag_name: &str, start: &gtk::TextIter, end: &gtk::TextIter) -> Action {
        let action = Action::tag(self, tag_name, start, end, false);
        self.remove_tag_by_name(tag_name, start, end);
        self.trigger_changed();
        action
    }

    pub fn add_check_button(&self, iter: &mut gtk::TextIter, checked: bool) -> Option<Action> {
        let _guard = self.internal_action(true);
        let anchor = self.create_child_anchor(iter)?;
        let check = check_box::new(checked);
        let weak = self.downgrade();
        check.connect_toggled(move |_| {
            if let Some(b) = weak.upgrade() {
                b.trigger_changed();
            }
        });
        if let Some(view) = self.view() {
            view.add_child_at_anchor(&check, &anchor);
        }
        Action::object(self, &anchor, true)
    }

    pub fn add_bullet(&self, iter: &mut gtk::TextIter) -> Option<Action> {
        let _guard = self.internal_action(true);
        let anchor = self.create_child_anchor(iter)?;
        let bullet = gtk::Image::from_icon_name(Some("menu-bullet"), gtk::IconSize::Menu);
        bullet.set_pixel_size(16);
        bullet.set_visible(true);
        if let Some(view) = self.view() {
            view.add_child_at_anchor(&bullet, &anchor);
        }
        Action::object(self, &anchor, true)
    }

    fn selection_or_cursor(&self) -> (gtk::TextIter, gtk::TextIter) {
        self.selection_bounds().unwrap_or_else(|| {
            let cursor = self.iter_at_mark(&self.get_insert().unwrap());
            (cursor, cursor)
        })
    }

    fn toggle_line_objects(&self, kind: AnchorType) {
        let mut actions = Vec::new();
        {
            let _guard = self.internal_action(true);
            let (start, end) = self.selection_or_cursor();
            let (first, last) = (start.line(), end.line());

            let all_have_objects = (first..=last).all(|line| self.iter_at_line(line).child_anchor().is_some());

            for line in first..=last {
                if all_have_objects {
                    if let Some(anchor) = self.iter_at_line(line).child_anchor() {
                        if let Some(mut action) = Action::object(self, &anchor, false) {
                            action.object_remove(self);
                            actions.push(action);
                        }
                    }
                } else {
                    let mut iter = self.iter_at_line(line);
                    let action = match kind {
                        AnchorType::Check => self.add_check_button(&mut iter, false),
                        AnchorType::Bullet => self.add_bullet(&mut iter),
                    };
                    actions.extend(action);
                }
            }
        }

        if !actions.is_empty() {
            self.add_undo_action(Action::Composite(actions));
        }
    }

    pub fn toggle_checklist(&self) {
        self.toggle_line_objects(AnchorType::Check);
    }

    pub fn toggle_bullets(&self) {
        self.toggle_line_objects(AnchorType::Bullet);
    }

    /// Return on an empty checklist/bullet line removes the checkbox/bullet.
    pub fn on_return(&self) -> glib::Propagation {
        if self.has_selection() {
            return glib::Propagation::Proceed;
        }

        let mut cursor = self.iter_at_mark(&self.get_insert().unwrap());
        let mut prev_char = cursor;
        prev_char.backward_char();

        if !cursor.ends_line() || !prev_char.starts_line() {
            return glib::Propagation::Proceed;
        }

        if prev_char.child_anchor().is_some() {
            self.delete(&mut prev_char, &mut cursor);
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    }

    pub fn shift(&self, is_up: bool) {
        let (start, end) = self.selection_or_cursor();
        let mut action = Action::Shift { start: start.line(), end: end.line(), is_up };

        {
            let _guard = self.internal_action(true);
            if is_up {
                if start == self.start_iter() {
                    return;
                }
                action.shift_up(self);
            } else {
                if end == self.end_iter() {
                    return;
                }
                action.shift_down(self);
            }
        }

        self.add_undo_action(action);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn flush_idle() {
        while glib::MainContext::default().iteration(false) {}
    }

    fn type_text(buffer: &NoteBuffer, text: &str) {
        for c in text.chars() {
            let mut cursor = buffer.iter_at_mark(&buffer.get_insert().unwrap());
            buffer.insert_interactive(&mut cursor, &c.to_string(), true);
        }
    }

    /// Run from `gtk_tests` in main.rs: GTK can only be used from one thread.
    pub fn buffer_behaviour() {
        // markup round trip
        let markup = "#check:1Buy ##milk#tag:bold: now#tag:bold:\n#bullet:item";
        let buffer = NoteBuffer::new();
        let view = gtk::TextView::with_buffer(buffer.text_buffer());
        buffer.set_view(&view);
        buffer.set_from_internal_markup(markup);
        assert_eq!(buffer.get_internal_markup(), markup);
        assert!(!buffer.can_undo(), "loading must not be undoable");

        // typing, undo and redo
        let buffer = NoteBuffer::new();
        let view = gtk::TextView::with_buffer(buffer.text_buffer());
        buffer.set_view(&view);
        let changed = Rc::new(Cell::new(0));
        let c = changed.clone();
        buffer.connect_content_changed(move || c.set(c.get() + 1));
        type_text(&buffer, "hello");
        assert_eq!(buffer.get_internal_markup(), "hello");
        flush_idle();
        assert!(changed.get() > 0);
        buffer.undo();
        assert_eq!(buffer.get_internal_markup(), "", "typed characters are joined into one undo step");
        buffer.redo();
        assert_eq!(buffer.get_internal_markup(), "hello");

        // formatting the selection
        let (s, e) = (buffer.iter_at_offset(0), buffer.iter_at_offset(5));
        buffer.select_range(&s, &e);
        buffer.tag_selection("bold");
        assert_eq!(buffer.get_internal_markup(), "#tag:bold:hello#tag:bold:");
        buffer.undo();
        assert_eq!(buffer.get_internal_markup(), "hello");

        // checklist toggling continues on new lines and is undoable
        let end = buffer.end_iter();
        buffer.select_range(&end, &end);
        buffer.toggle_checklist();
        assert_eq!(buffer.get_internal_markup(), "#check:0hello");
        type_text(&buffer, "\n");
        assert_eq!(buffer.get_internal_markup(), "#check:0hello\n#check:0");
        buffer.undo();
        assert_eq!(buffer.get_internal_markup(), "#check:0hello");
        buffer.undo();
        assert_eq!(buffer.get_internal_markup(), "hello");

        // links are detected when a link-ending character is typed
        let buffer = NoteBuffer::new();
        let view = gtk::TextView::with_buffer(buffer.text_buffer());
        buffer.set_view(&view);
        type_text(&buffer, "see https://linuxmint.com ");
        assert_eq!(buffer.get_internal_markup(), "see #tag:link:https://linuxmint.com#tag:link: ");

        // deleting text with a tag can be undone
        let (mut s, mut e) = (buffer.iter_at_offset(0), buffer.end_iter());
        buffer.delete_interactive(&mut s, &mut e, true);
        assert_eq!(buffer.get_internal_markup(), "");
        buffer.undo();
        assert_eq!(buffer.get_internal_markup(), "see #tag:link:https://linuxmint.com#tag:link: ");

        // moving lines
        let buffer = NoteBuffer::new();
        let view = gtk::TextView::with_buffer(buffer.text_buffer());
        buffer.set_view(&view);
        buffer.set_from_internal_markup("one\ntwo");
        let line2 = buffer.iter_at_line(1);
        buffer.select_range(&line2, &line2);
        buffer.shift(true);
        assert_eq!(buffer.get_internal_markup(), "two\none");
        buffer.undo();
        assert_eq!(buffer.get_internal_markup(), "one\ntwo");
    }
}
