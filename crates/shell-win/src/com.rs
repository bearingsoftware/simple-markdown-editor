//! The Win32 half: the registry writes, and the COM class Explorer activates.
//!
//! Everything `unsafe` in this crate is here. `AGENTS.md` permits it in a platform crate and blocks
//! it anywhere else; the crate docs carry the reason this crate is the third one.

#![allow(unsafe_code)]
// The #[implement] macro writes the COM vtables, and its generated code trips pedantic lints that
// are about hand-written style. Same three mdedit-text-dwrite allows, for the same reason.
#![allow(clippy::ref_as_ptr, clippy::inline_always, clippy::borrow_as_ptr)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;

use windows::Win32::Foundation::{E_NOINTERFACE, ERROR_FILE_NOT_FOUND, HGLOBAL, POINTL};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoInitializeEx, CoRegisterClassObject,
    CoRevokeClassObject, DVASPECT_CONTENT, FORMATETC, IClassFactory, IClassFactory_Impl,
    IDataObject, REGCLS_MULTIPLEUSE, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
    ReleaseStgMedium,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows_core::{BOOL, GUID, Interface, PCWSTR, Ref, implement};

use crate::registry::{CLASSES, Step};
use crate::{Deliver, Error};

/// The class Explorer activates, as a GUID rather than as text.
///
/// [`clsid_matches_the_registered_text`] holds it against [`crate::registry::CLSID`], because the
/// two are written in different alphabets and a mismatch would be a registration that succeeds and
/// an activation that finds nothing.
const CLSID_DROP_TARGET: GUID = GUID::from_u128(0x9F2B_4C6D_1E3A_4F58_9B7C_2D5E_8A0F_1C34);

/// A UTF-16, NUL-terminated copy of `text`, for the Win32 calls that want one.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The full key path for a step, under this user's classes.
fn full_key(step: &Step) -> Vec<u16> {
    wide(&format!("{CLASSES}\\{}", step.key()))
}

/// Writes every step. See [`crate::register`].
pub fn apply(steps: &[Step]) -> Result<(), Error> {
    for step in steps {
        let key = full_key(step);
        let mut handle = HKEY::default();
        // SAFETY: `key` is NUL-terminated and outlives the call; the out parameter is a live local.
        // Creating an existing key opens it, which is what makes registering twice idempotent.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &raw mut handle,
                None,
            )
        };
        if status.is_err() {
            return Err(Error::Registry(format!(
                "could not create {}: {status:?}",
                step.key()
            )));
        }

        let name = step.value().map(wide);
        let data = wide(step.data());
        // The byte length includes the terminator: `REG_SZ` values are stored with it, and a length
        // that omits it produces a value the shell reads as unterminated.
        let bytes: &[u8] = {
            // SAFETY: `data` is a live `Vec<u16>`; the slice borrows it for this expression and is
            // used before `data` is dropped. Alignment is weaker, not stronger, going u16 -> u8.
            unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2) }
        };
        // SAFETY: `handle` is open, `name` is NUL-terminated or absent for the default value, and
        // `bytes` is exactly the buffer described.
        let status = unsafe {
            RegSetValueExW(
                handle,
                name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
                None,
                REG_SZ,
                Some(bytes),
            )
        };
        // SAFETY: `handle` came from `RegCreateKeyExW` above and is closed exactly once.
        let _ = unsafe { RegCloseKey(handle) };
        if status.is_err() {
            return Err(Error::Registry(format!(
                "could not write {}: {status:?}",
                step.key()
            )));
        }
    }
    Ok(())
}

/// Removes every step. See [`crate::unregister`].
///
/// Owned roots go once each — deleting a subtree twice would report a missing key on the second
/// pass, and "already gone" is the state this function exists to reach.
pub fn remove(steps: &[Step]) -> Result<(), Error> {
    let mut deleted: Vec<&str> = Vec::new();
    for step in steps {
        match step {
            Step::Owned { root, .. } => {
                if deleted.contains(&root.as_str()) {
                    continue;
                }
                deleted.push(root);
                let key = wide(&format!("{CLASSES}\\{root}"));
                // SAFETY: `key` is NUL-terminated and outlives the call. `RegDeleteTreeW` removes
                // the key and everything under it, which is what "owned" means.
                let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr())) };
                if status.is_err() && status != ERROR_FILE_NOT_FOUND {
                    return Err(Error::Registry(format!(
                        "could not remove {root}: {status:?}"
                    )));
                }
            }
            Step::Shared { key, value, .. } => {
                let path = wide(&format!("{CLASSES}\\{key}"));
                let name = value.as_deref().map(wide);
                // SAFETY: both buffers are NUL-terminated and outlive the call. This removes one
                // value; the key it sits in belongs to the system and is left alone.
                let status = unsafe {
                    RegDeleteKeyValueW(
                        HKEY_CURRENT_USER,
                        PCWSTR(path.as_ptr()),
                        name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
                    )
                };
                if status.is_err() && status != ERROR_FILE_NOT_FOUND {
                    return Err(Error::Registry(format!(
                        "could not remove {key}: {status:?}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// A published class object. Revokes on drop.
#[derive(Debug)]
pub struct Server {
    cookie: u32,
}

impl Drop for Server {
    fn drop(&mut self) {
        // SAFETY: `cookie` came from `CoRegisterClassObject` and is revoked exactly once, because
        // `Server` is not `Clone` and this is its only drop.
        let _ = unsafe { CoRevokeClassObject(self.cookie) };
    }
}

/// Publishes the class object. See [`crate::serve`].
pub fn serve(deliver: Deliver) -> Result<Server, Error> {
    // Apartment-threaded, because that is what a UI thread is and what `IDropTarget` requires. An
    // already-initialised apartment answers `S_FALSE`, and one initialised the *other* way answers
    // `RPC_E_CHANGED_MODE` — neither is a failure of ours, and winit initialises OLE itself when the
    // loop starts, so both are reachable depending on call order.
    //
    // SAFETY: no arguments are borrowed, and the matching uninitialise is deliberately absent —
    // the apartment lives as long as the process, and tearing it down while winit still owns
    // windows on this thread would be the bug.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let factory: IClassFactory = Factory {
        deliver: Arc::new(deliver),
    }
    .into();

    // `REGCLS_MULTIPLEUSE` is the whole reuse story: the class stays published after the first
    // activation, so the second, third and twentieth Explorer request bind to this process rather
    // than starting another. `REGCLS_SINGLEUSE` would revoke after one and reintroduce exactly the
    // per-file process the ticket exists to remove.
    //
    // SAFETY: `factory` outlives the call — COM takes its own reference — and the GUID is a
    // 'static constant.
    let cookie = unsafe {
        CoRegisterClassObject(
            &CLSID_DROP_TARGET,
            &factory,
            CLSCTX_LOCAL_SERVER,
            REGCLS_MULTIPLEUSE,
        )
    }
    .map_err(|err| Error::Com(format!("could not publish the class object: {err}")))?;

    Ok(Server { cookie })
}

/// Hands out drop targets.
#[implement(IClassFactory)]
struct Factory {
    deliver: Arc<Deliver>,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, windows_core::IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        // Aggregation is a COM feature nothing here uses, and the contract is to refuse it rather
        // than ignore it.
        if !outer.is_null() {
            return Err(windows::Win32::Foundation::CLASS_E_NOAGGREGATION.into());
        }
        let target: IDropTarget = Target {
            deliver: Arc::clone(&self.deliver),
        }
        .into();
        // SAFETY: COM guarantees both pointers are valid for the call. `query` writes through
        // `object` only on success and is the standard way to honour whichever interface was asked
        // for rather than assuming `IDropTarget`.
        unsafe {
            if iid.is_null() || object.is_null() {
                return Err(E_NOINTERFACE.into());
            }
            target.query(&*iid, object).ok()
        }
    }

    fn LockServer(&self, _lock: BOOL) -> windows_core::Result<()> {
        // The process is a text editor with a window, not a server that exits when the last client
        // lets go. Its lifetime is the user's, so there is nothing to count.
        Ok(())
    }
}

/// What Explorer drops a selection onto.
#[implement(IDropTarget)]
struct Target {
    deliver: Arc<Deliver>,
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        // SAFETY: COM guarantees `effect` points at a live value for the call.
        unsafe { write_effect(effect, data.as_ref().is_some_and(carries_files)) };
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        // SAFETY: as `DragEnter`. Nothing is examined here — the answer cannot change between
        // entering and moving, because the data object does not.
        unsafe { write_effect(effect, true) };
        Ok(())
    }

    fn DragLeave(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        // SAFETY: COM guarantees `effect` points at a live value for the call.
        unsafe { write_effect(effect, false) };
        let Some(data) = data.as_ref() else {
            return Ok(());
        };
        let paths = files_in(data);
        if !paths.is_empty() {
            (self.deliver)(paths);
        }
        Ok(())
    }
}

/// Reports whether this target will take the drop.
///
/// # Safety
///
/// `effect` must point at a writable `DROPEFFECT` — which is COM's side of the contract.
unsafe fn write_effect(effect: *mut DROPEFFECT, accepted: bool) {
    if effect.is_null() {
        return;
    }
    // Copy rather than Move: opening a file in an editor does not take it from where it was, and
    // reporting Move would have Explorer delete the source on some drop sources.
    let answer = if accepted {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    };
    // SAFETY: the caller's contract, checked for null above.
    unsafe { *effect = answer };
}

/// Whether a data object offers a file list at all.
fn carries_files(data: &IDataObject) -> bool {
    // SAFETY: `format` is a live local and `data` is a borrowed COM interface. `QueryGetData` reads
    // and writes nothing of ours.
    let format = hdrop_format();
    // SAFETY: ormat is a live local for the call.
    unsafe { data.QueryGetData(&raw const format).is_ok() }
}

/// The one clipboard format this asks for: a file list in global memory.
fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0.cast_unsigned(),
    }
}

/// The paths in a dropped data object, in the order Explorer supplies them.
///
/// **Selection order is what the user sees**, so nothing sorts this: the tabs come out in the order
/// the shell gave, which is the order the acceptance case checks.
fn files_in(data: &IDataObject) -> Vec<PathBuf> {
    let format = hdrop_format();
    // SAFETY: `format` is a live local; `GetData` returns a medium this function owns and releases
    // below on every path out.
    let Ok(mut medium) = (unsafe { data.GetData(&raw const format) }) else {
        return Vec::new();
    };

    // SAFETY: the medium was asked for as `TYMED_HGLOBAL`, so the union holds a global handle.
    let handle: HGLOBAL = unsafe { medium.u.hGlobal };
    let drop = HDROP(handle.0);

    // `u32::MAX` as the index asks for the count rather than a name — the documented idiom, and the
    // reason this cannot be written as a simple loop over a known length.
    //
    // SAFETY: `drop` came from a `CF_HDROP` medium this function holds for the whole walk.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index` is below the count just read.
        let length = unsafe { DragQueryFileW(drop, index, None) };
        if length == 0 {
            continue;
        }
        // One more for the terminator, which the length excludes and the copy includes.
        let mut buffer = vec![0u16; length as usize + 1];
        // SAFETY: `buffer` is exactly as long as it says, and `DragQueryFileW` writes no further.
        let written = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
        if written == 0 {
            continue;
        }
        paths.push(PathBuf::from(String::from_utf16_lossy(
            &buffer[..written as usize],
        )));
    }

    // SAFETY: the medium came from `GetData` above and is released exactly once, after the last read
    // of the handle it owns.
    unsafe { ReleaseStgMedium(&raw mut medium) };
    paths
}

#[cfg(test)]
mod tests {
    use super::CLSID_DROP_TARGET;
    use crate::registry;

    /// **The GUID and the text must be the same class.** One is written into the registry and the
    /// other is what this process publishes; a mismatch registers a handler that Explorer activates
    /// and nothing answers, which looks exactly like the feature not working.
    #[test]
    fn clsid_matches_the_registered_text() {
        let text = format!("{{{CLSID_DROP_TARGET:?}}}");
        assert_eq!(
            text.to_ascii_uppercase(),
            registry::CLSID.to_ascii_uppercase()
        );
    }
}
