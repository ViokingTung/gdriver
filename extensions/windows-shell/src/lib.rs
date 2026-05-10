// ─── gDriver Windows Shell Extension ──────────────────────────────────────
//
// A COM DLL that provides:
//   - Icon overlays for sync status (IShellIconOverlayIdentifier)
//   - Right-click context menu items (IContextMenu)
//
// Communication with gdriver-daemon happens over Named Pipes using
// JSON-RPC 2.0 (matching the IPC protocol used by the Rust daemon and
// the Linux file manager extensions).

mod context_menu;
mod ipc;
mod overlay;

use std::ffi::c_void;

use windows::{
    core::*,
    Win32::{Foundation::*, System::Com::*, UI::Shell::*},
};

// ─── DLL Exports ────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn DllMain(_hinst: HINSTANCE, reason: u32, _reserved: *const c_void) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    const DLL_PROCESS_DETACH: u32 = 0;

    match reason {
        DLL_PROCESS_ATTACH => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter("gdriver_shell=info")
                .try_init();
        }
        DLL_PROCESS_DETACH => {}
        _ => {}
    }

    1
}

#[no_mangle]
pub extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        let clsid_ref = &*clsid;
        let iid_ref = &*iid;

        let factory: Option<IClassFactory> = if *clsid_ref == overlay::CLSID_CLOUD {
            Some(ClassFactory::new_for::<
                overlay::CloudOverlay,
                IShellIconOverlayIdentifier,
            >())
        } else if *clsid_ref == overlay::CLSID_SYNCING {
            Some(ClassFactory::new_for::<
                overlay::SyncingOverlay,
                IShellIconOverlayIdentifier,
            >())
        } else if *clsid_ref == overlay::CLSID_SYNCED {
            Some(ClassFactory::new_for::<
                overlay::SyncedOverlay,
                IShellIconOverlayIdentifier,
            >())
        } else if *clsid_ref == overlay::CLSID_ERROR {
            Some(ClassFactory::new_for::<
                overlay::ErrorOverlay,
                IShellIconOverlayIdentifier,
            >())
        } else if *clsid_ref == overlay::CLSID_UPLOADING {
            Some(ClassFactory::new_for::<
                overlay::UploadingOverlay,
                IShellIconOverlayIdentifier,
            >())
        } else if *clsid_ref == CLSID_CONTEXT_MENU {
            Some(ClassFactory::new_for::<
                context_menu::GDriverContextMenu,
                IContextMenu,
            >())
        } else {
            None
        };

        match factory {
            Some(f) => f.query(iid_ref, ppv),
            None => CLASS_E_CLASSNOTAVAILABLE,
        }
    }
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_OK
}

#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    S_OK
}

#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    S_OK
}

const CLSID_CONTEXT_MENU: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556600);

// ─── Class Factory ──────────────────────────────────────────────────────────

#[implement(IClassFactory)]
struct ClassFactory {
    create_fn: Box<dyn Fn() -> IUnknown>,
}

impl ClassFactory {
    fn new_for<T, I>() -> IClassFactory
    where
        T: Default + 'static,
        I: Interface + From<T>,
    {
        let factory = ClassFactory {
            create_fn: Box::new(|| {
                let obj = T::default();
                let iface: I = obj.into();
                let mut unk_ptr: *mut c_void = std::ptr::null_mut();
                unsafe {
                    let _ = iface.query(
                        &IUnknown::IID as *const GUID,
                        &mut unk_ptr as *mut *mut c_void,
                    );
                    Interface::from_raw(unk_ptr)
                }
            }),
        };
        factory.into()
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> Result<()> {
        if punkouter.is_some() {
            return Err(Error::from_hresult(CLASS_E_NOAGGREGATION));
        }

        let inner = self.get_impl();
        let unk = (inner.create_fn)();
        unsafe {
            let riid_ref = &*riid;
            let _ = unk.query(riid_ref, ppv);
        }
        Ok(())
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

// ─── Helper: open URL in default browser ────────────────────────────────────

mod open {
    use std::process::Command;

    pub fn that(url: &str) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(target_os = "windows")]
        {
            Command::new("cmd").args(["/c", "start", "", url]).spawn()?;
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = url;
        }

        Ok(())
    }
}
