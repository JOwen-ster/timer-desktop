use crate::audio::AlarmAudio;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::Emitter;

// Milliseconds and IDs must remain exactly representable in JavaScript.
const MAX_MS: u64 = 9_007_199_254_740_991;
const TICK: Duration = Duration::from_millis(250);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn duration_ms(seconds: u64) -> Result<u64, String> {
    seconds
        .checked_mul(1000)
        .filter(|ms| *ms > 0 && *ms <= MAX_MS - now_ms())
        .ok_or_else(|| "Enter a positive duration within the supported range.".into())
}

fn valid_name(name: String) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        Err("Give the timer a name.".into())
    } else {
        Ok(name.into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Timer {
    id: u64,
    name: String,
    duration_ms: u64,
    remaining_ms: u64,
    ring_duration_ms: u64,
    last_adjustment_ms: u64,
    deadline_ms: Option<u64>,
    #[serde(skip)]
    alarming: bool,
}

impl Timer {
    fn remaining(&self, now: u64) -> u64 {
        self.deadline_ms
            .map_or(self.remaining_ms, |deadline| deadline.saturating_sub(now))
    }

    fn settle(&mut self, now: u64) -> bool {
        if self.deadline_ms.is_some() && self.remaining(now) == 0 {
            self.remaining_ms = 0;
            self.deadline_ms = None;
            self.alarming = true;
            true
        } else {
            false
        }
    }

    fn toggle(&mut self, now: u64) {
        if self.alarming {
            self.alarming = false;
        } else if self.deadline_ms.is_some() {
            self.remaining_ms = self.remaining(now);
            self.deadline_ms = None;
        } else {
            if self.remaining_ms == 0 {
                self.remaining_ms = self.duration_ms;
                self.ring_duration_ms = self.duration_ms;
            }
            self.deadline_ms = Some(now + self.remaining_ms);
        }
    }

    fn edit(&mut self, name: String, duration: u64, now: u64) {
        self.name = name;
        if duration != self.duration_ms {
            self.alarming = false;
            self.duration_ms = duration;
            self.remaining_ms = duration;
            self.ring_duration_ms = duration;
            if self.deadline_ms.is_some() {
                self.deadline_ms = Some(now + duration);
            }
        }
    }

    fn adjust(&mut self, amount: u64, subtract: bool, now: u64) -> Result<(), String> {
        let remaining = self.remaining(now);
        let next = if subtract {
            remaining.saturating_sub(amount)
        } else {
            remaining
                .checked_add(amount)
                .filter(|ms| *ms <= MAX_MS - now)
                .ok_or("That adjustment would exceed the supported duration.")?
        };
        let running = self.deadline_ms.is_some() || (remaining == 0 && next > 0);
        self.remaining_ms = next;
        self.last_adjustment_ms = amount;
        self.ring_duration_ms = self.ring_duration_ms.max(next);
        self.deadline_ms = running.then_some(now + next);
        if next > 0 {
            self.alarming = false;
        }
        Ok(())
    }

    fn view(&self, now: u64) -> TimerView {
        let remaining_ms = self.remaining(now);
        TimerView {
            id: self.id,
            name: self.name.clone(),
            duration_ms: self.duration_ms,
            remaining_ms,
            ring_duration_ms: self.ring_duration_ms,
            last_adjustment_ms: self.last_adjustment_ms,
            status: if self.alarming {
                "alarming"
            } else if remaining_ms == 0 {
                "completed"
            } else if self.deadline_ms.is_some() {
                "running"
            } else {
                "paused"
            },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    id: u64,
    name: String,
    duration_ms: u64,
    remaining_ms: u64,
    ring_duration_ms: u64,
    last_adjustment_ms: u64,
    status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    revision: u64,
    timers: Vec<TimerView>,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    version: u8,
    next_id: u64,
    timers: Vec<Timer>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            timers: Vec::new(),
        }
    }
}

impl Saved {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.next_id == 0 || self.next_id >= MAX_MS {
            return Err("Unsupported or invalid saved timer data.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for timer in &self.timers {
            if timer.id == 0
                || timer.id >= self.next_id
                || !ids.insert(timer.id)
                || timer.name.trim().is_empty()
                || timer.duration_ms == 0
                || timer.duration_ms > MAX_MS - now_ms()
                || timer.duration_ms % 1000 != 0
                || timer.remaining_ms > MAX_MS - now_ms()
                || timer.ring_duration_ms < timer.remaining_ms
                || timer.ring_duration_ms == 0
                || timer.ring_duration_ms > MAX_MS
                || timer.last_adjustment_ms == 0
                || timer.last_adjustment_ms > MAX_MS
                || timer.last_adjustment_ms % 1000 != 0
                || timer.deadline_ms.is_some_and(|deadline| deadline > MAX_MS)
            {
                return Err("Invalid saved timer data.".into());
            }
        }
        Ok(())
    }
}

struct Data {
    saved: Saved,
    revision: u64,
    error: Option<String>,
    audio_error: Option<String>,
    load_failed: bool,
    stopping: bool,
    alarm_playing: bool,
    last_emit: Instant,
}

pub struct Engine {
    data: Mutex<Data>,
    wake: Condvar,
    workers: Mutex<Vec<JoinHandle<()>>>,
    audio: Mutex<Option<AlarmAudio>>,
    path: PathBuf,
    app: Option<tauri::AppHandle>,
}

impl Engine {
    pub fn open(path: PathBuf, app: Option<tauri::AppHandle>) -> Arc<Self> {
        let loaded = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Saved>(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|saved| {
                    saved.validate()?;
                    Ok(saved)
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
            Err(e) => Err(e.to_string()),
        };
        let error = loaded.as_ref().err().map(|e| format!("Could not load {}: {e} The file has been left untouched. Fix it and restart the app.", path.display()));
        let mut saved = loaded.unwrap_or_default();
        let now = now_ms();
        for timer in &mut saved.timers {
            timer.remaining_ms = timer.remaining(now);
            timer.deadline_ms = None;
        }
        let ids: Vec<_> = saved.timers.iter().map(|timer| timer.id).collect();
        let engine = Arc::new(Self {
            data: Mutex::new(Data {
                saved,
                revision: 0,
                load_failed: error.is_some(),
                error,
                audio_error: None,
                stopping: false,
                alarm_playing: false,
                last_emit: Instant::now(),
            }),
            wake: Condvar::new(),
            workers: Mutex::new(Vec::new()),
            audio: Mutex::new(None),
            path,
            app,
        });
        if engine.app.is_some() {
            let weak = Arc::downgrade(&engine);
            match AlarmAudio::new(move |error| {
                if let Some(engine) = weak.upgrade() {
                    let mut data = engine.data.lock().unwrap();
                    if data.stopping
                        || (error.is_some()
                            && !data.saved.timers.iter().any(|timer| timer.alarming))
                    {
                        return;
                    }
                    data.audio_error = error;
                    engine.publish(&mut data);
                }
            }) {
                Ok(audio) => *engine.audio.lock().unwrap() = Some(audio),
                Err(error) => engine.data.lock().unwrap().audio_error = Some(error),
            }
        }
        for id in ids {
            if let Err(error) = engine.spawn(id) {
                engine.data.lock().unwrap().error = Some(error);
            }
        }
        engine
    }

    fn view(data: &Data) -> Snapshot {
        let now = now_ms();
        Snapshot {
            revision: data.revision,
            timers: data
                .saved
                .timers
                .iter()
                .map(|timer| timer.view(now))
                .collect(),
            error: data.error.clone().or_else(|| data.audio_error.clone()),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut data = self.data.lock().unwrap();
        data.revision += 1;
        Self::view(&data)
    }

    fn publish(&self, data: &mut Data) -> Snapshot {
        data.revision += 1;
        data.last_emit = Instant::now();
        let snapshot = Self::view(data);
        if let Some(app) = &self.app {
            if let Err(error) = app.emit("timer-update", &snapshot) {
                eprintln!("Timer update failed: {error}");
            }
        }
        snapshot
    }

    fn sync_alarm(&self, data: &mut Data) {
        let playing = !data.stopping && data.saved.timers.iter().any(|timer| timer.alarming);
        if playing != data.alarm_playing {
            if let Some(audio) = self.audio.lock().unwrap().as_ref() {
                if let Err(error) = audio.set_playing(playing) {
                    data.audio_error = Some(error);
                }
            }
            data.alarm_playing = playing;
        }
    }

    fn save(&self, data: &mut Data) -> Result<(), String> {
        if data.load_failed {
            return Err(data.error.clone().unwrap());
        }
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            fs::create_dir_all(self.path.parent().ok_or("Invalid timer storage path")?)?;
            let mut saved = data.saved.clone();
            let now = now_ms();
            for timer in &mut saved.timers {
                timer.remaining_ms = timer.remaining(now);
            }
            let temp = self.path.with_extension("json.tmp");
            let mut file = fs::File::create(&temp)?;
            file.write_all(&serde_json::to_vec_pretty(&saved)?)?;
            file.sync_all()?;
            drop(file);
            fs::rename(temp, &self.path)?;
            Ok(())
        })();
        data.error = result.err().map(|error| format!("Could not save timers: {error}. Changes are only in memory; retry an action before quitting."));
        match &data.error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn writable(data: &Data) -> Result<(), String> {
        if data.load_failed {
            Err(data.error.clone().unwrap())
        } else if data.stopping {
            Err("The app is quitting.".into())
        } else {
            Ok(())
        }
    }

    fn spawn(self: &Arc<Self>, id: u64) -> Result<(), String> {
        let engine = self.clone();
        let handle = thread::Builder::new()
            .name(format!("timer-{id}"))
            .spawn(move || {
                let mut data = engine.data.lock().unwrap();
                loop {
                    if data.stopping {
                        break;
                    }
                    let Some(timer) = data.saved.timers.iter_mut().find(|timer| timer.id == id)
                    else {
                        break;
                    };
                    let finished = timer.settle(now_ms());
                    let running = timer.deadline_ms.is_some();
                    if finished {
                        let _ = engine.save(&mut data);
                        engine.sync_alarm(&mut data);
                        engine.publish(&mut data);
                    } else if running && data.last_emit.elapsed() >= TICK {
                        engine.publish(&mut data);
                    }
                    data = if running {
                        engine.wake.wait_timeout(data, TICK).unwrap().0
                    } else {
                        engine.wake.wait(data).unwrap()
                    };
                }
            })
            .map_err(|error| format!("Could not start timer thread: {error}"))?;
        let mut workers = self.workers.lock().unwrap();
        workers.retain(|worker| !worker.is_finished());
        workers.push(handle);
        Ok(())
    }

    pub fn create(self: &Arc<Self>, name: String, seconds: u64) -> Result<Snapshot, String> {
        let name = valid_name(name)?;
        let duration = duration_ms(seconds)?;
        let mut data = self.data.lock().unwrap();
        Self::writable(&data)?;
        let id = data.saved.next_id;
        if id >= MAX_MS - 1 {
            return Err("Timer ID limit reached.".into());
        }
        data.saved.timers.push(Timer {
            id,
            name,
            duration_ms: duration,
            remaining_ms: duration,
            ring_duration_ms: duration,
            last_adjustment_ms: 60_000,
            deadline_ms: Some(now_ms() + duration),
            alarming: false,
        });
        if let Err(error) = self.spawn(id) {
            data.saved.timers.pop();
            return Err(error);
        }
        data.saved.next_id += 1;
        let _ = self.save(&mut data);
        let snapshot = self.publish(&mut data);
        self.wake.notify_all();
        Ok(snapshot)
    }

    fn change(
        &self,
        id: u64,
        action: impl FnOnce(&mut Timer, u64) -> Result<(), String>,
    ) -> Result<Snapshot, String> {
        let mut data = self.data.lock().unwrap();
        Self::writable(&data)?;
        let timer = data
            .saved
            .timers
            .iter_mut()
            .find(|timer| timer.id == id)
            .ok_or("Timer no longer exists.")?;
        let now = now_ms();
        let mut updated = timer.clone();
        updated.settle(now);
        action(&mut updated, now)?;
        updated.settle(now);
        *timer = updated;
        let _ = self.save(&mut data);
        self.sync_alarm(&mut data);
        let snapshot = self.publish(&mut data);
        self.wake.notify_all();
        Ok(snapshot)
    }

    pub fn edit(&self, id: u64, name: String, seconds: u64) -> Result<Snapshot, String> {
        let name = valid_name(name)?;
        let duration = duration_ms(seconds)?;
        self.change(id, |timer, now| {
            timer.edit(name, duration, now);
            Ok(())
        })
    }

    pub fn toggle(&self, id: u64) -> Result<Snapshot, String> {
        self.change(id, |timer, now| {
            timer.toggle(now);
            Ok(())
        })
    }

    pub fn adjust(&self, id: u64, seconds: u64, subtract: bool) -> Result<Snapshot, String> {
        let amount = duration_ms(seconds)?;
        self.change(id, |timer, now| timer.adjust(amount, subtract, now))
    }

    pub fn delete(&self, id: u64) -> Result<Snapshot, String> {
        let mut data = self.data.lock().unwrap();
        Self::writable(&data)?;
        let index = data
            .saved
            .timers
            .iter()
            .position(|timer| timer.id == id)
            .ok_or("Timer no longer exists.")?;
        data.saved.timers.remove(index);
        let _ = self.save(&mut data);
        self.sync_alarm(&mut data);
        let snapshot = self.publish(&mut data);
        self.wake.notify_all();
        Ok(snapshot)
    }

    pub fn shutdown(&self) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        if data.stopping {
            return Ok(());
        }
        if !data.load_failed {
            let previous = data.saved.clone();
            let now = now_ms();
            for timer in &mut data.saved.timers {
                timer.remaining_ms = timer.remaining(now);
                timer.deadline_ms = None;
            }
            if let Err(error) = self.save(&mut data) {
                data.saved = previous;
                self.publish(&mut data);
                return Err(error);
            }
        }
        data.stopping = true;
        self.sync_alarm(&mut data);
        self.wake.notify_all();
        drop(data);
        self.audio.lock().unwrap().take();
        for worker in self.workers.lock().unwrap().drain(..) {
            let _ = worker.join();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> Timer {
        Timer {
            id: 1,
            name: "Tea".into(),
            duration_ms: 5000,
            remaining_ms: 5000,
            ring_duration_ms: 5000,
            last_adjustment_ms: 60_000,
            deadline_ms: Some(6000),
            alarming: false,
        }
    }

    #[test]
    fn pause_resume_and_independence() {
        let mut first = timer();
        let second = timer();
        first.toggle(2000);
        assert_eq!(first.remaining(3000), 4000);
        assert_eq!(second.remaining(3000), 3000);
        first.toggle(4000);
        assert_eq!(first.remaining(5000), 3000);
    }

    #[test]
    fn editing_name_does_not_reset_but_duration_does() {
        let mut timer = timer();
        timer.edit("Coffee".into(), 5000, 2000);
        assert_eq!(timer.remaining(2000), 4000);
        timer.edit("Coffee".into(), 8000, 3000);
        assert_eq!(timer.remaining(3000), 8000);
        timer.toggle(4000);
        timer.edit("Coffee".into(), 9000, 5000);
        assert_eq!(timer.deadline_ms, None);
        assert_eq!(timer.remaining(9000), 9000);
    }

    #[test]
    fn adjustments_clamp_expand_ring_and_keep_restart_duration() {
        let mut timer = timer();
        timer.adjust(6000, false, 2000).unwrap();
        assert_eq!(timer.remaining(2000), 10000);
        assert_eq!(timer.ring_duration_ms, 10000);
        assert_eq!(timer.duration_ms, 5000);
        timer.adjust(20000, true, 2000).unwrap();
        assert!(timer.settle(2000));
        assert!(!timer.settle(2000));
        timer.toggle(3000);
        assert_eq!(timer.remaining(3000), 0);
        assert!(!timer.alarming);
        timer.toggle(3000);
        assert_eq!(timer.remaining(3000), 5000);
        assert_eq!(timer.ring_duration_ms, 5000);
    }

    #[test]
    fn adding_to_completed_starts_and_paused_stays_paused() {
        let mut timer = timer();
        timer.toggle(2000);
        timer.adjust(1000, false, 3000).unwrap();
        assert_eq!(timer.deadline_ms, None);
        timer.adjust(10000, true, 3000).unwrap();
        timer.adjust(2000, false, 4000).unwrap();
        assert_eq!(timer.remaining(5000), 1000);
        assert!(timer.settle(6000));
        assert!(!timer.settle(7000));
    }

    #[test]
    fn first_click_acknowledges_alarm_and_second_click_restarts() {
        let mut timer = timer();
        assert!(timer.settle(6000));
        assert_eq!(timer.view(6000).status, "alarming");
        timer.toggle(7000);
        assert_eq!(timer.remaining(7000), 0);
        assert_eq!(timer.deadline_ms, None);
        assert_eq!(timer.view(7000).status, "completed");
        timer.toggle(8000);
        assert_eq!(timer.remaining(8000), 5000);
        assert_eq!(timer.view(8000).status, "running");
    }

    #[test]
    fn renaming_keeps_alarm_but_retiming_or_adding_time_clears_it() {
        let mut timer = timer();
        timer.settle(6000);
        timer.edit("Renamed".into(), 5000, 7000);
        assert!(timer.alarming);
        let mut adjusted = timer.clone();
        adjusted.adjust(1000, false, 7000).unwrap();
        assert!(!adjusted.alarming);
        assert_eq!(adjusted.view(7000).status, "running");
        timer.edit("Renamed".into(), 10000, 7000);
        assert!(!timer.alarming);
        assert_eq!(timer.view(7000).status, "paused");
    }

    #[test]
    fn alarms_stay_independent_and_never_replay_after_reopening() {
        let path = test_path("alarms");
        let engine = Engine::open(path.clone(), None);
        engine.create("First".into(), 60).unwrap();
        engine.create("Second".into(), 60).unwrap();
        for id in [1, 2] {
            engine
                .change(id, |timer, now| {
                    timer.deadline_ms = Some(now);
                    Ok(())
                })
                .unwrap();
        }
        assert!(engine.data.lock().unwrap().alarm_playing);
        engine.toggle(1).unwrap();
        assert_eq!(engine.snapshot().timers[0].status, "completed");
        assert!(engine.data.lock().unwrap().alarm_playing);
        engine.toggle(2).unwrap();
        assert!(!engine.data.lock().unwrap().alarm_playing);
        engine.toggle(1).unwrap();
        assert_eq!(engine.snapshot().timers[0].status, "running");
        engine
            .change(1, |timer, now| {
                timer.deadline_ms = Some(now);
                Ok(())
            })
            .unwrap();
        assert!(engine.data.lock().unwrap().alarm_playing);
        engine.delete(1).unwrap();
        assert!(!engine.data.lock().unwrap().alarm_playing);
        engine.toggle(2).unwrap();
        engine
            .change(2, |timer, now| {
                timer.deadline_ms = Some(now);
                Ok(())
            })
            .unwrap();
        assert!(engine.data.lock().unwrap().alarm_playing);
        engine.shutdown().unwrap();
        assert!(!engine.data.lock().unwrap().alarm_playing);
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(saved["timers"][0].get("alarming").is_none());
        let engine = Engine::open(path.clone(), None);
        assert_eq!(engine.snapshot().timers[0].status, "completed");
        assert!(!engine.data.lock().unwrap().alarm_playing);
        engine.shutdown().unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn validates_names_durations_and_overflow() {
        assert!(valid_name("  ".into()).is_err());
        assert_eq!(valid_name(" Tea ".into()).unwrap(), "Tea");
        assert!(duration_ms(0).is_err());
        assert!(duration_ms(u64::MAX).is_err());
        assert_eq!(duration_ms(3661).unwrap(), 3_661_000);
        let mut timer = timer();
        assert!(timer.adjust(MAX_MS, false, 1000).is_err());
        assert_eq!(timer.remaining(1000), 5000);
    }

    fn test_path(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "timer-desktop-{}-{label}-{}",
                std::process::id(),
                now_ms()
            ))
            .join("timers.json")
    }

    #[test]
    fn persistence_and_worker_lifecycle() {
        let path = test_path("roundtrip");
        let engine = Engine::open(path.clone(), None);
        engine.create("First".into(), 3601).unwrap();
        engine.create("Second".into(), 60).unwrap();
        engine.edit(1, "First renamed".into(), 3601).unwrap();
        engine.adjust(1, 150, false).unwrap();
        engine.toggle(2).unwrap();
        assert_eq!(engine.workers.lock().unwrap().len(), 2);
        engine.shutdown().unwrap();
        let engine = Engine::open(path.clone(), None);
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.timers.len(), 2);
        assert_eq!(snapshot.timers[0].name, "First renamed");
        assert_eq!(snapshot.timers[0].last_adjustment_ms, 150000);
        assert!(snapshot.timers[0].remaining_ms > 3_750_000);
        assert_eq!(snapshot.timers[0].status, "paused");
        assert_eq!(snapshot.timers[1].status, "paused");
        engine.delete(1).unwrap();
        engine.shutdown().unwrap();
        assert!(engine.workers.lock().unwrap().is_empty());
        let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        saved.validate().unwrap();
        assert_eq!(saved.timers.len(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn corrupt_storage_is_never_overwritten() {
        let path = test_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"broken json").unwrap();
        let engine = Engine::open(path.clone(), None);
        assert!(engine.snapshot().error.is_some());
        assert!(engine.create("Tea".into(), 5).is_err());
        engine.shutdown().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"broken json");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn worker_completes_and_saves_without_a_frontend() {
        let path = test_path("background");
        let engine = Engine::open(path.clone(), None);
        engine.create("Background".into(), 60).unwrap();
        {
            let mut data = engine.data.lock().unwrap();
            data.saved.timers[0].deadline_ms = Some(now_ms().saturating_sub(1));
        }
        engine.wake.notify_all();
        let timeout = Instant::now() + Duration::from_secs(3);
        loop {
            if engine.data.lock().unwrap().saved.timers[0]
                .deadline_ms
                .is_none()
            {
                break;
            }
            assert!(Instant::now() < timeout, "background worker did not finish");
            thread::sleep(Duration::from_millis(10));
        }
        let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.timers[0].remaining_ms, 0);
        assert_eq!(saved.timers[0].deadline_ms, None);
        assert_eq!(engine.snapshot().timers[0].status, "alarming");
        engine.shutdown().unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_save_reports_error_and_prevents_quit() {
        let path = test_path("blocked");
        fs::create_dir_all(&path).unwrap();
        let engine = Engine::open(path.clone(), None);
        // Simulate the storage path becoming unwritable after a successful load.
        engine.data.lock().unwrap().load_failed = false;
        assert!(engine.create("Tea".into(), 60).unwrap().error.is_some());
        assert!(engine.shutdown().is_err());
        assert!(!engine.data.lock().unwrap().stopping);
        fs::remove_dir(&path).unwrap();
        engine.shutdown().unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
