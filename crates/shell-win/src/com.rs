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

use windows::Win32::Foundation::{
    DV_E_FORMATETC, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_UNEXPECTED, ERROR_FILE_NOT_FOUND,
    GlobalFree, HGLOBAL, POINT, POINTL,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
    CoAllowSetForegroundWindow, CoInitializeEx, CoLockObjectExternal, CoRegisterClassObject,
    CoRevokeClassObject, CoUninitialize, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IClassFactory,
    IClassFactory_Impl, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA,
    REGCLS_MULTIPLEUSE, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
};
use windows::Win32::System::Ole::{
    ACTIVEOBJECT_WEAK, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, GetActiveObject,
    IDropTarget, IDropTarget_Impl, RegisterActiveObject, ReleaseStgMedium, RevokeActiveObject,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};
use windows_core::{BOOL, GUID, IUnknown, Interface, PCWSTR, Ref, implement};

use crate::registry::{CLASSES, Step};
use crate::{Claim, Deliver, Error, HandedOff};

/// The class Explorer activates, as a GUID rather than as text.
///
/// [`clsid_matches_the_registered_text`] holds it against [`crate::registry::CLSID`], because the
/// two are written in different alphabets and a mismatch would be a registration that succeeds and
/// an activation that finds nothing.
pub(crate) const CLSID_DROP_TARGET: GUID =
    GUID::from_u128(0x9F2B_4C6D_1E3A_4F58_9B7C_2D5E_8A0F_1C34);

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
    // The Running Object Table entry that tells a second launch an editor is here. `None` if the
    // table refused, which costs the hand-off and nothing else.
    active: Option<u32>,
    // Why `active` is `None`, for the caller to log.
    unavailable: Option<String>,
    // A weak registration does not keep the object alive, so this does; `Drop` also unlocks it.
    target: IDropTarget,
}

impl Server {
    /// Why a second launch cannot find this process, if it cannot.
    #[must_use]
    pub fn hand_off_unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // SAFETY: `cookie` came from `CoRegisterClassObject` and is revoked exactly once, because
        // `Server` is not `Clone` and this is its only drop.
        let _ = unsafe { CoRevokeClassObject(self.cookie) };
        if let Some(active) = self.active {
            // SAFETY: `active` came from `RegisterActiveObject` and is revoked exactly once, for the
            // same reason as `cookie`.
            let _ = unsafe { RevokeActiveObject(active, None) };
            // SAFETY: balances the lock taken after registering; the target is still alive.
            let _ = unsafe { CoLockObjectExternal(&self.target, false, true) };
        }
    }
}

/// Publishes the class object. See [`crate::serve`].
pub fn serve(deliver: Deliver) -> Result<Server, Error> {
    serve_class(deliver, &CLSID_DROP_TARGET)
}

/// [`serve`] under a given class, so tests in parallel processes do not share one table entry.
pub fn serve_class(deliver: Deliver, class: &GUID) -> Result<Server, Error> {
    // Apartment-threaded, because that is what a UI thread is and what `IDropTarget` requires. An
    // already-initialised apartment answers `S_FALSE`, and one initialised the *other* way answers
    // `RPC_E_CHANGED_MODE` — neither is a failure of ours, and winit initialises OLE itself when the
    // loop starts, so both are reachable depending on call order.
    //
    // SAFETY: no arguments are borrowed, and the matching uninitialise is deliberately absent —
    // the apartment lives as long as the process, and tearing it down while winit still owns
    // windows on this thread would be the bug.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let deliver = Arc::new(deliver);
    let factory: IClassFactory = Factory {
        deliver: Arc::clone(&deliver),
    }
    .into();

    // `REGCLS_MULTIPLEUSE` is the whole reuse story: the class stays published after the first
    // activation, so the second, third and twentieth Explorer request bind to this process rather
    // than starting another. `REGCLS_SINGLEUSE` would revoke after one and reintroduce exactly the
    // per-file process the ticket exists to remove.
    //
    // SAFETY: `factory` outlives the call — COM takes its own reference — and the GUID is a
    // 'static constant.
    let cookie =
        unsafe { CoRegisterClassObject(class, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE) }
            .map_err(|err| Error::Com(format!("could not publish the class object: {err}")))?;

    // The same target Explorer is handed, published where a second launch can find it without
    // asking the registry or starting anything: looking in the table asks only whether a live
    // process holds the entry. That is what keeps the cold path "this process becomes the editor"
    // rather than "COM starts a server first".
    let target: IDropTarget = Target { deliver }.into();
    let mut active = 0u32;
    // SAFETY: `target` is a live COM object, the GUID is a constant and `active` is a live local.
    let registered =
        unsafe { RegisterActiveObject(&target, class, ACTIVEOBJECT_WEAK, &raw mut active) };

    if registered.is_ok() {
        // A weak registration lets go of the stub when the last external connection does, so the
        // second hand-off of an editor's life would find nothing. The lock keeps it for as long as
        // the entry is published; `Drop` undoes it.
        //
        // SAFETY: `target` is a live COM object.
        let _ = unsafe { CoLockObjectExternal(&target, true, false) };
    }
    Ok(Server {
        cookie,
        active: registered.is_ok().then_some(active),
        unavailable: registered
            .err()
            .map(|err| format!("a second launch cannot hand files to this editor: {err}")),
        target,
    })
}

/// Offers `paths` to the editor in the Running Object Table. See [`crate::hand_off`].
///
/// Runs on a thread of its own, so it joins the multithreaded apartment: nothing here needs a message
/// loop, and the call into the editor's apartment is marshalled and waits for its UI thread.
pub fn hand_off_class(paths: &[PathBuf], claim: &Claim, class: &GUID) -> Result<HandedOff, Error> {
    // SAFETY: no arguments are borrowed. Balanced below only when it succeeded; a thread already in
    // the other mode answers an error and is left as it was.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let _guard = initialised.then_some(Apartment);

    let mut found: Option<IUnknown> = None;
    // SAFETY: the GUID is a constant and `found` is a live local the call writes through.
    let looked = unsafe { GetActiveObject(class, None, &raw mut found) };
    let Some(running) = found.filter(|_| looked.is_ok()) else {
        return Ok(HandedOff::NobodyListening);
    };

    let target: IDropTarget = running
        .cast()
        .map_err(|err| Error::Com(format!("the running editor is not a drop target: {err}")))?;
    // A foreground-lock exemption for the server, which is what lets its window come forward rather
    // than flash in the taskbar. Failing only costs focus.
    //
    // SAFETY: `running` is a live interface pointer for the call.
    let _ = unsafe { CoAllowSetForegroundWindow(&running, None) };

    let data: IDataObject = Outgoing {
        paths: paths.to_vec(),
    }
    .into();
    let mut effect = DROPEFFECT_COPY;
    let at = POINTL { x: 0, y: 0 };
    // The sender may already have given up; if so nothing is sent, so nothing can arrive late.
    if !claim.begin_delivery() {
        return Ok(HandedOff::NobodyListening);
    }
    // SAFETY: a live data object, a live `POINTL` and a live `DROPEFFECT`, which is the whole of what
    // these calls read and write. Enter then drop is the sequence Explorer uses.
    unsafe { target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), at, &raw mut effect) }
        .map_err(|err| Error::Com(format!("the running editor refused the files: {err}")))?;
    // SAFETY: as above.
    match unsafe { target.Drop(&data, MODIFIERKEYS_FLAGS(0), at, &raw mut effect) } {
        Ok(()) => Ok(HandedOff::Delivered),
        // The receiver delivers before it returns, so an error here (a dropped connection, a panic
        // contained on its side) may come after the tabs opened. Starting a window as well would
        // open them twice, which is worse than the rare case where they never opened.
        Err(err) => Ok(HandedOff::Uncertain(format!(
            "the drop failed after the editor was offered the files: {err}"
        ))),
    }
}

/// Balances a successful `CoInitializeEx` on this thread.
struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: only constructed after a `CoInitializeEx` that succeeded on this thread.
        unsafe { CoUninitialize() };
    }
}

/// A `CF_HDROP` data object over a list of paths: what Explorer hands `Drop`, built by the sender.
#[implement(IDataObject)]
struct Outgoing {
    paths: Vec<PathBuf>,
}

/// Whether a `FORMATETC` asks for a file list in global memory.
///
/// # Safety
///
/// `wanted` must be null or point at a live `FORMATETC`, which is COM's side of the contract.
unsafe fn asks_for_files(wanted: *const FORMATETC) -> bool {
    // SAFETY: the caller's contract.
    unsafe { wanted.as_ref() }.is_some_and(|format| {
        format.cfFormat == CF_HDROP.0 && format.tymed & TYMED_HGLOBAL.0.cast_unsigned() != 0
    })
}

impl IDataObject_Impl for Outgoing_Impl {
    fn GetData(&self, wanted: *const FORMATETC) -> windows_core::Result<STGMEDIUM> {
        // SAFETY: COM passes a live `FORMATETC`.
        if !unsafe { asks_for_files(wanted) } {
            return Err(DV_E_FORMATETC.into());
        }
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0.cast_unsigned(),
            u: STGMEDIUM_0 {
                hGlobal: hdrop_global(&self.paths)?,
            },
            // `None` hands ownership of the allocation to the receiver, who releases it.
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        })
    }

    fn QueryGetData(&self, wanted: *const FORMATETC) -> windows_core::HRESULT {
        // SAFETY: COM passes a live `FORMATETC`.
        if unsafe { asks_for_files(wanted) } {
            windows_core::HRESULT(0)
        } else {
            DV_E_FORMATETC
        }
    }

    // Everything else is `E_NOTIMPL`: a receiver needing more is asking for something Explorer's
    // selection does not offer either.
    fn GetDataHere(&self, _f: *const FORMATETC, _m: *mut STGMEDIUM) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalFormatEtc(
        &self,
        _in: *const FORMATETC,
        _out: *mut FORMATETC,
    ) -> windows_core::HRESULT {
        E_NOTIMPL
    }
    fn SetData(
        &self,
        _f: *const FORMATETC,
        _m: *const STGMEDIUM,
        _release: BOOL,
    ) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumFormatEtc(&self, _direction: u32) -> windows_core::Result<IEnumFORMATETC> {
        Err(E_NOTIMPL.into())
    }
    fn DAdvise(
        &self,
        _f: *const FORMATETC,
        _advf: u32,
        _sink: Ref<'_, IAdviseSink>,
    ) -> windows_core::Result<u32> {
        Err(E_NOTIMPL.into())
    }
    fn DUnadvise(&self, _connection: u32) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumDAdvise(&self) -> windows_core::Result<IEnumSTATDATA> {
        Err(E_NOTIMPL.into())
    }
}

/// A `CF_HDROP` block in movable global memory: a `DROPFILES` header and a double-NUL-terminated list
/// of wide paths, in the order given and cut at [`MAX_FILES`], which is what the receiver reads.
///
/// Wide units are taken from the `OsStr` as they are, so names with unpaired surrogates survive. A
/// NUL inside a name would end the list early, so such a name is left out rather than split.
fn hdrop_global(paths: &[PathBuf]) -> windows_core::Result<HGLOBAL> {
    use std::os::windows::ffi::OsStrExt;

    let mut list: Vec<u16> = Vec::new();
    for path in paths.iter().take(MAX_FILES as usize) {
        let name: Vec<u16> = path.as_os_str().encode_wide().collect();
        if name.is_empty() || name.contains(&0) {
            continue;
        }
        list.extend(name);
        list.push(0);
    }
    list.push(0);

    let header = size_of::<DROPFILES>();
    let bytes = header + list.len() * 2;
    // SAFETY: an allocation of a size that fits both parts, freed on every path below that does not
    // hand it to the caller.
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) }?;
    // SAFETY: `handle` is the live allocation from above.
    let base = unsafe { GlobalLock(handle) };
    if base.is_null() {
        // SAFETY: allocated above and not yet shared.
        let _ = unsafe { GlobalFree(Some(handle)) };
        return Err(E_OUTOFMEMORY.into());
    }
    // SAFETY: `base` points at `bytes` writable bytes, locked until the unlock below. Unaligned
    // writes, so nothing depends on where the allocator put the block.
    unsafe {
        base.cast::<DROPFILES>().write_unaligned(DROPFILES {
            pFiles: u32::try_from(header).unwrap_or(0),
            pt: POINT { x: 0, y: 0 },
            fNC: BOOL(0),
            fWide: BOOL(1),
        });
        std::ptr::copy_nonoverlapping(
            list.as_ptr().cast::<u8>(),
            base.cast::<u8>().add(header),
            list.len() * 2,
        );
        // The remaining lock count is the answer, and is expected to be zero.
        let _ = GlobalUnlock(handle);
    }
    Ok(handle)
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

/// The most files one drop is taken to carry.
///
/// Not a limit on what a person can select — Explorer's own multi-select runs out of patience long
/// before this, and MDEDIT-61's acceptance case was twenty. It is a bound on what a *hostile* data
/// object can claim: the count comes from `DragQueryFileW` reading memory the other process filled
/// in, and `Vec::with_capacity` on a claim of four billion is a multi-gigabyte allocation and an
/// abort, from a caller that only had to write a number. MDEDIT-104.
const MAX_FILES: u32 = 4096;

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

        // **A panic here aborts the editor.** This runs inside a `#[implement]` vtable shim, and that
        // ABI does not unwind — so a panic anywhere below, in code reading a data object supplied by
        // another process or in the delivery closure supplied by the caller, takes the whole process
        // down at the exact moment the user dropped files on it. MDEDIT-104.
        //
        // `AssertUnwindSafe` because `Deliver` is a `Fn` the caller owns and this crate cannot
        // constrain; the assertion is that a panicking delivery leaves the editor's state to the
        // editor's own panic handling, which is where it belongs. The error goes back to the shell
        // rather than being swallowed, and the panic hook has already written the reason to the log.
        let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let paths = files_in(data);
            if !paths.is_empty() {
                (self.deliver)(paths);
            }
        }));
        if delivered.is_err() {
            return Err(E_UNEXPECTED.into());
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

    // **A medium that is not what was asked for is not released either.** `ReleaseStgMedium`
    // dispatches on `tymed` as well, so freeing a lying medium is the same wrong-union-arm hazard as
    // reading one — it would call `Release` through a pointer that is not an interface. Found by
    // driving a hostile data object through this path: the read was refused and the *release* then
    // took the process down with an access violation. Leaking a hostile caller's own allocation is
    // much the better of the two, and a conforming one is released as it always was.
    if medium.tymed != TYMED_HGLOBAL.0.cast_unsigned() {
        return Vec::new();
    }

    let paths = paths_in(&medium);
    // SAFETY: the medium came from `GetData` above, its `tymed` is the one checked immediately
    // above, and it is released exactly once after the last read of the handle it owns.
    unsafe { ReleaseStgMedium(&raw mut medium) };
    paths
}

/// The paths a `CF_HDROP` medium holds, or nothing if it is not one.
///
/// Split out of [`files_in`] because this is the trust boundary and the COM call is not: **this
/// class is activatable by any local process**, so everything the medium says about itself is a
/// claim by somebody else's code. Taking it apart here is also what makes it testable without a
/// shell.
///
/// The medium is borrowed and **not** released here; that is the caller's, on every path out.
fn paths_in(medium: &STGMEDIUM) -> Vec<PathBuf> {
    // **Check what came back, not what was asked for.** `IDataObject::GetData` is implemented by the
    // caller's object, so a non-conforming or hostile one can return `S_OK` with a different `tymed`
    // — and reading the union on the strength of the *request* would reinterpret, say, an `IStream*`
    // as an `HGLOBAL`, hand it to `DragQueryFileW`, and then release it under the wrong union arm.
    // MDEDIT-104; the old SAFETY comment here argued from the request.
    if medium.tymed != TYMED_HGLOBAL.0.cast_unsigned() {
        return Vec::new();
    }

    // SAFETY: the union arm is the one `tymed` names, checked immediately above.
    let handle: HGLOBAL = unsafe { medium.u.hGlobal };
    if handle.0.is_null() {
        return Vec::new();
    }
    let drop = HDROP(handle.0);

    // `u32::MAX` as the index asks for the count rather than a name — the documented idiom, and the
    // reason this cannot be written as a simple loop over a known length.
    //
    // SAFETY: `drop` came from a `CF_HDROP` medium the caller holds for the whole walk.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) }.min(MAX_FILES);
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

    paths
}

#[cfg(test)]
mod tests {
    // A test fixture that cannot allocate has nothing to say about the code under test, and the
    // pointer arithmetic below is building a Win32 structure by hand — which is the point of it.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::cast_ptr_alignment
    )]

    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use windows::Win32::Foundation::{E_NOTIMPL, GlobalFree, HGLOBAL, POINT, POINTL};
    use windows::Win32::System::Com::IAdviseSink;
    use windows::Win32::System::Com::{FORMATETC, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA};
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, IDropTarget};
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows_core::{Ref, implement};

    use windows::Win32::System::Com::{IDataObject, STGMEDIUM, TYMED_HGLOBAL, TYMED_ISTREAM};
    use windows::Win32::System::Memory::{GMEM_FIXED, GMEM_ZEROINIT, GlobalAlloc};
    use windows::Win32::UI::Shell::DROPFILES;
    use windows_core::BOOL;

    use super::{CLSID_DROP_TARGET, MAX_FILES, Target, paths_in};
    use crate::{Deliver, registry};

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

    // ---- the drop path's inputs, which come from another process (MDEDIT-104) ----

    /// A real `CF_HDROP` block in global memory, as Explorer builds one: a `DROPFILES` header
    /// followed by a double-NUL-terminated list of wide paths.
    ///
    /// Owns the allocation and frees it on drop, so a failing assertion does not leak one.
    struct Hdrop(HGLOBAL);

    impl Hdrop {
        fn of(paths: &[&str]) -> Self {
            let mut list: Vec<u16> = Vec::new();
            for path in paths {
                list.extend(path.encode_utf16());
                list.push(0);
            }
            // The second terminator ends the list, which is how the count is derived at all.
            list.push(0);

            let header = size_of::<DROPFILES>();
            let bytes = header + list.len() * 2;
            // SAFETY: a fixed, zeroed allocation of a size that fits both parts. `GMEM_FIXED` means
            // the handle *is* the pointer, so nothing has to be locked to write through it.
            let handle = unsafe { GlobalAlloc(GMEM_FIXED | GMEM_ZEROINIT, bytes) }
                .expect("the test allocation should succeed");

            // SAFETY: `handle` is a live allocation of at least `header` bytes, written once.
            unsafe {
                handle.0.cast::<DROPFILES>().write(DROPFILES {
                    pFiles: u32::try_from(header).expect("a header this small fits"),
                    pt: POINT { x: 0, y: 0 },
                    fNC: BOOL(0),
                    fWide: BOOL(1),
                });
                // SAFETY: the list starts `header` bytes in and is exactly as long as it says.
                let names = handle.0.cast::<u8>().add(header).cast::<u16>();
                std::ptr::copy_nonoverlapping(list.as_ptr(), names, list.len());
            }
            Self(handle)
        }

        /// Hands the allocation to something else, which then owns freeing it.
        ///
        /// `ReleaseStgMedium` frees an `HGLOBAL` medium with no `pUnkForRelease`, so a fixture that
        /// also freed it would be a double free — and this crate's whole `Drop` path ends in exactly
        /// that release.
        fn into_handle(self) -> HGLOBAL {
            std::mem::ManuallyDrop::new(self).0
        }

        fn medium(&self, tymed: u32) -> STGMEDIUM {
            STGMEDIUM {
                tymed,
                u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: self.0 },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            }
        }
    }

    impl Drop for Hdrop {
        fn drop(&mut self) {
            // SAFETY: the handle came from `GlobalAlloc` above and is freed exactly once.
            let _ = unsafe { GlobalFree(Some(self.0)) };
        }
    }

    /// The shape MDEDIT-61 drove by hand through a real shell and never kept as a test.
    #[test]
    fn a_conforming_medium_yields_its_paths_in_order() {
        let block = Hdrop::of(&[r"C:\notes\one.md", r"C:\notes\two.md", r"C:\notes\three.md"]);

        let paths = paths_in(&block.medium(TYMED_HGLOBAL.0.cast_unsigned()));

        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"C:\notes\one.md"),
                PathBuf::from(r"C:\notes\two.md"),
                PathBuf::from(r"C:\notes\three.md"),
            ],
            "selection order is what the user sees, so nothing may sort this"
        );
    }

    /// **The defect.** `GetData` is implemented by the caller's object, so the `tymed` that comes
    /// back is its claim rather than our request. Reading the union on the strength of the request
    /// reinterprets whatever it returned — an `IStream*`, here — as an `HGLOBAL`, hands it to
    /// `DragQueryFileW`, and then releases it under the wrong union arm.
    ///
    /// The handle below is a perfectly good `HDROP`; only the `tymed` lies. That is what makes this
    /// test about the check rather than about the pointer.
    #[test]
    fn a_medium_that_is_not_global_memory_is_refused() {
        let block = Hdrop::of(&[r"C:\notes\one.md"]);

        let paths = paths_in(&block.medium(TYMED_ISTREAM.0.cast_unsigned()));

        assert!(paths.is_empty(), "got {paths:?}");
    }

    #[test]
    fn a_null_handle_is_refused() {
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0.cast_unsigned(),
            u: windows::Win32::System::Com::STGMEDIUM_0 {
                hGlobal: HGLOBAL(std::ptr::null_mut()),
            },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        };

        assert!(paths_in(&medium).is_empty());
    }

    /// An empty list is a drop of nothing, not a crash.
    #[test]
    fn an_empty_list_yields_nothing() {
        let block = Hdrop::of(&[]);
        assert!(paths_in(&block.medium(TYMED_HGLOBAL.0.cast_unsigned())).is_empty());
    }

    /// A zero-length name is skipped rather than becoming an empty path, which would open a tab on
    /// nothing.
    #[test]
    fn a_zero_length_name_is_skipped() {
        let block = Hdrop::of(&[r"C:\notes\one.md", "", r"C:\notes\two.md"]);

        let paths = paths_in(&block.medium(TYMED_HGLOBAL.0.cast_unsigned()));

        assert!(
            !paths.iter().any(|path| path.as_os_str().is_empty()),
            "got {paths:?}"
        );
    }

    /// The count comes from memory the other process filled in, and `Vec::with_capacity` on a claim
    /// of four billion is a multi-gigabyte allocation and an abort.
    ///
    /// A hand-built list is the only count `DragQueryFileW` will report here — the four-billion claim
    /// cannot be produced through the real API — so what this pins is the cap itself, at a size no
    /// real selection reaches.
    #[test]
    fn a_list_longer_than_the_cap_is_cut_to_it() {
        let names: Vec<String> = (0..MAX_FILES + 500)
            .map(|index| format!(r"C:\n\{index}.md"))
            .collect();
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        let block = Hdrop::of(&borrowed);

        let paths = paths_in(&block.medium(TYMED_HGLOBAL.0.cast_unsigned()));

        assert_eq!(paths.len(), MAX_FILES as usize);
    }

    // ---- the whole `Drop`, driven without a shell ----

    /// A data object that answers `GetData` with whatever it was built with.
    ///
    /// MDEDIT-61 built one of these by hand to drive the feature through `CoCreateInstance`, and did
    /// not keep it. This is that, kept: it is what makes the vtable boundary — the one place a panic
    /// aborts the editor — reachable from a test at all.
    #[implement(IDataObject)]
    struct Offering {
        handle: HGLOBAL,
        tymed: u32,
    }

    impl IDataObject_Impl for Offering_Impl {
        fn GetData(&self, _wanted: *const FORMATETC) -> windows_core::Result<STGMEDIUM> {
            Ok(STGMEDIUM {
                tymed: self.tymed,
                u: windows::Win32::System::Com::STGMEDIUM_0 {
                    hGlobal: self.handle,
                },
                // `None` means the medium owns its own handle, so `ReleaseStgMedium` frees it. The
                // fixture must therefore not free it a second time — see the caller.
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            })
        }

        fn QueryGetData(&self, _wanted: *const FORMATETC) -> windows_core::HRESULT {
            windows_core::HRESULT(0)
        }

        // Everything else is `E_NOTIMPL`, because nothing in this crate calls it. A data object that
        // answered more would be testing the fake rather than the code.
        fn GetDataHere(
            &self,
            _f: *const FORMATETC,
            _m: *mut STGMEDIUM,
        ) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn GetCanonicalFormatEtc(
            &self,
            _in: *const FORMATETC,
            _out: *mut FORMATETC,
        ) -> windows_core::HRESULT {
            E_NOTIMPL
        }
        fn SetData(
            &self,
            _f: *const FORMATETC,
            _m: *const STGMEDIUM,
            _release: BOOL,
        ) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn EnumFormatEtc(&self, _direction: u32) -> windows_core::Result<IEnumFORMATETC> {
            Err(E_NOTIMPL.into())
        }
        fn DAdvise(
            &self,
            _f: *const FORMATETC,
            _advf: u32,
            _sink: Ref<'_, IAdviseSink>,
        ) -> windows_core::Result<u32> {
            Err(E_NOTIMPL.into())
        }
        fn DUnadvise(&self, _connection: u32) -> windows_core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn EnumDAdvise(&self) -> windows_core::Result<IEnumSTATDATA> {
            Err(E_NOTIMPL.into())
        }
    }

    /// Drives one drop through the real vtable and returns what the target reported.
    fn drop_onto(offering: &IDataObject, deliver: Deliver) -> windows_core::Result<()> {
        let target: IDropTarget = Target {
            deliver: Arc::new(deliver),
        }
        .into();

        let mut effect = DROPEFFECT_COPY;
        let at = POINTL { x: 0, y: 0 };
        // SAFETY: a live data object, a live `POINTL` and a live `DROPEFFECT`, which is the whole of
        // what this call reads and writes.
        unsafe { target.Drop(Some(offering), MODIFIERKEYS_FLAGS(0), at, &raw mut effect) }
    }

    /// End to end: a conforming data object, and the paths come out in order.
    #[test]
    fn a_conforming_data_object_delivers_its_files() {
        let block = Hdrop::of(&[r"C:\notes\one.md", r"C:\notes\two.md"]);
        let offering: IDataObject = Offering {
            handle: block.into_handle(),
            tymed: TYMED_HGLOBAL.0.cast_unsigned(),
        }
        .into();

        let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let answer = drop_onto(
            &offering,
            Box::new(move |paths| {
                recorder
                    .lock()
                    .expect("no other thread holds it")
                    .extend(paths);
            }),
        );

        assert!(answer.is_ok());
        assert_eq!(
            *seen.lock().expect("no other thread holds it"),
            vec![
                PathBuf::from(r"C:\notes\one.md"),
                PathBuf::from(r"C:\notes\two.md")
            ]
        );
    }

    /// And a hostile one delivers nothing, rather than reinterpreting whatever it returned.
    #[test]
    fn a_data_object_lying_about_its_medium_delivers_nothing() {
        let block = Hdrop::of(&[r"C:\notes\one.md"]);
        let offering: IDataObject = Offering {
            handle: block.into_handle(),
            tymed: TYMED_ISTREAM.0.cast_unsigned(),
        }
        .into();

        let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let answer = drop_onto(
            &offering,
            Box::new(move |paths| {
                recorder
                    .lock()
                    .expect("no other thread holds it")
                    .extend(paths);
            }),
        );

        assert!(answer.is_ok(), "a refusal is not an error to report");
        assert!(seen.lock().expect("no other thread holds it").is_empty());
    }

    /// **A panic must not cross the vtable.** `IDropTarget::Drop` is called through a `#[implement]`
    /// shim whose ABI does not unwind, so without containment a panicking delivery closure takes the
    /// whole editor down at the moment the user dropped files on it — the least forgivable time.
    ///
    /// Without the `catch_unwind` this test does not fail; the process aborts and takes the rest of
    /// the suite with it, which under nextest is one test binary rather than all of them.
    #[test]
    fn a_panicking_delivery_does_not_cross_the_vtable() {
        let block = Hdrop::of(&[r"C:\notes\one.md"]);
        let offering: IDataObject = Offering {
            handle: block.into_handle(),
            tymed: TYMED_HGLOBAL.0.cast_unsigned(),
        }
        .into();

        // The panic message would otherwise be printed by the default hook and read as a failure.
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let answer = drop_onto(
            &offering,
            Box::new(|_| panic!("the delivery closure belongs to the caller")),
        );
        std::panic::set_hook(previous);

        assert!(
            answer.is_err(),
            "the failure is reported to the shell rather than swallowed"
        );
    }

    // ---- the sender (MDEDIT-204) ----

    fn round_trip(paths: &[PathBuf]) -> Vec<PathBuf> {
        let handle = super::hdrop_global(paths).expect("the allocation should succeed");
        let mut medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0.cast_unsigned(),
            u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: handle },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        };
        let read = paths_in(&medium);
        // SAFETY: the medium owns `handle`, is an HGLOBAL one, and is released once.
        unsafe { windows::Win32::System::Ole::ReleaseStgMedium(&raw mut medium) };
        read
    }

    /// What the sender builds, the receiver's own reader takes apart: order, spaces and non-ASCII
    /// kept.
    #[test]
    fn what_the_sender_builds_the_receiver_reads() {
        let paths: Vec<PathBuf> = [
            r"C:\notes\one.md",
            r"C:\my notes\two words.md",
            r"C:\日本語\ünï cödé.md",
            r"\server\share\x.md",
        ]
        .map(PathBuf::from)
        .into();
        assert_eq!(round_trip(&paths), paths);
        assert!(round_trip(&[]).is_empty());
    }

    #[test]
    fn the_sender_cuts_at_the_same_cap_the_receiver_reads() {
        let paths: Vec<PathBuf> = (0..MAX_FILES + 10)
            .map(|index| PathBuf::from(format!(r"C:\n\{index}.md")))
            .collect();
        let read = round_trip(&paths);
        assert_eq!(read.len(), MAX_FILES as usize);
        assert_eq!(read[..], paths[..MAX_FILES as usize]);
    }

    #[test]
    fn a_name_with_a_nul_is_left_out_not_split() {
        let paths = vec![
            PathBuf::from(r"C:\a.md"),
            PathBuf::from("C:\\b\0c.md"),
            PathBuf::from(r"C:\d.md"),
        ];
        assert_eq!(
            round_trip(&paths),
            vec![PathBuf::from(r"C:\a.md"), PathBuf::from(r"C:\d.md")]
        );
    }

    // ---- the whole route. Each test publishes under a class of its own: the table is shared by
    // every test process running at once, and an editor from another test must not answer. ----

    use std::time::Duration;

    fn serve_in(
        class: &windows_core::GUID,
        deliver: impl Fn(Vec<PathBuf>) + Send + Sync + 'static,
    ) -> super::Server {
        super::serve_class(Box::new(deliver), class).expect("the class should publish")
    }

    /// Pumps this thread's messages (the STA's obligation to the calls routed to it) until `result`
    /// has an answer.
    fn pump_until<T>(result: &std::sync::mpsc::Receiver<T>) -> T {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let mut message = MSG::default();
            // SAFETY: a live `MSG`.
            while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                // SAFETY: the message came from the call above.
                unsafe {
                    let _ = TranslateMessage(&raw const message);
                    DispatchMessageW(&raw const message);
                }
            }
            if let Ok(answer) = result.try_recv() {
                return answer;
            }
            assert!(std::time::Instant::now() < deadline, "the hand-off hung");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Runs one hand-off from a second thread while this one pumps, as an editor's UI thread does.
    fn hand_off_while_pumping(
        paths: Vec<PathBuf>,
        limit: Duration,
        class: windows_core::GUID,
    ) -> Result<crate::HandedOff, crate::Error> {
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || done.send(crate::hand_off_within(&paths, limit, &class)));
        pump_until(&result)
    }

    fn sent_paths() -> Vec<PathBuf> {
        vec![
            PathBuf::from(r"C:\my notes\one.md"),
            PathBuf::from(r"C:\日本語\two.md"),
        ]
    }

    /// Nobody has published the entry, so there is nobody to hand to, and nothing was started.
    #[test]
    fn with_no_editor_published_nobody_is_listening() {
        let class = windows_core::GUID::new().expect("a fresh GUID");
        let answer = crate::hand_off_within(&sent_paths(), Duration::from_secs(5), &class);
        assert!(
            matches!(answer, Ok(crate::HandedOff::NobodyListening)),
            "{answer:?}"
        );
    }

    /// This thread is the editor's UI thread, publishing through `serve`; another thread is the
    /// second launch, finding it in the Running Object Table. **In one process COM hands out the
    /// object directly** (a `#[implement]` object is agile), so this proves the lookup, the data
    /// object and the receiver, not cross-process marshalling; that is checked by hand.
    #[test]
    fn a_hand_off_reaches_a_published_server_across_apartments() {
        let class = windows_core::GUID::new().expect("a fresh GUID");
        let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let _server = serve_in(&class, move |paths| {
            recorder.lock().expect("uncontended").extend(paths);
        });

        let answer = hand_off_while_pumping(sent_paths(), Duration::from_secs(20), class);

        assert!(
            matches!(answer, Ok(crate::HandedOff::Delivered)),
            "{answer:?}"
        );
        assert_eq!(*seen.lock().expect("uncontended"), sent_paths());
    }

    /// The entry outlives its first client: a second hand-off finds it.
    #[test]
    fn a_second_hand_off_still_finds_the_server() {
        let class = windows_core::GUID::new().expect("a fresh GUID");
        let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let _server = serve_in(&class, move |paths| {
            recorder.lock().expect("uncontended").extend(paths);
        });

        for _ in 0..2 {
            let answer = hand_off_while_pumping(sent_paths(), Duration::from_secs(20), class);
            assert!(
                matches!(answer, Ok(crate::HandedOff::Delivered)),
                "{answer:?}"
            );
        }
        assert_eq!(seen.lock().expect("uncontended").len(), 4);
    }

    /// **The timeout path through the hand-off itself.** Delivery is in flight and the editor is busy
    /// past the bound: not a fallback, because the files are on their way. Without the bounded wait
    /// this waits out the editor and answers `Delivered`, so the assertion fails.
    #[test]
    fn a_busy_editor_past_the_bound_is_not_a_fallback() {
        let class = windows_core::GUID::new().expect("a fresh GUID");
        let _server = serve_in(&class, |_| std::thread::sleep(Duration::from_millis(1500)));

        let answer = hand_off_while_pumping(sent_paths(), Duration::from_millis(300), class);

        assert!(
            matches!(answer, Ok(crate::HandedOff::Uncertain(_))),
            "{answer:?}"
        );
    }
}
