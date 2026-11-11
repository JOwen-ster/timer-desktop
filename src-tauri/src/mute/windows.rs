use super::sessions::Sessions;
use ::windows::{
    Win32::{
        Foundation::S_OK,
        Media::Audio::{
            AUDCLNT_E_DEVICE_INVALIDATED, AudioSessionStateExpired, DEVICE_STATE_ACTIVE,
            IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator,
            ISimpleAudioVolume, MMDeviceEnumerator, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
    },
    core::{Interface, Result},
};
use std::ptr;

struct ComApartment;

impl ComApartment {
    fn new() -> Result<Self> {
        // CPAL opens the alarm output on this worker and initializes it as an STA.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub(crate) struct Mute {
    sessions: Sessions<String, ISimpleAudioVolume>,
    devices: IMMDeviceEnumerator,
    _com: ComApartment,
}

impl Mute {
    pub(crate) fn start() -> std::result::Result<Self, String> {
        let open = || -> Result<_> {
            let com = ComApartment::new()?;
            let devices = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
            Ok(Self {
                sessions: Sessions::new(),
                devices,
                _com: com,
            })
        };
        open().map_err(|error| format!("Could not mute other apps: {error}"))
    }

    pub(crate) fn refresh(&mut self) -> std::result::Result<(), String> {
        let mut scan = || -> Result<()> {
            // Fresh enumerators pick up new sessions and output devices during the alarm.
            let devices = unsafe {
                self.devices
                    .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?
            };
            let mut result = Ok(());
            for index in 0..unsafe { devices.GetCount()? } {
                let scanned = unsafe { devices.Item(index) }
                    .and_then(|device| mute_device(&mut self.sessions, &device));
                result = result.and(ignore_disconnected(scanned));
            }
            result
        };
        scan().map_err(|error| format!("Could not mute other apps: {error}"))
    }

    pub(crate) fn restore(self) -> std::result::Result<(), String> {
        self.sessions
            .restore(|_, volume| ignore_disconnected(unsafe { volume.SetMute(false, ptr::null()) }))
            .map_err(|error| error.to_string())
    }
}

fn mute_device(
    sessions: &mut Sessions<String, ISimpleAudioVolume>,
    device: &IMMDevice,
) -> Result<()> {
    // The device and its sessions can disappear between enumeration and inspection.
    unsafe {
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let listed = manager.GetSessionEnumerator()?;
        let mut result = Ok(());
        for index in 0..listed.GetCount()? {
            let scanned = (|| {
                let session: IAudioSessionControl2 = listed.GetSession(index)?.cast()?;
                let system_sounds = session.IsSystemSoundsSession();
                system_sounds.ok()?;
                if system_sounds == S_OK
                    || session.GetState()? == AudioSessionStateExpired
                    || session.GetProcessId()? == std::process::id()
                {
                    return Ok(());
                }

                let id = session.GetSessionInstanceIdentifier()?;
                let identifier = id.to_string();
                CoTaskMemFree(Some(id.as_ptr().cast()));
                let identifier = identifier?;
                sessions.observe(identifier, || {
                    let volume: ISimpleAudioVolume = session.cast()?;
                    if volume.GetMute()?.as_bool() {
                        return Ok(None);
                    }
                    volume.SetMute(true, ptr::null())?;
                    Ok(Some(volume))
                })
            })();
            result = result.and(ignore_disconnected(scanned));
        }
        result
    }
}

fn ignore_disconnected(result: Result<()>) -> Result<()> {
    match result {
        Err(error) if error.code() == AUDCLNT_E_DEVICE_INVALIDATED => Ok(()),
        result => result,
    }
}
