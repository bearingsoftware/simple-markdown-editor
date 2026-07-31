//! Making the dropdown menus dark.
//!
//! # Why this is not just a muda setting
//!
//! [`MenuTheme::Dark`](muda::MenuTheme) covers the menu *bar*, and only the bar: muda owner-draws it
//! by handling `WM_UAHDRAWMENU` and `WM_UAHDRAWMENUITEM`, the undocumented non-client messages
//! Windows sends for it. A dropdown is an ordinary `HMENU` popup that Windows draws itself, and
//! muda has no hand in it. So MDEDIT-32 made the bar dark and left the menus that open from it light
//! grey, which is what MDEDIT-35 is.
//!
//! # The mechanism, and why it is undocumented
//!
//! There is no public API for this. Two uxtheme exports do it, and both are exported **by ordinal
//! only** — they have no names:
//!
//! - `SetPreferredAppMode` (135), which appeared in 1903, puts the process into dark mode for the
//!   controls that honour it, menus among them.
//! - `FlushMenuThemes` (136), which re-themes menus afterwards. It exists for exactly this.
//!
//! This is what Notepad++, Windows Terminal and adzm's `win32-darkmode` reference implementation all
//! do. The only documented alternative is to owner-draw every popup ourselves, which is a great deal
//! of code to reproduce what Windows already knows how to draw.
//!
//! # The risk, and what is done about it
//!
//! An ordinal is a promise about a position in an export table, not about a function. If a future
//! Windows moves them, ordinal 135 resolves to *something* and calling it through this signature is
//! undefined behaviour rather than a failed call — which is a different and much worse class of
//! problem than a menu being the wrong colour.
//!
//! So the build number is checked first and the ordinals are only touched on a Windows known to have
//! them, and every step can fail into doing nothing at all. An editor with light menus still edits;
//! that is the whole reason this is allowed to be best-effort.
//!
//! # This forces dark, it does not follow Windows
//!
//! `AGENTS.md` says to read the user's dark-mode preference from the registry rather than calling
//! `ShouldAppsUseDarkMode`, which is unreliable from 1903. That caution is about *querying* the
//! setting, and nothing here queries it. The editor's palette is one hardcoded dark theme, so the
//! menus are forced to match it — the same reasoning that made the bar `MenuTheme::Dark`. When the
//! editor grows a light palette, the theme here and there become one variable together.

// This crate is the platform layer for menus, and there is no safe way to reach an export that has
// no name. Every block below carries its own `SAFETY` note; the module docs carry the reasoning that
// makes them true.
#![allow(unsafe_code)]

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{FARPROC, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

/// Ordinal of `SetPreferredAppMode` in `uxtheme.dll`.
const SET_PREFERRED_APP_MODE: u16 = 135;
/// Ordinal of `FlushMenuThemes` in `uxtheme.dll`.
const FLUSH_MENU_THEMES: u16 = 136;

/// `PreferredAppMode::ForceDark`. Dark whatever the user's Windows is set to, because the editor is.
const FORCE_DARK: i32 = 2;

/// Windows 10 1903, the build `SetPreferredAppMode` arrived in.
///
/// 1809 had an earlier export at the same ordinal with a *different* signature (`AllowDarkModeForApp`,
/// taking a `BOOL`), which is precisely the reason to check a build number rather than just try the
/// call: on 1809 this would compile, resolve, and pass an `i32` to something expecting a `BOOL`.
const FIRST_BUILD: u32 = 18362;

type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
type FlushMenuThemes = unsafe extern "system" fn();

/// Asks Windows to draw this process's menus dark.
///
/// Best-effort by design: does nothing on a Windows without these exports, and nothing if either
/// fails to resolve. Call once, before the menu is attached.
pub fn use_dark_menus() {
    if nt_build() < FIRST_BUILD {
        return;
    }

    // SAFETY: a null-terminated ASCII name for a system DLL. `LoadLibraryA` either maps it and
    // returns a handle or returns null, which is checked below. uxtheme is already loaded into any
    // process with a window, so this takes a reference to it rather than mapping it afresh.
    let uxtheme = unsafe { LoadLibraryA(c"uxtheme.dll".as_ptr().cast()) };
    if uxtheme.is_null() {
        return;
    }

    let Some(set_mode) = (unsafe { by_ordinal(uxtheme, SET_PREFERRED_APP_MODE) }) else {
        return;
    };
    // SAFETY: on a build at or past 1903, ordinal 135 of uxtheme is `SetPreferredAppMode`, whose
    // signature is `PreferredAppMode __stdcall(PreferredAppMode)` — matched by `SetPreferredAppMode`
    // above. `FORCE_DARK` is a valid member of that enum. The build check is what makes this claim
    // about the ordinal true; see the module docs for why it is not merely a version guard.
    let set_mode: SetPreferredAppMode = unsafe { std::mem::transmute(set_mode) };
    // SAFETY: the function resolved above, called with the argument it documents.
    unsafe { set_mode(FORCE_DARK) };

    let Some(flush) = (unsafe { by_ordinal(uxtheme, FLUSH_MENU_THEMES) }) else {
        return;
    };
    // SAFETY: as above — ordinal 136 is `FlushMenuThemes`, `void __stdcall(void)`, which is what
    // `FlushMenuThemes` declares. It takes no arguments, so there is nothing to get wrong but the
    // ordinal itself.
    let flush: FlushMenuThemes = unsafe { std::mem::transmute(flush) };
    // SAFETY: the function resolved above, which takes nothing.
    unsafe { flush() };
}

/// Looks an export up by ordinal rather than by name.
///
/// # Safety
///
/// `module` must be a live module handle.
unsafe fn by_ordinal(module: HMODULE, ordinal: u16) -> Option<*const c_void> {
    // An ordinal is passed to `GetProcAddress` as a pointer whose high bits are zero — the documented
    // `MAKEINTRESOURCEA` convention, and the only way to reach an export that has no name.
    let name = ordinal as usize as *const u8;
    let found: FARPROC = unsafe { GetProcAddress(module, name) };
    found.map(|address| address as *const c_void)
}

/// The Windows build number.
///
/// From `RtlGetNtVersionNumbers` rather than `GetVersionExW`, which lies: it reports 6.2 for any
/// process without a compatibility manifest naming a newer Windows, so it would say 9200 here and
/// this whole path would switch itself off.
fn nt_build() -> u32 {
    type RtlGetNtVersionNumbers = unsafe extern "system" fn(*mut u32, *mut u32, *mut u32);

    // SAFETY: a null-terminated ASCII name for a system DLL; null is checked.
    let ntdll = unsafe { LoadLibraryA(c"ntdll.dll".as_ptr().cast()) };
    if ntdll.is_null() {
        return 0;
    }
    // SAFETY: a null-terminated ASCII export name in a live module.
    let Some(address) =
        (unsafe { GetProcAddress(ntdll, c"RtlGetNtVersionNumbers".as_ptr().cast()) })
    else {
        return 0;
    };
    // SAFETY: `RtlGetNtVersionNumbers` has been exported by ntdll under this name since Windows XP
    // with the signature `void __stdcall(DWORD*, DWORD*, DWORD*)`, which is what is declared above.
    // Resolved by name rather than by ordinal, so there is no positional assumption to be wrong.
    let version: RtlGetNtVersionNumbers = unsafe { std::mem::transmute(address) };

    let (mut major, mut minor, mut build) = (0u32, 0u32, 0u32);
    // SAFETY: three live, writable `u32`s, which is what the function writes through.
    unsafe { version(&raw mut major, &raw mut minor, &raw mut build) };
    // The top bits are a checked-build flag rather than part of the number.
    build & 0x0fff_ffff
}

#[cfg(test)]
mod tests {
    use super::{FIRST_BUILD, nt_build, use_dark_menus};

    /// The gate decides whether any of this runs at all, and the way it fails is silent — a wrong
    /// build number switches the feature off and looks exactly like Windows declining to honour it.
    ///
    /// This is also what would catch the `GetVersionExW` trap the doc comment describes: that call
    /// reports build 9200 for a process without a compatibility manifest, which is below the floor,
    /// so the menus would quietly stay light on every supported Windows.
    #[test]
    fn the_build_number_is_a_real_one() {
        let build = nt_build();
        assert!(
            build >= FIRST_BUILD,
            "this project is Windows-10-1903-or-later, so {build} is either a lie or a machine \
             that cannot run the editor"
        );
    }

    /// Nothing here may panic or trap. It cannot be asserted that the menus *look* dark — that needs
    /// eyes on Windows — but it can be asserted that asking costs nothing, including when asked
    /// twice, since attaching a menu to a second window would.
    #[test]
    fn asking_twice_is_harmless() {
        use_dark_menus();
        use_dark_menus();
    }
}
