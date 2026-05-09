// ─── Shell Icon Overlay Implementation ────────────────────────────────────
//
// Implements `IShellIconOverlayIdentifier` to show sync status icons on
// files managed by gDriver in Windows Explorer.
//
// Five overlay states:
//   - cloud_only: Gray cloud (file is only in the cloud)
//   - syncing: Blue spinning arrow (file is being synced)
//   - synced/cached/offline: Green checkmark (file is synced)
//   - error: Red X (sync error)
//   - uploading: Blue up arrow (file is being uploaded)

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::*;
use crate::ipc;

// ─── Overlay CLSIDs ────────────────────────────────────────────────────────
//
// Each overlay needs its own CLSID. These must match the registry entries
// created during installation.

/// CLSID for the "cloud only" overlay (gray cloud).
pub const CLSID_CLOUD: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556601);

/// CLSID for the "syncing" overlay (blue spinning arrow).
pub const CLSID_SYNCING: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556602);

/// CLSID for the "synced" overlay (green checkmark).
pub const CLSID_SYNCED: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556603);

/// CLSID for the "error" overlay (red X).
pub const CLSID_ERROR: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556604);

/// CLSID for the "uploading" overlay (blue up arrow).
pub const CLSID_UPLOADING: GUID = GUID::from_u128(0xA1B2C3D4_1111_2222_3333_444455556605);

// ─── Overlay priority constants ────────────────────────────────────────────
//
// Lower values have higher priority. When multiple overlays could apply,
// Windows uses the one with the lowest priority value.

const PRIORITY_SYNCING: i32 = 0;
const PRIORITY_UPLOADING: i32 = 0;
const PRIORITY_ERROR: i32 = 1;
const PRIORITY_CLOUD: i32 = 2;
const PRIORITY_SYNCED: i32 = 3;

// ─── Icon Overlay Implementations ──────────────────────────────────────────

/// Macro to implement IShellIconOverlayIdentifier for each overlay state.
macro_rules! impl_overlay {
    ($name:ident, $state:expr, $priority:expr) => {
        #[implement(IShellIconOverlayIdentifier)]
        pub struct $name;

        impl IShellIconOverlayIdentifier_Impl for $name {
            fn IsMemberOf(&self, path: &PCWSTR, _attributes: u32) -> Result<()> {
                // Convert the wide string path to a Rust string
                let path_str = unsafe { path.to_string().unwrap_or_default() };

                // Query the daemon for the file's sync state
                if let Some(state) = ipc::get_sync_state(&path_str) {
                    let sync_state = state
                        .get("state")
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown");

                    // Check if this overlay should apply
                    let should_apply = match $state {
                        "cloud_only" => sync_state == "cloud_only",
                        "syncing" => {
                            sync_state == "syncing"
                                || sync_state == "downloading"
                                || sync_state == "uploading"
                        }
                        "synced" => {
                            sync_state == "synced"
                                || sync_state == "cached"
                                || sync_state == "offline"
                        }
                        "error" => sync_state == "error",
                        "uploading" => sync_state == "uploading",
                        _ => false,
                    };

                    if should_apply {
                        return Ok(());
                    }
                }

                // Return error to indicate this overlay doesn't apply
                Err(Error::from_hresult(HRESULT::from_win32(1))) // S_FALSE
            }

            fn GetOverlayInfo(&self, icon_file_buffer: &mut [u16]) -> Result<i32> {
                // Return the path to the overlay icon file.
                // In production, these would be embedded resources or
                // installed alongside the DLL.
                let icon_path = match $state {
                    "cloud_only" => "gdriver_overlay_cloud.dll,-101",
                    "syncing" => "gdriver_overlay_syncing.dll,-102",
                    "synced" => "gdriver_overlay_synced.dll,-103",
                    "error" => "gdriver_overlay_error.dll,-104",
                    "uploading" => "gdriver_overlay_uploading.dll,-105",
                    _ => "",
                };

                // Copy the icon path to the buffer
                let wide: Vec<u16> = icon_path.encode_utf16().chain(std::iter::once(0)).collect();
                let len = wide.len().min(icon_file_buffer.len());
                icon_file_buffer[..len].copy_from_slice(&wide[..len]);

                Ok($priority)
            }

            fn GetPriority(&self) -> Result<i32> {
                Ok($priority)
            }
        }
    };
}

// Implement all five overlay states
impl_overlay!(CloudOverlay, "cloud_only", PRIORITY_CLOUD);
impl_overlay!(SyncingOverlay, "syncing", PRIORITY_SYNCING);
impl_overlay!(SyncedOverlay, "synced", PRIORITY_SYNCED);
impl_overlay!(ErrorOverlay, "error", PRIORITY_ERROR);
impl_overlay!(UploadingOverlay, "uploading", PRIORITY_UPLOADING);
