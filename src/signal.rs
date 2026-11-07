//! A small single-threaded signal type, used in place of the custom GObject
//! signals the Python version declared on its plain Python objects.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandlerId(u64);

type Handler<T> = Rc<dyn Fn(&T)>;

pub struct Signal<T> {
    next_id: Cell<u64>,
    handlers: RefCell<Vec<(HandlerId, Handler<T>)>>,
    blocked: RefCell<Vec<HandlerId>>,
}

impl<T> Default for Signal<T> {
    fn default() -> Self {
        Signal { next_id: Cell::new(1), handlers: RefCell::default(), blocked: RefCell::default() }
    }
}

impl<T> Signal<T> {
    pub fn connect<F: Fn(&T) + 'static>(&self, f: F) -> HandlerId {
        let id = HandlerId(self.next_id.get());
        self.next_id.set(id.0 + 1);
        self.handlers.borrow_mut().push((id, Rc::new(f)));
        id
    }

    #[allow(dead_code)]
    pub fn disconnect(&self, id: HandlerId) {
        self.handlers.borrow_mut().retain(|(h, _)| *h != id);
    }

    pub fn block(&self, id: HandlerId) {
        self.blocked.borrow_mut().push(id);
    }

    pub fn unblock(&self, id: HandlerId) {
        let mut blocked = self.blocked.borrow_mut();
        if let Some(pos) = blocked.iter().position(|h| *h == id) {
            blocked.remove(pos);
        }
    }

    pub fn emit(&self, value: &T) {
        // Handlers may connect or disconnect while we run, so work on a copy.
        let handlers: Vec<_> = self.handlers.borrow().clone();
        for (id, handler) in handlers {
            if self.blocked.borrow().contains(&id) {
                continue;
            }
            if !self.handlers.borrow().iter().any(|(h, _)| *h == id) {
                continue;
            }
            handler(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_and_disconnect() {
        let sig: Signal<i32> = Signal::default();
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let id = sig.connect(move |v| h.set(h.get() + v));
        sig.emit(&2);
        sig.block(id);
        sig.emit(&5);
        sig.unblock(id);
        sig.emit(&1);
        sig.disconnect(id);
        sig.emit(&100);
        assert_eq!(hits.get(), 3);
    }
}
