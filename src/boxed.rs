extern crate alloc;

use alloc::boxed::Box;
use core::pin::Pin;
use core::ptr::NonNull;

pub struct Aliasable<T>(NonNull<T>);

impl<T> Aliasable<T> {
    pub fn new(data: T) -> Self {
        Self(NonNull::from(Box::leak(Box::new(data))))
    }
    pub fn get(self: Pin<&Self>) -> &T {
        unsafe { self.get_ref().0.as_ref() }
    }
}

impl<T: Default> Default for Aliasable<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Drop for Aliasable<T> {
    fn drop(&mut self) {
        unsafe { Box::from_raw(self.0.as_ptr()) };
    }
}
