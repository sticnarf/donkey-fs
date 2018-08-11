use std::cell::RefCell;
use std::rc::Rc;
use *;

#[derive(Debug, Clone)]
pub struct Handle {
    inner: Rc<RefCell<Donkey>>,
}

impl Handle {
    pub(crate) fn new(dk: Donkey) -> Self {
        Handle {
            inner: Rc::new(RefCell::new(dk)),
        }
    }
}
