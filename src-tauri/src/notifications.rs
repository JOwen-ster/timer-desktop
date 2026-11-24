use std::{
    collections::HashMap,
    iter::once,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread,
};

use tauri::{AppHandle, Manager};
use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager},
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile, STGM_READ,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            Variant::{VARENUM, VT_LPWSTR},
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
        UI::{
            Shell::PropertiesSystem::IPropertyStore,
            Shell::{
                FOLDERID_Programs, IShellLinkW, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath, ShellLink,
            },
        },
    },
    core::{HSTRING, Interface, PCWSTR, PWSTR},
};

const APP_ID: &str = "com.jowen-ster.timer-desktop";
const GROUP: &str = "timer-completions";

static LIVE_TOASTS: OnceLock<Mutex<HashMap<u64, ToastNotification>>> = OnceLock::new();

pub fn initialize() -> windows::core::Result<()> {
    let thread = thread::Builder::new()
        .name("windows-notification-setup".into())
        .spawn(register_shortcut)
        .map_err(|_| windows::core::Error::from_thread())?;
    thread
        .join()
        .map_err(|_| windows::core::Error::from_thread())??;
    Ok(())
}

fn register_shortcut() -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }
    let result = unsafe { register_shortcut_com() };
    unsafe { CoUninitialize() };
    result
}

unsafe fn register_shortcut_com() -> windows::core::Result<()> {
    let folder = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KNOWN_FOLDER_FLAG(0), None)? };
    let folder_path = PathBuf::from(unsafe { folder.to_string()? });
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(folder.0.cast())) };

    let shortcut_path = folder_path.join("Timers.lnk");
    let exe_path = std::env::current_exe().map_err(|_| windows::core::Error::from_thread())?;
    let exe = wide(exe_path.as_os_str());
    let link_path = wide(shortcut_path.as_os_str());

    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
    let persist_file: IPersistFile = shell_link.cast()?;
    if shortcut_path.exists() {
        unsafe { persist_file.Load(PCWSTR(link_path.as_ptr()), STGM_READ)? };
    } else {
        unsafe { shell_link.SetPath(PCWSTR(exe.as_ptr()))? };
        unsafe { shell_link.SetIconLocation(PCWSTR(exe.as_ptr()), 0)? };
    }

    let property_store: IPropertyStore = shell_link.cast()?;
    let app_id_wide = wide(std::ffi::OsStr::new(APP_ID));
    let mut app_id_value = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VARENUM(VT_LPWSTR.0),
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    pwszVal: PWSTR(app_id_wide.as_ptr() as *mut _),
                },
            }),
        },
    };
    let app_user_model_id = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };
    unsafe {
        property_store.SetValue(&app_user_model_id, &app_id_value)?;
        property_store.Commit()?;
    }
    app_id_value.Anonymous = PROPVARIANT_0::default();

    unsafe { persist_file.Save(PCWSTR(link_path.as_ptr()), true)? };
    Ok(())
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(once(0)).collect()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn show(app: AppHandle, id: u64, name: String) {
    let result = unsafe {
        if let Err(error) = RoInitialize(RO_INIT_MULTITHREADED) {
            Err(error)
        } else {
            let result = show_toast(&app, id, &name);
            RoUninitialize();
            result
        }
    };
    if let Err(error) = result {
        eprintln!("Could not show timer notification: {error}");
    }
}

unsafe fn show_toast(app: &AppHandle, id: u64, name: &str) -> windows::core::Result<()> {
    let xml = format!(
        "<toast launch=\"open:{}\"><visual><binding template=\"ToastGeneric\"><text>Timer finished</text><text>{}</text></binding></visual><actions><action content=\"Open\" arguments=\"open:{}\" activationType=\"foreground\" /><action content=\"Snooze\" arguments=\"snooze:{}\" activationType=\"foreground\" /></actions></toast>",
        id,
        xml_escape(name),
        id,
        id
    );
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    let tag = HSTRING::from(format!("timer-{id}"));
    toast.SetTag(&tag)?;
    toast.SetGroup(&HSTRING::from(GROUP))?;

    let callback_app = app.clone();
    let handler: TypedEventHandler<ToastNotification, windows::core::IInspectable> =
        TypedEventHandler::<ToastNotification, windows::core::IInspectable>::new(
            move |_sender, args| {
                if let Some(args) = args.as_ref() {
                    if let Ok(args) = args.cast::<ToastActivatedEventArgs>() {
                        if let Ok(arguments) = args.Arguments() {
                            activate(&callback_app, arguments.to_string());
                        }
                    }
                }
                Ok(())
            },
        );
    toast.Activated(&handler)?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?;
    let mut live_toasts = LIVE_TOASTS.get_or_init(Default::default).lock().unwrap();
    live_toasts.insert(id, toast.clone());
    if let Err(error) = notifier.Show(&toast) {
        drop(live_toasts);
        dismiss(id);
        return Err(error);
    }
    Ok(())
}

fn activate(app: &AppHandle, arguments: String) {
    if let Some(id) = arguments
        .strip_prefix("snooze:")
        .and_then(|id| id.parse::<u64>().ok())
    {
        if let Some(engine) = app.try_state::<std::sync::Arc<crate::timers::Engine>>() {
            if let Err(error) = engine.acknowledge_alarm(id) {
                eprintln!("Could not snooze timer from notification: {error}");
            }
        }
        return;
    }
    if let Some(id) = arguments
        .strip_prefix("open:")
        .and_then(|id| id.parse::<u64>().ok())
    {
        dismiss(id);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn dismiss(id: u64) {
    if let Some(toasts) = LIVE_TOASTS.get() {
        toasts.lock().unwrap().remove(&id);
    }
    let _ = thread::Builder::new()
        .name("dismiss-timer-notification".into())
        .spawn(move || unsafe {
            if RoInitialize(RO_INIT_MULTITHREADED).is_err() {
                return;
            }
            if let Ok(history) = ToastNotificationManager::History() {
                let _ = history.RemoveGroupedTagWithId(
                    &HSTRING::from(format!("timer-{id}")),
                    &HSTRING::from(GROUP),
                    &HSTRING::from(APP_ID),
                );
            }
            RoUninitialize();
        });
}
