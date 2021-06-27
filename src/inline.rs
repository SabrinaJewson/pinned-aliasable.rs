use core::marker::PhantomPinned;
use core::pin::Pin;

#[derive(Default)]
pub struct Aliasable<T> {
    data: T,
    _pinned: PhantomPinned,
}

impl<T> Aliasable<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            _pinned: PhantomPinned,
        }
    }
    pub fn get(self: Pin<&Self>) -> &T {
        &self.get_ref().data
    }
}
