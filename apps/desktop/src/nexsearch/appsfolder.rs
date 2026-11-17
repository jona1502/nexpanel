//! `shell:AppsFolder` source: enumerates the virtual Apps folder (Store/UWP apps and classic
//! apps) through `IShellItem`/`IEnumShellItems`. Launching goes through
//! `ShellExecuteW` with the path `shell:AppsFolder\<AppUserModelID>`.

use std::path::PathBuf;

use super::launcher::{AppEntry, AppSource};

/// Builds the launch path for an AppUserModelID.
pub fn apps_folder_path(app_id: &str) -> PathBuf {
    PathBuf::from(format!("shell:AppsFolder\\{app_id}"))
}

/// Enumerates `FOLDERID_AppsFolder`. COM is initialised on the calling (index) thread.
pub struct AppsFolderSource;

#[cfg(windows)]
impl AppSource for AppsFolderSource {
    fn scan(&self) -> Vec<AppEntry> {
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
        };
        // SAFETY: paired with CoUninitialize below on the same thread, only if init succeeded.
        let com =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let out = enumerate().unwrap_or_else(|error| {
            log::warn!("AppsFolder-Aufzählung fehlgeschlagen: {error}");
            Vec::new()
        });
        if com.is_ok() {
            // SAFETY: balanced with the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        out
    }
}

#[cfg(not(windows))]
impl AppSource for AppsFolderSource {
    fn scan(&self) -> Vec<AppEntry> {
        Vec::new()
    }
}

#[cfg(windows)]
fn enumerate() -> windows::core::Result<Vec<AppEntry>> {
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
        SHGetKnownFolderItem, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    };
    use windows::Win32::{Foundation::HANDLE, System::Com::CoTaskMemFree};

    fn display(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
        // SAFETY: the returned PWSTR is CoTaskMem allocated; copied, then freed.
        unsafe {
            let p = item.GetDisplayName(kind).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        }
    }

    // SAFETY: plain COM calls on an initialised thread; all interfaces are released on drop.
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(
            &FOLDERID_AppsFolder,
            KF_FLAG_DEFAULT,
            Some(HANDLE::default()),
        )?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut out = Vec::new();
        loop {
            let mut buf: [Option<IShellItem>; 1] = [None];
            let mut fetched = 0u32;
            items.Next(&mut buf, Some(&mut fetched))?;
            if fetched == 0 {
                break;
            }
            let Some(item) = buf[0].take() else { break };
            let (Some(name), Some(id)) = (
                display(&item, SIGDN_NORMALDISPLAY),
                display(&item, SIGDN_PARENTRELATIVEPARSING),
            ) else {
                continue;
            };
            out.push(AppEntry {
                name,
                path: apps_folder_path(&id),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_format() {
        assert_eq!(
            apps_folder_path("A.B_x!App"),
            PathBuf::from("shell:AppsFolder\\A.B_x!App")
        );
    }
}
