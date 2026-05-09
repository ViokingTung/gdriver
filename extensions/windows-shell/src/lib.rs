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

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::*;

// ─── DLL Exports ────────────────────────────────────────────────────────────
//
// These are the standard COM DLL entry points that Windows Explorer calls
// when loading the shell extension.

#[no_mangle]
pub extern "system" fn DllMain(
    _hinst: HINSTANCE,
    reason: u32,
    _reserved: *const c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    const DLL_PROCESS_DETACH: u32 = 0;

    match reason {
        DLL_PROCESS_ATTACH => {
            // Initialize logging
            let _ = tracing_subscriber::fmt()
                .with_env_filter("gdriver_shell=info")
                .try_init();
        }
        DLL_PROCESS_DETACH => {}
        _ => {}
    }

    1 // TRUE
}

/// Get a class factory for the requested CLSID.
#[no_mangle]
pub extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        let clsid_ref = &*clsid;
        let iid_ref = &*iid;

        // Determine which class to create based on CLSID
        let factory: Option<IClassFactory> = if *clsid_ref == overlay::CLSID_CLOUD {
            Some(ClassFactory::new::<overlay::CloudOverlay>())
        } else if *clsid_ref == overlay::CLSID_SYNCING {
            Some(ClassFactory::new::<overlay::SyncingOverlay>())
        } else if *clsid_ref == overlay::CLSID_SYNCED {
            Some(ClassFactory::new::<overlay::SyncedOverlay>())
        } else if *clsid_ref == overlay::CLSID_ERROR {
            Some(ClassFactory::new::<overlay::ErrorOverlay>())
        } else if *clsid_ref == overlay::CLSID_UPLOADING {
            Some(ClassFactory::new::<overlay::UploadingOverlay>())
        } else if *clsid_ref == CLSID_CONTEXT_MENU {
            Some(ClassFactory::new::<context_menu::GDriverContextMenu>())
        } else {
            None
        };

        match factory {
            Some(f) => {
                let result = f.query(iid_ref, ppv);
                f.release();
                result
            }
            None => CLASS_E_CLASSNOTAVAILABLE,
        }
    }
}

/// Check if the DLL can be unloaded (no outstanding references).
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    // In a production implementation, we'd track reference counts.
    // For now, always allow unload.
    S_OK
}

/// Register the shell extension entries in the Windows registry.
#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    // This is typically handled by the installer (NSIS).
    // The registration script is in register.reg.
    S_OK
}

/// Unregister the shell extension entries from the Windows registry.
#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    // This is typically handled by the uninstaller.
    S_OK
}

// ─── CLSID for the context menu handler ────────────────────────────────────

const CLSID_CONTEXT_MENU: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556600);

// ─── Class Factory ──────────────────────────────────────────────────────────

struct ClassFactory {
    create_fn: fn() -> IUnknown,
    ref_count: u32,
}

impl ClassFactory {
    fn new<T>() -> IClassFactory
    where
        T: Default + 'static,
        IClassFactory: From<T>,
    {
        let factory = Box::new(Self {
            create_fn || {
                let obj = T::default();
                IClassFactory::from(obj)
            },
            ref_count: 1,
        });
        IClassFactory::from(Box::into_raw(factory))
    }
}

// ─── Helper: open URL in default browser ────────────────────────────────────

mod open {
    use std::process::Command;

    pub fn that(url: &str) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(target_os = "windows")]
        {
            Command::new("cmd")
                .args(["/c", "start", "", url])
                .spawn()?;
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = url;
        }

        Ok(())
    }
}
