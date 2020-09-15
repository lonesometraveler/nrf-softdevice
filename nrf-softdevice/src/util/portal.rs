use core::cell::UnsafeCell;
use core::future::Future;
use core::mem;
use core::mem::MaybeUninit;

use crate::util::*;

/// Utility to call a closure across tasks.
pub struct Portal<T> {
    state: UnsafeCell<State<T>>,
}

enum State<T> {
    None,
    Waiting(*mut dyn FnMut(T)),
}

unsafe impl<T> Send for Portal<T> {}
unsafe impl<T> Sync for Portal<T> {}

fn assert_thread_mode() {
    deassert!(
        cortex_m::peripheral::SCB::vect_active()
            == cortex_m::peripheral::scb::VectActive::ThreadMode,
        "portals are not usable from interrupts"
    );
}

impl<T> Portal<T> {
    pub const fn new() -> Self {
        Self {
            state: UnsafeCell::new(State::None),
        }
    }

    pub fn call(&self, val: T) {
        assert_thread_mode();

        // safety: this runs from thread mode
        unsafe {
            let state = &mut *self.state.get();
            if let State::Waiting(func) = *state {
                *state = State::None;
                (*func)(val);
            }
        }
    }

    pub fn wait<'a, R, F>(&'a self, mut func: F) -> impl Future<Output = R> + 'a
    where
        F: FnMut(T) -> R + 'a,
    {
        assert_thread_mode();

        async move {
            let bomb = DropBomb::new();

            let signal = Signal::new();
            let mut result: MaybeUninit<R> = MaybeUninit::uninit();
            let mut call_func = |val: T| {
                unsafe { result.as_mut_ptr().write(func(val)) };
                signal.signal(());
            };

            let func_ptr: *mut dyn FnMut(T) = &mut call_func as _;
            let func_ptr: *mut dyn FnMut(T) = unsafe { mem::transmute(func_ptr) };

            // safety: this runs from thread mode
            unsafe {
                let state = &mut *self.state.get();
                match state {
                    State::None => {}
                    _ => depanic!("Multiple tasks waiting on same portal"),
                }
                *state = State::Waiting(func_ptr);
            }

            signal.wait().await;

            bomb.defuse();

            unsafe { result.assume_init() }
        }
    }
}
