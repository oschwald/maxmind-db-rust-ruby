//! Running pure Rust work without holding Ruby's Global VM Lock (GVL).

use magnus::error::Error;
use std::{
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    ptr,
};

struct Call<F, R> {
    func: Option<F>,
    result: Option<std::thread::Result<R>>,
}

/// Runs `func` with the GVL released so other Ruby threads can run.
///
/// `func` must not touch any Ruby object or call any Ruby API, including
/// creating exceptions. Return plain Rust data and convert it to Ruby values
/// after this function returns.
///
/// No unblocking function is registered, so `func` always runs to the end.
/// `Thread#raise`, `Timeout` and signals such as Ctrl-C take effect once it
/// returns, and are returned here as an `Err` so they can propagate normally.
pub(crate) fn without_gvl<F, R>(func: F) -> Result<R, Error>
where
    F: FnOnce() -> R,
{
    let mut call = Call {
        func: Some(func),
        result: None,
    };
    let data: *mut c_void = (&mut call as *mut Call<F, R>).cast();

    // `rb_thread_call_without_gvl` checks for pending interrupts after the
    // callback returns and may raise. `protect` catches that raise so it does
    // not unwind over Rust frames; `call` lives outside the closure and is
    // dropped normally.
    let protected = magnus::rb_sys::protect(|| {
        // SAFETY: the trampoline matches the callback signature, `call`
        // outlives the call, and the caller guarantees `func` does not use
        // the Ruby API.
        unsafe {
            rb_sys::rb_thread_call_without_gvl(
                Some(call_trampoline::<F, R>),
                data,
                None,
                ptr::null_mut(),
            );
        }
        rb_sys::Qnil as rb_sys::VALUE
    });

    match call.result.take() {
        Some(Ok(value)) => protected.map(|_| value),
        Some(Err(payload)) => panic::resume_unwind(payload),
        None => {
            protected?;
            unreachable!("rb_thread_call_without_gvl returned without running the callback")
        }
    }
}

unsafe extern "C" fn call_trampoline<F, R>(data: *mut c_void) -> *mut c_void
where
    F: FnOnce() -> R,
{
    // SAFETY: `data` is the `Call<F, R>` that `without_gvl` passed in, and it
    // stays borrowed until `rb_thread_call_without_gvl` returns.
    let call = unsafe { &mut *data.cast::<Call<F, R>>() };
    if let Some(func) = call.func.take() {
        // A panic must not unwind across the C frame, so catch it here and
        // resume it once the GVL is held again.
        call.result = Some(panic::catch_unwind(AssertUnwindSafe(func)));
    }
    ptr::null_mut()
}
