use crate::audio::{AlarmAudio, AudioStatus};
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

fn adjustment_ms(seconds: u64) -> Result<u64, String> {
    seconds
        .checked_mul(1000)
        .filter(|ms| (1..=MAX_MS).contains(ms))
        .ok_or_else(|| "Enter a positive quick adjustment within the supported range.".into())
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
    mute_error: Option<String>,
    load_failed: bool,
    stopping: bool,
    alarm_playing: bool,
    last_emit: Instant,
}

fn alarm_change(playing: &mut bool, timers: &[Timer], stopping: bool) -> Option<bool> {
    let next = !stopping && timers.iter().any(|timer| timer.alarming);
    (std::mem::replace(playing, next) != next).then_some(next)
}

pub struct Engine {
    data: Mutex<Data>,
    wake: Condvar,
    workers: Mutex<Vec<JoinHandle<()>>>,
    audio: Mutex<Option<AlarmAudio>>,
    path: PathBuf,
    app: tauri::AppHandle,
}

impl Engine {
    pub fn open(path: PathBuf, app: tauri::AppHandle) -> Arc<Self> {
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
        let saved = loaded.unwrap_or_default();
        let ids: Vec<_> = saved.timers.iter().map(|timer| timer.id).collect();
        let engine = Arc::new(Self {
            data: Mutex::new(Data {
                saved,
                revision: 0,
                load_failed: error.is_some(),
                error,
                audio_error: None,
                mute_error: None,
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
        let weak = Arc::downgrade(&engine);
        match AlarmAudio::new(move |status| {
            if let Some(engine) = weak.upgrade() {
                let mut data = engine.data.lock().unwrap();
                if data.stopping {
                    return;
                }
                match status {
                    AudioStatus::Playback(Some(_)) if !data.alarm_playing => return,
                    AudioStatus::Playback(error) => data.audio_error = error,
                    AudioStatus::Muting(error) => data.mute_error = error,
                }
                engine.publish(&mut data);
            }
        }) {
            Ok(audio) => *engine.audio.lock().unwrap() = Some(audio),
            Err(error) => engine.data.lock().unwrap().audio_error = Some(error),
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
            error: data
                .error
                .as_ref()
                .or(data.audio_error.as_ref())
                .or(data.mute_error.as_ref())
                .cloned(),
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
        if let Err(error) = self.app.emit("timer-update", &snapshot) {
            eprintln!("Timer update failed: {error}");
        }
        snapshot
    }

    fn sync_alarm(&self, data: &mut Data) {
        if let Some(playing) =
            alarm_change(&mut data.alarm_playing, &data.saved.timers, data.stopping)
        {
            if let Some(audio) = self.audio.lock().unwrap().as_ref() {
                if let Err(error) = audio.set_playing(playing) {
                    data.audio_error = Some(error);
                }
            }
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
                    #[cfg(target_os = "windows")]
                    let finished_name = finished.then(|| timer.name.clone());
                    if finished {
                        let _ = engine.save(&mut data);
                        engine.sync_alarm(&mut data);
                        engine.publish(&mut data);
                        #[cfg(target_os = "windows")]
                        if let Some(name) = finished_name {
                            crate::notifications::show(engine.app.clone(), id, name);
                        }
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

    pub fn create(
        self: &Arc<Self>,
        name: String,
        seconds: u64,
        adjustment_seconds: u64,
    ) -> Result<Snapshot, String> {
        let name = valid_name(name)?;
        let duration = duration_ms(seconds)?;
        let last_adjustment_ms = adjustment_ms(adjustment_seconds)?;
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
            last_adjustment_ms,
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
        #[cfg(target_os = "windows")]
        if !timer.alarming {
            crate::notifications::dismiss(id);
        }
        let _ = self.save(&mut data);
        self.sync_alarm(&mut data);
        let snapshot = self.publish(&mut data);
        self.wake.notify_all();
        Ok(snapshot)
    }

    pub fn edit(
        &self,
        id: u64,
        name: String,
        seconds: u64,
        adjustment_seconds: u64,
    ) -> Result<Snapshot, String> {
        let name = valid_name(name)?;
        let duration = duration_ms(seconds)?;
        let last_adjustment_ms = adjustment_ms(adjustment_seconds)?;
        self.change(id, |timer, now| {
            timer.edit(name, duration, now);
            timer.last_adjustment_ms = last_adjustment_ms;
            Ok(())
        })
    }

    pub fn toggle(&self, id: u64) -> Result<Snapshot, String> {
        self.change(id, |timer, now| {
            timer.toggle(now);
            Ok(())
        })
    }

    pub fn acknowledge_alarm(&self, id: u64) -> Result<Option<Snapshot>, String> {
        let mut data = self.data.lock().unwrap();
        Self::writable(&data)?;
        let Some(timer) = data.saved.timers.iter_mut().find(|timer| timer.id == id) else {
            return Ok(None);
        };
        let now = now_ms();
        let mut updated = timer.clone();
        updated.settle(now);
        if !updated.alarming {
            return Ok(None);
        }
        updated.toggle(now);
        *timer = updated;
        #[cfg(target_os = "windows")]
        crate::notifications::dismiss(id);
        let _ = self.save(&mut data);
        self.sync_alarm(&mut data);
        let snapshot = self.publish(&mut data);
        self.wake.notify_all();
        Ok(Some(snapshot))
    }

    pub fn adjust(&self, id: u64, seconds: u64, subtract: bool) -> Result<Snapshot, String> {
        let amount = adjustment_ms(seconds)?;
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
        #[cfg(target_os = "windows")]
        crate::notifications::dismiss(id);
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
        if !data.load_failed
            && let Err(error) = self.save(&mut data)
        {
            self.publish(&mut data);
            return Err(error);
        }
        #[cfg(target_os = "windows")]
        for timer in data.saved.timers.iter().filter(|timer| timer.alarming) {
            crate::notifications::dismiss(timer.id);
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
    use crate::mute::sessions::Sessions;

    #[test]
    fn quick_adjustment_must_fit_in_a_saved_timer() {
        assert_eq!(adjustment_ms(45), Ok(45_000));
        assert!(adjustment_ms(0).is_err());
        let max_seconds = MAX_MS / 1000;
        assert!(adjustment_ms(max_seconds).is_ok());
        assert!(adjustment_ms(max_seconds + 1).is_err());
    }

    #[test]
    fn overlapping_alarms_restore_only_audio_muted_by_timers() {
        let timer = Timer {
            id: 1,
            name: "First".into(),
            duration_ms: 1000,
            remaining_ms: 1000,
            ring_duration_ms: 1000,
            last_adjustment_ms: 1000,
            deadline_ms: Some(1000),
            alarming: false,
        };
        let mut timers = [
            timer.clone(),
            Timer {
                id: 2,
                deadline_ms: Some(2000),
                ..timer
            },
        ];
        let mut playing = false;
        assert!(timers[0].settle(1000));
        assert_eq!(alarm_change(&mut playing, &timers, false), Some(true));

        let mut sessions = Sessions::new();
        sessions.observe("music", || Ok::<_, ()>(Some(()))).unwrap();
        sessions
            .observe("already muted", || Ok::<_, ()>(None))
            .unwrap();
        assert_eq!(sessions.observe("failed", || Err(())), Err(()));

        assert!(timers[1].settle(2000));
        assert_eq!(alarm_change(&mut playing, &timers, false), None);
        assert_eq!(
            sessions.observe::<()>("music", || panic!("muted twice")),
            Ok(())
        );

        timers[0].toggle(2000);
        assert_eq!(alarm_change(&mut playing, &timers, false), None);
        timers[1].toggle(2000);
        assert_eq!(alarm_change(&mut playing, &timers, false), Some(false));
        let mut restored = Vec::new();
        sessions
            .restore(|id, ()| {
                restored.push(id);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(restored, ["music"]);

        timers[0].alarming = true;
        assert_eq!(alarm_change(&mut playing, &timers, false), Some(true));
        assert_eq!(alarm_change(&mut playing, &timers, true), Some(false));
    }
}
