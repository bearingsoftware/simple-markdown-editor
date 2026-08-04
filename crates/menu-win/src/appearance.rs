//! Whether Windows is set to light or dark, read from the registry.
//!
//! # Why the registry rather than the API
//!
//! `AGENTS.md` and `docs/research/05-windows-platform.md` both say to read
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme` rather than
//! calling `ShouldAppsUseDarkMode`, which has been unreliable since 1903 — it is exported by ordinal
//! only, its behaviour changed across builds, and Notepad++ carries the same note beside the same
//! decision. The value is a `DWORD`: **0 is dark, anything else is light**.
//!
//! # Why it lives in this crate
//!
//! It is a Win32 call, and `unsafe` outside a platform crate is a review block. There are three
//! platform crates and this is the menus' one, which is where the rest of the appearance machinery
//! already is — `dark.rs` forces the popups dark from here. A fourth `unsafe` crate would need its
//! own argument on its own ticket; a registry read for the menu bar's own theme does not need one.
//!
//! # What this does not do
//!
//! It does not watch for changes. `WM_SETTINGCHANGE` reaches the app through winit, which turns it
//! into `WindowEvent::ThemeChanged` — see the app's event loop, and the note there about winit
//! deciding the *value* with the API this module deliberately avoids. The event is the notification;
//! this is the answer.

// The platform layer for menus. The one block below carries its own `SAFETY` note.
#![allow(unsafe_code)]

use std::ffi::c_void;

use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

/// Whether Windows is drawing apps light, or `None` if it does not say.
///
/// `None` rather than a default, so the caller decides what an absent value means. It is absent on
/// a Windows old enough not to have the setting, and on one where the user has never opened the
/// personalisation page — and "dark" is the editor's answer in both cases, which is a decision about
/// this editor rather than about Windows.
#[must_use]
pub fn apps_use_light_theme() -> Option<bool> {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value: u32 = 0;
    let mut size = u32::try_from(size_of::<u32>()).ok()?;

    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call. `RRF_RT_REG_DWORD`
    // makes the API refuse anything that is not a `DWORD`, so the four bytes it writes through the
    // pointer are exactly the `u32` below — a string value under this name cannot be read into it.
    // `size` is that `u32`'s length going in, and is overwritten with what was written coming out.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast::<c_void>(),
            &raw mut size,
        )
    };

    // Any failure at all — no key, no value, the wrong type, no permission — is "Windows did not
    // say". There is nothing to report: a missing personalisation setting is not an error condition.
    (status == 0).then_some(value != 0)
}

/// A NUL-terminated UTF-16 buffer, as every wide Win32 string argument wants.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
