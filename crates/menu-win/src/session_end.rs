//! Hearing that Windows is shutting down or logging off (MDEDIT-120).
//!
//! winit handles neither `WM_QUERYENDSESSION` nor `WM_ENDSESSION`, so a subclass on the window does.
//! It cannot do the work itself: `Editor` is owned by `run_app` and the window procedure has no way
//! to reach it. It tells the caller through `notify`, which posts a wake into the event loop, and on
//! the message that means the process is about to be killed it asks again, since the query can come
//! long before it and typing since would be missed, then pumps messages for a bounded time so
//! that wake is dispatched before it returns.
//!
//! It never blocks the shutdown: `WM_QUERYENDSESSION` always answers `TRUE`, and the pump gives up
//! well inside the five seconds Windows allows.

#![allow(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
    TranslateMessage, WM_ENDSESSION, WM_NCDESTROY, WM_QUERYENDSESSION,
};

use crate::SessionEnd;

/// muda subclasses this same window under its own id; this one is distinct.
const SUBCLASS_ID: usize = 0x4d44_5345;

/// Under the roughly five seconds Windows gives a process after `WM_ENDSESSION`.
const PUMP_LIMIT: Duration = Duration::from_secs(3);

struct State {
    notify: Box<dyn Fn(SessionEnd)>,
    done: Arc<AtomicBool>,
}

pub fn watch(hwnd: isize, notify: impl Fn(SessionEnd) + 'static, done: Arc<AtomicBool>) {
    let state = Box::into_raw(Box::new(State {
        notify: Box::new(notify),
        done,
    }));
    // SAFETY: `hwnd` is the live window handle the caller got from the windowing layer, on this
    // thread, which `SetWindowSubclass` requires. `subclass_proc` matches `SUBCLASSPROC`. `state` is
    // a leaked box, valid until `subclass_proc` frees it on `WM_NCDESTROY`; if the call fails nothing
    // will ever free it, so it is reclaimed here.
    let installed = unsafe {
        SetWindowSubclass(
            hwnd as HWND,
            Some(subclass_proc),
            SUBCLASS_ID,
            state as usize,
        )
    };
    if installed == 0 {
        // SAFETY: the subclass was not installed, so nothing else holds `state`.
        drop(unsafe { Box::from_raw(state) });
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    match msg {
        WM_QUERYENDSESSION => {
            // SAFETY: `data` is the `State` pointer `watch` installed; it lives until `WM_NCDESTROY`.
            let state = unsafe { &*(data as *const State) };
            (state.notify)(SessionEnd::Ending);
            return 1;
        }
        WM_ENDSESSION => {
            // SAFETY: as above.
            let state = unsafe { &*(data as *const State) };
            if wparam != 0 {
                // The query can come long before this, while Windows asks other apps; anything typed
                // since was not in that flush. Ask again, at the last moment.
                state.done.store(false, Ordering::Release);
                (state.notify)(SessionEnd::Ending);
                pump_until(&state.done);
            } else {
                (state.notify)(SessionEnd::Cancelled);
            }
            return 0;
        }
        WM_NCDESTROY => {
            // SAFETY: the window is going away, so this is the last message it will see. The
            // subclass is removed first so nothing can reach `data` afterwards, and `data` is the box
            // `watch` leaked.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
                drop(Box::from_raw(data as *mut State));
            }
        }
        _ => {}
    }
    // SAFETY: the arguments are the ones this procedure was called with.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Dispatches this thread's messages, sleeping between them, until `done` is set or the limit passes.
fn pump_until(done: &AtomicBool) {
    let deadline = Instant::now() + PUMP_LIMIT;
    // SAFETY: `MSG` is plain data that Win32 fills in; all-zero is a valid initial value.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while !done.load(Ordering::Acquire) && Instant::now() < deadline {
        // SAFETY: `msg` is a valid out-pointer; a null window handle means every window on this
        // thread, which is what winit's own loop asks for. Translate and dispatch take that message.
        let got = unsafe { PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) };
        if got == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let millis = u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX);
            // SAFETY: no handles are passed (count 0, null array), so this waits only for a message
            // to arrive on this thread or for the timeout.
            unsafe { MsgWaitForMultipleObjects(0, std::ptr::null(), 0, millis, QS_ALLINPUT) };
            continue;
        }
        // SAFETY: `msg` was just filled by `PeekMessageW`.
        unsafe {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}
