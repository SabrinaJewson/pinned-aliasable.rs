# pinned-aliasable

`Pin`-based stopgap for unboxed aliasable values in self-referential data structures.

## Uniqueness

For the sake of optimization, the Rust compiler likes to assume that all mutable references
(`&mut`) are completely unique. This uniqueness gives it some extremely important guarantees
that can be easily exploited for faster code, such as:
- All reads from an `&mut` location are guaranteed to be the same if the reference is not
written to in between.
- Writes to the location are guaranteed to stay there unless explicitly overwritten with the
same mutable reference.
- No one is able to see the data stored behind the mutable reference while it exists without
using that mutable reference.

A simple example of where `&mut` uniqueness is useful is in this code:

```rust
fn foo(x: &mut i32) -> i32 {
    *x = 400;
    do_some_other_stuff();
    *x
}
```

The compiler will optimize this function to always return the constant 400, instead of having
to actually load the value stored behind `x` every time. It was only able to do this because `x`
is a unique pointer; if it wasn't, it could be possible that `do_some_other_stuff` would mutate
it in between and always returning the constant would result in unexpected behaviour.

## Self-referential types

However, this assumption starts to run into problems when using self-referential types. What
if, instead of being a simple integer, `x` was a type that held a reference to itself? Although
it isn't immediately obvious, the uniqueness guarantee is actually violated here: the
self-reference held in `x` aliases with the `&mut` to `x`, meaning the mutable reference _is no
longer unique_! And this issue isn't just theoretical, it causes miscompilations in the wild.
For example this code, which was based off [an actual soundness issue in
the `owning-ref` crate](https://github.com/Kimundi/owning-ref-rs/issues/49):

```rust
use std::cell::Cell;

struct Helper {
    reference: &'static Cell<u8>,
    owner: Box<Cell<u8>>,
}
fn helper(x: Helper) -> u8 {
    x.owner.set(10);
    x.reference.set(20);
    x.owner.get()
}

let owner = Box::new(Cell::new(0));
let reference = unsafe { &*(&*owner as *const Cell<u8>) };
let x = Helper { reference, owner };
println!("{}", helper(x));
```

When run in release mode, this program prints out `10` instead of the expected value of `20`.
This is because inside `helper`, the optimizer sees that we have unique access to the
`Cell<u8>` (`Box`es, like `&mut`s, are seen as unique pointers), and so it assumes that any
writes to that location will never be overwritten. But because we violated the optimizer's
expectations, we ended up with a nonsensical result.

So what's the solution to this? Well, as it stands, there isn't one - at least not one that's
both sound and doesn't sacrifice performance. It is possible to use a different kind of smart
pointer than `Box`, one that doesn't allow the compiler to assume its pointer is unique, and
that _would_ work for the above case with almost no performance impact - but in cases where the
self-referenced value is not boxed in the first place it's a much tougher choice to make.

It is very likely Rust eventually will have a solution to this, it's a well known bug that
needs to be fixed. In terms of what this solution will look like, it will most likely take the
shape of a `Aliasable<T>` wrapper type that exists in libcore and gives the guarantee that any
`&mut` references to the value will _not_ be considered to be unique, so that one
`&mut Aliasable<T>` and either one `&mut T` or any number of `&T`s can coexist (but not two
`&mut T`s or two `&mut Aliasable<T>`s; the regular borrowing rules still apply). Unfortunately,
this type doesn't exist today and there aren't even any concrete proposals for it yet. So what
can we do in the meantime?

## A Solution

Although it isn't possible to create sound self-referential types, as it turns out it _is_
possible to create unsound self-referential types _that we know won't miscompile_. This is
because to ensure that async blocks (which generate self-referential types) do not miscompile
in today's Rust, a temporary loophole was added to the `&mut` uniqueness rule: it only applies
when the referenced type doesn't implement `Unpin`. Thus to create these self-referential types
we simply have to make sure that they are `!Unpin`, and everything will work as expected.

However, doing this manually and upholding all the invariants that come with it is a pain, not
to mention the migration effort that will be required in future once Rust does support true
self-referential types. So that's where this crate comes in. It provides a type `Aliasable<T>`
which both abstracts the work of making the container type `!Unpin` and should be forward
compatible with the hypothetical libcore equivalent. As soon as `Aliasable<T>` _does_ get
added to the language itself, I will be able to publish a new version of this crate internally
based on it and yank all previous versions, which would then be unsound and obsolete.

There is one final snag: Miri. Miri is a tool for running your Rust code and dynamically
checking whether it is sound or not. If we used the main implementation of this crate under
Miri, it would report all kinds of errors, because what we are doing is fundamentally unsound
after all. So instead, when this crate detects that Miri is enabled it switches to a different
backend that boxes the value in a way that is totally sound, but isn't used normally for
efficiency reasons.

And that's it! Although this crate is tiny, it is really useful for defining any kind of
self-referential type because you no longer have to worry so much about whether you can cause
miscompilations, _and_ you can run your tests under Miri.

However, there is one important detail to be aware of. Remember how above I said that `Box`es
are also treated as always-unique pointers? This is true, and unfortunately they don't get the
same loophole that `&mut` does. This means you have to be very careful when working with boxed
`Aliasable<T>`s - make sure that any functions that take them by value always delegate to a
second function that takes them by unique or shared reference, so Rust doesn't assume your
pointer to it is unique.

## Examples

A boxed slice that also stores a subslice of itself:

```rust
use core::pin::Pin;
use core::ptr::NonNull;
use core::slice::SliceIndex;
use core::cell::UnsafeCell;

use pin_project::pin_project;
use pin_utils::pin_mut;
use pinned_aliasable::Aliasable;

#[pin_project]
pub struct OwningSlice<T: 'static> {
    // In a real implementation you would avoid the `T: 'static` bound by using some kind of
    // raw pointer here.
    slice: Option<&'static mut [T]>,
    #[pin]
    data: Aliasable<UnsafeCell<Box<[T]>>>,
}
impl<T: 'static> From<Box<[T]>> for OwningSlice<T> {
    fn from(data: Box<[T]>) -> Self {
        Self {
            slice: None,
            data: Aliasable::new(UnsafeCell::new(data)),
        }
    }
}
impl<T> OwningSlice<T> {
    pub fn slice(self: Pin<&mut Self>, range: impl SliceIndex<[T], Output = [T]>) {
        let mut this = self.project();
        let current_slice = this.slice.take().unwrap_or_else(|| {
            unsafe { &mut **this.data.as_ref().get_extended().get() }
        });
        *this.slice = Some(&mut current_slice[range]);
    }
    pub fn get(self: Pin<&Self>) -> &[T] {
        let this = self.project_ref();
        this.slice.as_deref().unwrap_or_else(|| unsafe { &**this.data.get().get() })
    }
    pub fn get_mut(self: Pin<&mut Self>) -> &mut [T] {
        let this = self.project();
        let data = this.data.as_ref();
        this.slice.as_deref_mut().unwrap_or_else(|| unsafe { &mut **data.get().get() })
    }
}

let slice = OwningSlice::from(vec![1, 2, 3, 4, 5].into_boxed_slice());
pin_mut!(slice);
assert_eq!(slice.as_ref().get(), &[1, 2, 3, 4, 5]);

slice.as_mut().slice(1..);
assert_eq!(slice.as_ref().get(), &[2, 3, 4, 5]);

slice.as_mut().slice(2..=3);
assert_eq!(slice.as_ref().get(), &[4, 5]);

slice.as_mut().slice(0..0);
assert_eq!(slice.as_ref().get(), &[]);
```

A pair type:

```rust
use core::pin::Pin;
use core::cell::Cell;

use pin_project::{pin_project, pinned_drop};
use pin_utils::pin_mut;
use pinned_aliasable::Aliasable;

#[pin_project(PinnedDrop)]
pub struct Pair(#[pin] Aliasable<PairInner>);

struct PairInner {
    value: u64,
    other: Cell<Option<&'static PairInner>>,
}

#[pinned_drop]
impl PinnedDrop for Pair {
    fn drop(self: Pin<&mut Self>) {
        if let Some(other) = self.project().0.as_ref().get().other.get() {
            other.other.set(None);
        }
    }
}

impl Pair {
    pub fn new(value: u64) -> Self {
        Self(Aliasable::new(PairInner {
            value,
            other: Cell::new(None),
        }))
    }
    pub fn get(self: Pin<&Self>) -> u64 {
        self.project_ref().0.get().other.get().unwrap().value
    }
}

pub fn link_up(left: Pin<&Pair>, right: Pin<&Pair>) {
    let left = unsafe { left.project_ref().0.get_extended() };
    let right = unsafe { right.project_ref().0.get_extended() };
    left.other.set(Some(right));
    right.other.set(Some(left));
}

fn main() {
    let pair_1 = Pair::new(10);
    let pair_2 = Pair::new(20);
    pin_mut!(pair_1);
    pin_mut!(pair_2);

    link_up(pair_1.as_ref(), pair_2.as_ref());

    assert_eq!(pair_1.as_ref().get(), 20);
    assert_eq!(pair_2.as_ref().get(), 10);
}
```

License: MIT
