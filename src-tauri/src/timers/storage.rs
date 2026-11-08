use super::model::{MAX_MS, Timer, TimerCheckpoint};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, path::PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedTimer {
    id: u64,
    name: String,
    duration_ms: u64,
    remaining_ms: u64,
    ring_duration_ms: u64,
    last_adjustment_ms: u64,
    deadline_ms: Option<u64>,
}

impl From<TimerCheckpoint> for SavedTimer {
    fn from(timer: TimerCheckpoint) -> Self {
        Self {
            id: timer.id,
            name: timer.name,
            duration_ms: timer.duration_ms,
            remaining_ms: timer.remaining_ms,
            ring_duration_ms: timer.ring_duration_ms,
            last_adjustment_ms: timer.last_adjustment_ms,
            deadline_ms: timer.deadline_ms,
        }
    }
}

impl From<SavedTimer> for TimerCheckpoint {
    fn from(timer: SavedTimer) -> Self {
        Self {
            id: timer.id,
            name: timer.name,
            duration_ms: timer.duration_ms,
            remaining_ms: timer.remaining_ms,
            ring_duration_ms: timer.ring_duration_ms,
            last_adjustment_ms: timer.last_adjustment_ms,
            deadline_ms: timer.deadline_ms,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedFile {
    version: u8,
    next_id: u64,
    timers: Vec<SavedTimer>,
}

impl SavedFile {
    fn validate_metadata(&self) -> Result<(), String> {
        if self.version != 1 || self.next_id == 0 || self.next_id >= MAX_MS {
            return Err("Unsupported or invalid saved timer data.".into());
        }
        let mut ids = HashSet::new();
        for timer in &self.timers {
            if timer.id == 0 || timer.id >= self.next_id || !ids.insert(timer.id) {
                return Err("Invalid saved timer data.".into());
            }
        }
        Ok(())
    }

    fn restore(self, now: u64) -> Result<(Vec<Timer>, u64), String> {
        self.validate_metadata()?;
        let timers = self.timers.into_iter()
            .map(|timer| Timer::restore_paused(timer.into(), now))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((timers, self.next_id))
    }
}

pub struct Store {
    path: PathBuf,
    load_error: Option<String>,
}

impl Store {
    pub fn open(path: PathBuf, now: u64) -> (Self, Vec<Timer>, u64) {
        let loaded = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<SavedFile>(&bytes)
                .map_err(|error| error.to_string())
                .and_then(|saved| saved.restore(now)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((Vec::new(), 1)),
            Err(error) => Err(error.to_string()),
        };
        match loaded {
            Ok((timers, next_id)) => (Self { path, load_error: None }, timers, next_id),
            Err(error) => {
                let error = format!("Could not load {}: {error} The file has been left untouched. Fix it and restart the app.", path.display());
                (Self { path, load_error: Some(error) }, Vec::new(), 1)
            }
        }
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn save(&self, timers: &[Timer], next_id: u64, now: u64) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(error.clone());
        }
        let saved = SavedFile {
            version: 1,
            next_id,
            timers: timers.iter().map(|timer| timer.checkpoint(now).into()).collect(),
        };
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            saved.validate_metadata()?;
            let bytes = serde_json::to_vec_pretty(&saved)?;
            fs::create_dir_all(self.path.parent().ok_or("Invalid timer storage path")?)?;
            let temporary = self.path.with_extension("json.tmp");
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(temporary, &self.path)?;
            Ok(())
        })();
        result.map_err(|error| format!("Could not save timers: {error}. Changes are only in memory; retry an action before quitting."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timers::model::{TimerAction, TimerStatus};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("timer-store-{}-{}", std::process::id(), NEXT_PATH.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("timers.json")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn missing_storage_starts_empty_and_preserves_order_and_adjustments() {
        let directory = TestDirectory::new();
        let (store, timers, next_id) = Store::open(directory.file(), 1000);
        assert!(store.load_error().is_none());
        assert!(timers.is_empty());
        assert_eq!(next_id, 1);
        let mut first = Timer::new(1, "First".into(), 3601, 1000).unwrap();
        first.apply(TimerAction::Toggle, 2000).unwrap();
        first.apply(TimerAction::Adjust { seconds: 150, subtract: false }, 3000).unwrap();
        let expected = first.view(3000);
        let mut second = Timer::new(2, "Second".into(), 60, 1000).unwrap();
        second.freeze(3000);
        store.save(&[first, second], 3, 3000).unwrap();
        let (restored_store, restored, next_id) = Store::open(directory.file(), 9000);
        assert!(restored_store.load_error().is_none());
        assert_eq!(next_id, 3);
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].view(9000), expected);
        assert_eq!(restored[1].view(9000).name, "Second");
        assert_eq!(restored[1].view(9000).status, TimerStatus::Paused);
    }

    #[test]
    fn version_one_files_restore_deadlines_paused_without_replaying_alarms() {
        let directory = TestDirectory::new();
        fs::write(directory.file(), br#"{
            "version": 1, "nextId": 3, "timers": [
                {"id": 1, "name": "Live", "durationMs": 5000, "remainingMs": 5000, "ringDurationMs": 5000, "lastAdjustmentMs": 60000, "deadlineMs": 6000},
                {"id": 2, "name": "Expired", "durationMs": 5000, "remainingMs": 0, "ringDurationMs": 5000, "lastAdjustmentMs": 60000, "deadlineMs": null}
            ]
        }"#).unwrap();
        let (store, restored, next_id) = Store::open(directory.file(), 3000);
        assert!(store.load_error().is_none());
        assert_eq!(next_id, 3);
        assert_eq!(restored[0].view(9000).remaining_ms, 3000);
        assert_eq!(restored[0].view(9000).status, TimerStatus::Paused);
        assert_eq!(restored[1].view(9000).status, TimerStatus::Completed);
        store.save(&restored, next_id, 9000).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(directory.file()).unwrap()).unwrap();
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["timers"][0]["deadlineMs"], serde_json::Value::Null);
        assert!(saved["timers"][0].get("alarming").is_none());
    }

    #[test]
    fn clean_quit_snapshots_are_exact_and_alarms_are_transient() {
        let directory = TestDirectory::new();
        let (store, _, _) = Store::open(directory.file(), 1000);
        let mut running = Timer::new(1, "Running".into(), 60, 1000).unwrap();
        let mut ringing = Timer::new(2, "Ringing".into(), 1, 1000).unwrap();
        ringing.tick(3000);
        running.freeze(3000);
        ringing.freeze(3000);
        let expected: Vec<_> = [&running, &ringing].into_iter().map(|timer| timer.view(3000)).collect();
        store.save(&[running, ringing], 3, 3000).unwrap();
        let (_, restored, _) = Store::open(directory.file(), 100_000);
        assert_eq!(restored.iter().map(|timer| timer.view(100_000)).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn corrupt_and_unsupported_saves_are_never_overwritten() {
        for contents in [
            "broken json",
            r#"{"version":2,"nextId":1,"timers":[]}"#,
            r#"{"version":1,"nextId":0,"timers":[]}"#,
            r#"{"version":1,"nextId":2,"timers":[{"id":1,"name":"Bad","durationMs":0,"remainingMs":0,"ringDurationMs":1,"lastAdjustmentMs":1000,"deadlineMs":null}]}"#,
        ] {
            let directory = TestDirectory::new();
            fs::write(directory.file(), contents).unwrap();
            let (store, restored, _) = Store::open(directory.file(), 1000);
            assert!(store.load_error().is_some());
            assert!(restored.is_empty());
            let timer = Timer::new(1, "Tea".into(), 5, 1000).unwrap();
            assert!(store.save(&[timer], 2, 1000).is_err());
            assert_eq!(fs::read_to_string(directory.file()).unwrap(), contents);
        }
    }

    #[test]
    fn duplicate_ids_are_rejected_and_the_original_file_is_preserved() {
        let directory = TestDirectory::new();
        let (store, _, _) = Store::open(directory.file(), 1000);
        let timer = Timer::new(1, "Tea".into(), 5, 1000).unwrap();
        store.save(&[timer.clone()], 2, 1000).unwrap();
        let original = fs::read(directory.file()).unwrap();
        assert!(store.save(&[timer.clone(), timer], 2, 1000).is_err());
        assert_eq!(fs::read(directory.file()).unwrap(), original);
    }

    #[test]
    fn save_failure_can_be_retried_without_changing_load_protection() {
        let directory = TestDirectory::new();
        let (store, _, _) = Store::open(directory.file(), 1000);
        let timer = Timer::new(1, "Tea".into(), 5, 1000).unwrap();
        fs::create_dir(directory.file()).unwrap();
        assert!(store.save(&[timer.clone()], 2, 1000).is_err());
        assert!(store.load_error().is_none());
        fs::remove_dir(directory.file()).unwrap();
        store.save(&[timer], 2, 1000).unwrap();
        let (restored_store, restored, _) = Store::open(directory.file(), 2000);
        assert!(restored_store.load_error().is_none());
        assert_eq!(restored[0].view(2000).remaining_ms, 4000);
    }
}
