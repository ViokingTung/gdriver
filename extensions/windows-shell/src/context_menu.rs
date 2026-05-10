// ─── Context Menu Implementation ──────────────────────────────────────────
//
// Implements `IContextMenu` and `IShellExtInit` to add gDriver-specific
// right-click menu items in Windows Explorer.
//
// Menu items:
//   - Available offline: Mark file for offline access
//   - Online only: Remove offline access, free local space
//   - Copy Drive link: Copy Google Drive share link to clipboard
//   - View in Drive web: Open file in Google Drive web interface
//   - Share: Open sharing dialog in browser

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::{Com::*, DataExchange::*, Memory::*},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

use crate::{ipc, open};

// ─── Menu item IDs ─────────────────────────────────────────────────────────

const CMD_AVAILABLE_OFFLINE: u32 = 0;
const CMD_ONLINE_ONLY: u32 = 1;
const CMD_COPY_LINK: u32 = 2;
const CMD_VIEW_IN_DRIVE: u32 = 3;
const CMD_SHARE: u32 = 4;

// ─── Context Menu Implementation ───────────────────────────────────────────

#[implement(IContextMenu, IShellExtInit)]
pub struct GDriverContextMenu {
    /// The path of the file/folder that was right-clicked.
    file_path: String,
    /// Whether the file is inside the gDriver mount directory.
    is_gdriver_path: bool,
}

impl GDriverContextMenu {
    pub fn new() -> Self {
        Self {
            file_path: String::new(),
            is_gdriver_path: false,
        }
    }

    /// Check if a path is inside the gDriver mount directory.
    fn is_gdriver_path(path: &str) -> bool {
        // Check common mount points
        let gdriver_paths = [r"G:\", r"Google Drive\", "GoogleDrive"];

        let path_lower = path.to_lowercase();
        gdriver_paths
            .iter()
            .any(|prefix| path_lower.starts_with(&prefix.to_lowercase()))
    }

    /// Get the relative path within the gDriver mount.
    fn relative_path(&self) -> String {
        // Strip the mount point prefix to get the Drive-relative path
        let mount_points = [r"G:\", "Google Drive\\"];
        for prefix in &mount_points {
            if self
                .file_path
                .to_lowercase()
                .starts_with(&prefix.to_lowercase())
            {
                return self.file_path[prefix.len()..].to_string();
            }
        }
        self.file_path.clone()
    }
}

impl IShellExtInit_Impl for GDriverContextMenu {
    fn Initialize(
        &self,
        _pidl_folder: Option<*const ITEMIDLIST>,
        data_object: Option<&IDataObject>,
        _hkey: *const HKEY__,
    ) -> Result<()> {
        if let Some(data_obj) = data_object {
            // Get the file path from the data object
            let format_etc = FORMATETC {
                cfFormat: CF_HDROP.0 as u16,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0 as u32,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            };

            let mut medium = STGMEDIUM::default();
            unsafe {
                data_obj.GetData(&format_etc, &mut medium)?;
            }

            if medium.tymed == TYMED_HGLOBAL.0 as u32 {
                unsafe {
                    let hdrop = HDROP(medium.hGlobal);
                    let file_count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);

                    if file_count > 0 {
                        // Get the first selected file
                        let len = DragQueryFileW(hdrop, 0, None) as usize;
                        let mut buffer = vec![0u16; len + 1];
                        DragQueryFileW(hdrop, 0, Some(&mut buffer));
                        let path = String::from_utf16_lossy(&buffer[..len]);

                        self.file_path = path;
                        self.is_gdriver_path = Self::is_gdriver_path(&self.file_path);
                    }

                    ReleaseStgMedium(&raw const medium);
                }
            }
        }

        Ok(())
    }
}

impl IContextMenu_Impl for GDriverContextMenu {
    fn QueryContextMenu(
        &self,
        menu: HMENU,
        index: u32,
        cmd_first: u32,
        _cmd_last: u32,
        _flags: u32,
    ) -> Result<()> {
        if !self.is_gdriver_path {
            return Ok(());
        }

        unsafe {
            // Add a separator
            InsertMenuW(menu, index, MF_BYPOSITION | MF_SEPARATOR, 0, None);

            // Available offline
            InsertMenuW(
                menu,
                index + 1,
                MF_BYPOSITION | MF_STRING,
                (cmd_first + CMD_AVAILABLE_OFFLINE) as usize,
                w!("Available offline"),
            );

            // Online only
            InsertMenuW(
                menu,
                index + 2,
                MF_BYPOSITION | MF_STRING,
                (cmd_first + CMD_ONLINE_ONLY) as usize,
                w!("Online only"),
            );

            // Separator
            InsertMenuW(menu, index + 3, MF_BYPOSITION | MF_SEPARATOR, 0, None);

            // Copy Drive link
            InsertMenuW(
                menu,
                index + 4,
                MF_BYPOSITION | MF_STRING,
                (cmd_first + CMD_COPY_LINK) as usize,
                w!("Copy Drive link"),
            );

            // View in Drive web
            InsertMenuW(
                menu,
                index + 5,
                MF_BYPOSITION | MF_STRING,
                (cmd_first + CMD_VIEW_IN_DRIVE) as usize,
                w!("View in Drive web"),
            );

            // Share
            InsertMenuW(
                menu,
                index + 6,
                MF_BYPOSITION | MF_STRING,
                (cmd_first + CMD_SHARE) as usize,
                w!("Share"),
            );
        }

        Ok(())
    }

    fn InvokeCommand(&self, command: *const CMINVOKECOMMANDINFO) -> Result<()> {
        // Check if the command is a verb string or a command offset
        let cmd = unsafe {
            if (*command).lpVerb.0 as u32 <= CMD_SHARE {
                (*command).lpVerb.0 as u32
            } else {
                // Try to parse as a verb string
                let verb = (*command).lpVerb.to_string().unwrap_or_default();
                match verb.as_str() {
                    "available_offline" => CMD_AVAILABLE_OFFLINE,
                    "online_only" => CMD_ONLINE_ONLY,
                    "copy_link" => CMD_COPY_LINK,
                    "view_in_drive" => CMD_VIEW_IN_DRIVE,
                    "share" => CMD_SHARE,
                    _ => return Ok(()),
                }
            }
        };

        let relative = self.relative_path();

        match cmd {
            CMD_AVAILABLE_OFFLINE => {
                let _ = ipc::set_offline(&relative, true);
            }
            CMD_ONLINE_ONLY => {
                let _ = ipc::set_offline(&relative, false);
            }
            CMD_COPY_LINK => {
                if let Some(link) = ipc::get_share_link(&relative) {
                    // Copy to clipboard
                    unsafe {
                        if OpenClipboard(HWND::default()).is_ok() {
                            EmptyClipboard();

                            let wide: Vec<u16> =
                                link.encode_utf16().chain(std::iter::once(0)).collect();
                            let size = wide.len() * std::mem::size_of::<u16>();
                            let hmem =
                                GlobalAlloc(GLOBAL_ALLOC_FLAGS(GMEM_MOVEABLE.0 as u32), size);
                            if let Ok(ptr) = hmem {
                                let dst = GlobalLock(ptr) as *mut u16;
                                if !dst.is_null() {
                                    std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                                    GlobalUnlock(ptr);
                                    SetClipboardData(
                                        CF_UNICODETEXT.0 as u32,
                                        Some(HANDLE(ptr as isize)),
                                    );
                                }
                            }
                            CloseClipboard();
                        }
                    }
                }
            }
            CMD_VIEW_IN_DRIVE => {
                // Open in browser
                let url = format!("https://drive.google.com/file/d/{}", relative);
                let _ = open::that(&url);
            }
            CMD_SHARE => {
                // Open sharing dialog
                let url = format!(
                    "https://drive.google.com/file/d/{}/edit?usp=sharing",
                    relative
                );
                let _ = open::that(&url);
            }
            _ => {}
        }

        Ok(())
    }

    fn GetCommandString(
        &self,
        command: usize,
        flags: u32,
        _reserved: *const u32,
        name: PSTR,
        _cch_max: u32,
    ) -> Result<()> {
        let text = match command as u32 {
            CMD_AVAILABLE_OFFLINE => "Mark this file for offline access",
            CMD_ONLINE_ONLY => "Remove offline access and free local space",
            CMD_COPY_LINK => "Copy Google Drive share link to clipboard",
            CMD_VIEW_IN_DRIVE => "Open this file in Google Drive web",
            CMD_SHARE => "Share this file via Google Drive",
            _ => return Ok(()),
        };

        let wide: Vec<u16> = text.encode_utf16().collect();

        match flags {
            // GCS_HELPTEXTW
            6 => {
                let dst = unsafe {
                    std::slice::from_raw_parts_mut(name.0 as *mut u16, _cch_max as usize)
                };
                let len = wide.len().min(dst.len() - 1);
                dst[..len].copy_from_slice(&wide[..len]);
                dst[len] = 0;
            }
            // GCS_VERBW
            4 => {
                let verb = match command as u32 {
                    CMD_AVAILABLE_OFFLINE => "available_offline",
                    CMD_ONLINE_ONLY => "online_only",
                    CMD_COPY_LINK => "copy_link",
                    CMD_VIEW_IN_DRIVE => "view_in_drive",
                    CMD_SHARE => "share",
                    _ => return Ok(()),
                };
                let wide_verb: Vec<u16> = verb.encode_utf16().collect();
                let dst = unsafe {
                    std::slice::from_raw_parts_mut(name.0 as *mut u16, _cch_max as usize)
                };
                let len = wide_verb.len().min(dst.len() - 1);
                dst[..len].copy_from_slice(&wide_verb[..len]);
                dst[len] = 0;
            }
            _ => {}
        }

        Ok(())
    }
}
