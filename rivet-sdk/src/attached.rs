//! **A value attached to a GPU buffer object for exactly that object's life**,
//! with no C in it.
//!
//! `crate::gpu::Screen::present` gives each GBM buffer object one `wl_buffer`,
//! made the first time the object is presented and attached again on every
//! later turn. The compositor imports a dma-buf when its `wl_buffer` is created,
//! so a `wl_buffer` per frame is an import per frame — and on a board whose
//! memory reaches past what the display controller can address, each import has
//! the kernel map the buffer through a bounce buffer that cannot hold a frame.
//!
//! **A GBM surface's buffer objects are not fixed, so the `wl_buffer` cannot
//! outlive its object.** Mesa allocates up to four for a surface as the
//! compositor holds them, and destroys the oldest unused one once a surface has
//! had more free than it needed for a thousand frames (`platform_drm.c`'s
//! `destroy_oldest_unused_bo`). A `wl_buffer` kept past its object's
//! destruction keeps the compositor holding that object's dma-buf — and the
//! compositor may not let go of a buffer its client has not destroyed — so
//! every destroy-and-reallocate strands one frame of GPU memory and the
//! compositor's copy of it until the client exits. And a new object may be
//! allocated at a destroyed one's address, so a value found by the object's
//! address is not evidence it belongs to the object there now.
//!
//! So the value lives in the object's own user data, and the object's
//! destruction drops it. GBM's `gbm_bo_set_user_data` is that slot and calls
//! back when the object goes; [`UserData`] is the slot without the C, so the
//! rule is tested here against a model of Mesa where the development host
//! cannot link the real one.
#![allow(unsafe_code)]

use std::ffi::c_void;

/// What a buffer object is told to call when it is destroyed, with its own
/// address and the data it was given.
pub(crate) type Destroy = unsafe extern "C" fn(bo: *mut c_void, data: *mut c_void);

/// **A buffer object's one slot for its owner's data**, and the call it makes
/// when the object is destroyed — `gbm_bo_set_user_data` and
/// `gbm_bo_get_user_data` in `crate::gpu`.
pub(crate) trait UserData {
    /// The data last set on `bo`, or null when none has been.
    ///
    /// # Safety
    ///
    /// `bo` is a live buffer object of this store.
    unsafe fn get(&self, bo: *mut c_void) -> *mut c_void;

    /// Set `data` on `bo`. `destroy` is called with it when `bo` is destroyed.
    ///
    /// # Safety
    ///
    /// `bo` is a live buffer object of this store.
    unsafe fn set(&self, bo: *mut c_void, data: *mut c_void, destroy: Destroy);
}

/// **The value attached to `bo`, made by `make` the first time the object is
/// seen**, and handed out through `view` on every turn after.
///
/// The value is owned by the object from then on: when the object is
/// destroyed, the value is dropped, and a value whose `Drop` destroys a
/// protocol object destroys it then.
///
/// # Errors
///
/// Whatever `make` returns. A failure attaches nothing, so the object's next
/// turn tries again rather than being handed a value that was never made.
///
/// # Safety
///
/// `bo` is a live buffer object of `store`, and **nothing but this function
/// sets user data on it** with any `T` but this one: the slot is read back as a
/// `T`.
pub(crate) unsafe fn get_or_attach<T, R, E>(
    store: &impl UserData,
    bo: *mut c_void,
    make: impl FnOnce() -> Result<T, E>,
    view: impl Fn(&T) -> R,
) -> Result<R, E> {
    // SAFETY: the caller's contract — `bo` is live.
    let held = unsafe { store.get(bo) }.cast::<T>();
    if !held.is_null() {
        // SAFETY: the slot holds a `Box<T>` this function leaked into it, and
        // it is still there because the object is live: its destruction is
        // what takes it back (`drop_attached`).
        return Ok(view(unsafe { &*held }));
    }
    let value = make()?;
    let seen = view(&value);
    let leaked = Box::into_raw(Box::new(value));
    // SAFETY: the caller's contract — `bo` is live. Ownership of `leaked`
    // passes to the object and comes back through `drop_attached::<T>`.
    unsafe { store.set(bo, leaked.cast(), drop_attached::<T>) };
    Ok(seen)
}

/// **The object is going: drop what was attached to it.**
///
/// Called by GBM from inside its own calls, on the thread that made them.
/// Nothing here may unwind — the release profile aborts on a panic, and an
/// unwind across this boundary would be undefined anyway — so it does nothing
/// that can.
///
/// # Safety
///
/// `data` is null or the `Box<T>` [`get_or_attach`] leaked, given back once.
unsafe extern "C" fn drop_attached<T>(_bo: *mut c_void, data: *mut c_void) {
    if data.is_null() {
        return;
    }
    // SAFETY: the function's contract — this is `get_or_attach`'s box, and
    // GBM calls this once, when the object it was attached to is destroyed.
    drop(unsafe { Box::from_raw(data.cast::<T>()) });
}

#[cfg(test)]
mod tests {
    //! **Against a model of Mesa's GBM surface**, because the development host
    //! cannot link the real one. The model keeps what the real one keeps that
    //! matters here: one user-data slot per object, the destroy call when an
    //! object goes, and a new object free to land at a destroyed one's address.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{get_or_attach, Destroy, UserData};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::rc::Rc;

    /// Buffer objects by address, each with its slot.
    #[derive(Default)]
    struct Gbm {
        slots: RefCell<HashMap<usize, (*mut c_void, Option<Destroy>)>>,
    }

    impl Gbm {
        /// A new object at `address`, which may be one a destroyed object had.
        fn allocate(&self, address: usize) -> *mut c_void {
            let fresh = self.slots.borrow_mut().insert(address, (std::ptr::null_mut(), None));
            assert!(fresh.is_none(), "the model allocated over a live object at {address:#x}");
            address as *mut c_void
        }

        /// Destroy `bo` the way `gbm_bo_destroy` does: the destroy call first.
        fn destroy(&self, bo: *mut c_void) {
            let (data, destroy) = self
                .slots
                .borrow_mut()
                .remove(&(bo as usize))
                .expect("the model destroyed an object it does not have");
            if let Some(destroy) = destroy {
                // SAFETY: the data and the function were set together by
                // `get_or_attach`, and the object is destroyed once.
                unsafe { destroy(bo, data) };
            }
        }
    }

    impl UserData for Gbm {
        unsafe fn get(&self, bo: *mut c_void) -> *mut c_void {
            self.slots.borrow().get(&(bo as usize)).expect("a live object").0
        }

        unsafe fn set(&self, bo: *mut c_void, data: *mut c_void, destroy: Destroy) {
            let mut slots = self.slots.borrow_mut();
            let slot = slots.get_mut(&(bo as usize)).expect("a live object");
            *slot = (data, Some(destroy));
        }
    }

    /// A `wl_buffer` stand-in: a number, and a count of how many were destroyed.
    struct Buffer {
        id: u32,
        destroyed: Rc<Cell<u32>>,
    }

    impl Drop for Buffer {
        fn drop(&mut self) {
            self.destroyed.set(self.destroyed.get() + 1);
        }
    }

    /// One `present`: the object's buffer, made with the next id if it has none.
    fn present(gbm: &Gbm, bo: *mut c_void, made: &Cell<u32>, destroyed: &Rc<Cell<u32>>) -> u32 {
        // SAFETY: `bo` is live in `gbm`, and only this function sets its slot.
        unsafe {
            get_or_attach(
                gbm,
                bo,
                || {
                    made.set(made.get() + 1);
                    Ok::<_, ()>(Buffer { id: made.get(), destroyed: Rc::clone(destroyed) })
                },
                |buffer| buffer.id,
            )
        }
        .unwrap()
    }

    /// A buffer object presented again is handed its first value and nothing is
    /// made — the property that stops the compositor importing a dma-buf once
    /// per frame.
    #[test]
    fn an_object_seen_again_is_handed_its_first_value_and_nothing_is_made() {
        let gbm = Gbm::default();
        let (made, destroyed) = (Cell::new(0), Rc::new(Cell::new(0)));
        let bo = gbm.allocate(0x1000);
        for _ in 0..30 {
            assert_eq!(present(&gbm, bo, &made, &destroyed), 1);
        }
        assert_eq!(made.get(), 1, "thirty frames of one object made {} buffers", made.get());
        assert_eq!(destroyed.get(), 0);
    }

    /// Three objects in rotation, the way a GBM surface cycles them: one value
    /// each, made on that object's first turn, and none after.
    #[test]
    fn a_rotation_of_objects_makes_one_value_per_object() {
        let gbm = Gbm::default();
        let (made, destroyed) = (Cell::new(0), Rc::new(Cell::new(0)));
        let bos = [gbm.allocate(0x1000), gbm.allocate(0x2000), gbm.allocate(0x3000)];
        for frame in 0..30 {
            present(&gbm, bos[frame % 3], &made, &destroyed);
        }
        assert_eq!(made.get(), 3);
        assert_eq!(destroyed.get(), 0);
    }

    /// **The leak this module exists to stop.** Mesa destroys an object it has
    /// stopped needing; the `wl_buffer` made for it must go with it, or the
    /// compositor holds that object's dma-buf and its own copy of the frame
    /// for as long as this client lives.
    #[test]
    fn an_object_mesa_destroys_takes_its_value_with_it() {
        let gbm = Gbm::default();
        let (made, destroyed) = (Cell::new(0), Rc::new(Cell::new(0)));
        let bo = gbm.allocate(0x1000);
        present(&gbm, bo, &made, &destroyed);
        gbm.destroy(bo);
        assert_eq!(destroyed.get(), 1, "Mesa destroyed the object and its buffer survived it");
    }

    /// **A new object at a destroyed object's address is a new object.** Mesa
    /// allocates the replacement with `malloc`, which may hand back the same
    /// address; a buffer found by address would then be the destroyed object's,
    /// and the compositor would be shown a frame nothing draws into any more.
    #[test]
    fn a_new_object_at_a_destroyed_objects_address_gets_a_value_of_its_own() {
        let gbm = Gbm::default();
        let (made, destroyed) = (Cell::new(0), Rc::new(Cell::new(0)));
        let first = gbm.allocate(0x1000);
        assert_eq!(present(&gbm, first, &made, &destroyed), 1);
        gbm.destroy(first);
        let second = gbm.allocate(0x1000);
        assert_eq!(second, first, "the model reused the address, which is the case under test");
        assert_eq!(
            present(&gbm, second, &made, &destroyed),
            2,
            "the object at a reused address was handed the destroyed object's buffer"
        );
    }

    /// **Mesa's trim and reallocate, a hundred times over**: the buffers alive
    /// never exceed the objects alive, however many cycles there have been.
    #[test]
    fn a_hundred_trims_and_reallocations_leave_one_buffer_per_live_object() {
        let gbm = Gbm::default();
        let (made, destroyed) = (Cell::new(0), Rc::new(Cell::new(0)));
        let mut bos = vec![gbm.allocate(0x1000), gbm.allocate(0x2000), gbm.allocate(0x3000)];
        for cycle in 0..100usize {
            for &bo in &bos {
                present(&gbm, bo, &made, &destroyed);
            }
            let oldest = bos.remove(0);
            gbm.destroy(oldest);
            bos.push(gbm.allocate(0x10_0000 + cycle * 0x1000));
        }
        for &bo in &bos {
            present(&gbm, bo, &made, &destroyed);
        }
        assert_eq!(
            made.get() - destroyed.get(),
            3,
            "{} buffers made and {} destroyed, for three live objects",
            made.get(),
            destroyed.get()
        );
    }

    /// A failed make attaches nothing: the object's next turn tries again, and
    /// the value that then succeeds is the one kept.
    #[test]
    fn a_failed_make_attaches_nothing() {
        let gbm = Gbm::default();
        let bo = gbm.allocate(0x1000);
        // SAFETY: `bo` is live in `gbm`, and only this test sets its slot.
        let failed = unsafe { get_or_attach(&gbm, bo, || Err::<u8, _>("export failed"), |v| *v) };
        assert_eq!(failed, Err("export failed"));
        // SAFETY: as above.
        let second = unsafe { get_or_attach(&gbm, bo, || Ok::<u8, &str>(7), |v| *v) };
        assert_eq!(second, Ok(7));
        // SAFETY: as above.
        let third = unsafe { get_or_attach(&gbm, bo, || Err("must not be called"), |v: &u8| *v) };
        assert_eq!(third, Ok(7));
        gbm.destroy(bo);
    }
}
