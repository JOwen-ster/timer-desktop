use objc2::{AnyThread, rc::autoreleasepool};
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBufferList, AudioTimeStamp};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSString, NSUUID};
use std::{
    ffi::c_void,
    mem::size_of,
    ptr::{self, NonNull},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

pub(crate) struct Mute {
    pending: Receiver<Result<Tap, String>>,
    active: Option<Tap>,
    error: Option<String>,
}

impl Mute {
    pub(crate) fn start() -> Result<Self, String> {
        let (ready, pending) = mpsc::channel();
        // The system permission prompt can block tap setup while the alarm still needs to stop.
        thread::Builder::new()
            .name("prepare-audio-tap".into())
            .spawn(move || {
                let _ = ready.send(autoreleasepool(|_| Tap::prepare()));
            })
            .map_err(|error| format!("Could not prepare other-app muting: {error}"))?;
        Ok(Self {
            pending,
            active: None,
            error: None,
        })
    }

    pub(crate) fn refresh(&mut self) -> Result<(), String> {
        if self.active.is_none() && self.error.is_none() {
            let prepared = match self.pending.try_recv() {
                Ok(prepared) => prepared,
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    Err("Audio tap setup stopped unexpectedly".into())
                }
            };
            match prepared.and_then(Tap::activate) {
                Ok(tap) => self.active = Some(tap),
                Err(error) => self.error = Some(format!("Could not mute other apps: {error}")),
            }
        }
        self.error.clone().map_or(Ok(()), Err)
    }

    pub(crate) fn restore(self) -> Result<(), String> {
        if let Some(mut tap) = self.active {
            tap.close()?;
        }
        Ok(())
    }
}

struct Tap {
    tap: AudioObjectID,
    device: AudioObjectID,
    callback: AudioDeviceIOProcID,
    running: bool,
}

impl Tap {
    fn prepare() -> Result<Self, String> {
        let mut mute = Self {
            tap: kAudioObjectUnknown,
            device: kAudioObjectUnknown,
            callback: None,
            running: false,
        };
        let excluded = NSArray::from_retained_slice(&[NSNumber::new_u32(process_audio_id()?)]);

        // The private tap sends audio only to the discard callback.
        unsafe {
            let description = CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &excluded,
            );
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::MutedWhenTapped);
            check(
                AudioHardwareCreateProcessTap(Some(&description), &mut mute.tap),
                "create the audio tap",
            )?;

            let tap_uid = description.UUID().UUIDString();
            let yes = NSNumber::new_bool(true);
            let no = NSNumber::new_bool(false);
            let tap_values: [&NSObject; 2] = [&tap_uid, &yes];
            let tap = NSDictionary::from_slices(
                &[
                    &*key(kAudioSubTapUIDKey),
                    &*key(kAudioSubTapDriftCompensationKey),
                ],
                &tap_values,
            );
            let taps = NSArray::from_retained_slice(&[tap]);
            let name = NSString::from_str("Timers alarm");
            let device_uid = NSUUID::new().UUIDString();
            let keys = [
                kAudioAggregateDeviceNameKey,
                kAudioAggregateDeviceUIDKey,
                kAudioAggregateDeviceIsPrivateKey,
                kAudioAggregateDeviceTapAutoStartKey,
                kAudioAggregateDeviceTapListKey,
            ]
            .map(key);
            let values: [&NSObject; 5] = [&name, &device_uid, &yes, &no, &taps];
            let device = NSDictionary::from_slices(&keys.each_ref().map(|key| &**key), &values);
            let device: &CFDictionary<NSString, NSObject> = (*device).as_ref();
            check(
                AudioHardwareCreateAggregateDevice(device.as_opaque(), (&mut mute.device).into()),
                "create the private audio device",
            )?;
            check(
                AudioDeviceCreateIOProcID(
                    mute.device,
                    Some(discard_audio),
                    ptr::null_mut(),
                    (&mut mute.callback).into(),
                ),
                "attach the audio tap. Allow system audio recording for Timers in System Settings",
            )?;

            // Core Audio can accept a callback even if the user denied system audio access.
            let mut address = AudioObjectPropertyAddress {
                mSelector: kAudioTapPropertyDescription,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: kAudioObjectPropertyElementMain,
            };
            let description_ptr = ptr::from_ref(&*description);
            check(
                AudioObjectSetPropertyData(
                    mute.tap,
                    (&mut address).into(),
                    0,
                    ptr::null(),
                    size_of::<*const CATapDescription>() as u32,
                    NonNull::from(&description_ptr).cast(),
                ),
                "configure the audio tap. Allow system audio recording for Timers in System Settings",
            )?;
        }
        Ok(mute)
    }

    fn activate(mut self) -> Result<Self, String> {
        check(
            unsafe { AudioDeviceStart(self.device, self.callback) },
            "start the audio tap",
        )?;
        self.running = true;
        Ok(self)
    }

    fn close(&mut self) -> Result<(), String> {
        let mut result = Ok(());
        // Stop reading first so the system can resume playback even if device cleanup fails.
        unsafe {
            if self.running {
                result = check(
                    AudioDeviceStop(self.device, self.callback),
                    "stop the audio tap",
                );
                self.running = false;
            }
            if let Some(callback) = self.callback.take() {
                result = result.and(check(
                    AudioDeviceDestroyIOProcID(self.device, Some(callback)),
                    "detach the audio tap",
                ));
            }
            if self.device != kAudioObjectUnknown {
                let status = AudioHardwareDestroyAggregateDevice(self.device);
                if status == 0 {
                    self.device = kAudioObjectUnknown;
                }
                result = result.and(check(status, "remove the private audio device"));
            }
            if self.tap != kAudioObjectUnknown {
                let status = AudioHardwareDestroyProcessTap(self.tap);
                if status == 0 {
                    self.tap = kAudioObjectUnknown;
                }
                result = result.and(check(status, "remove the audio tap"));
            }
        }
        result
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn process_audio_id() -> Result<AudioObjectID, String> {
    let pid = std::process::id() as i32;
    let mut id = kAudioObjectUnknown;
    let mut address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size = size_of::<AudioObjectID>() as u32;
    unsafe {
        check(
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject as AudioObjectID,
                (&mut address).into(),
                size_of::<i32>() as u32,
                ptr::from_ref(&pid).cast(),
                (&mut size).into(),
                NonNull::from(&mut id).cast(),
            ),
            "identify the timer's audio process",
        )?;
    }
    if id == kAudioObjectUnknown {
        return Err("The timer's audio process is unavailable".into());
    }
    Ok(id)
}

fn key(value: &std::ffi::CStr) -> objc2::rc::Retained<NSString> {
    NSString::from_str(value.to_str().expect("Core Audio keys are ASCII"))
}

fn check(status: i32, operation: &str) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "Could not {operation} (Core Audio status {status})"
        ))
    }
}

unsafe extern "C-unwind" fn discard_audio(
    _: AudioObjectID,
    _: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    _: NonNull<AudioBufferList>,
    _: NonNull<AudioTimeStamp>,
    _: *mut c_void,
) -> i32 {
    0
}
