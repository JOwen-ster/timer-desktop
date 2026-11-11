#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::Mute;
#[cfg(target_os = "macos")]
pub(crate) use macos::Mute;
#[cfg(target_os = "windows")]
pub(crate) use windows::Mute;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) struct Mute;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl Mute {
    pub(crate) fn start() -> Result<Self, String> {
        Err("Other-app muting is unavailable on this platform.".into())
    }

    pub(crate) fn refresh(&mut self) -> Result<(), String> {
        Ok(())
    }

    pub(crate) fn restore(self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
pub(crate) mod sessions {
    use std::{
        collections::{HashMap, hash_map::Entry},
        hash::Hash,
    };

    pub(crate) struct Sessions<Id, Control>(HashMap<Id, Option<Control>>);

    impl<Id: Eq + Hash, Control> Sessions<Id, Control> {
        pub(crate) fn new() -> Self {
            Self(HashMap::new())
        }

        pub(crate) fn observe<E>(
            &mut self,
            id: Id,
            mute: impl FnOnce() -> Result<Option<Control>, E>,
        ) -> Result<(), E> {
            if let Entry::Vacant(entry) = self.0.entry(id) {
                entry.insert(mute()?);
            }
            Ok(())
        }

        pub(crate) fn restore<E>(
            self,
            mut restore: impl FnMut(Id, Control) -> Result<(), E>,
        ) -> Result<(), E> {
            let mut result = Ok(());
            for (id, control) in self.0 {
                if let Some(control) = control {
                    result = result.and(restore(id, control));
                }
            }
            result
        }
    }
}
