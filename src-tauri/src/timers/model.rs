use serde::Serialize;

// Milliseconds and IDs must remain exactly representable in JavaScript.
pub const MAX_MS: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerCheckpoint {
    pub id: u64,
    pub name: String,
    pub duration_ms: u64,
    pub remaining_ms: u64,
    pub ring_duration_ms: u64,
    pub last_adjustment_ms: u64,
    pub deadline_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Timer {
    state: TimerCheckpoint,
    alarming: bool,
}

pub enum TimerAction {
    Edit { name: String, seconds: u64 },
    Toggle,
    Adjust { seconds: u64, subtract: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimerStatus {
    Running,
    Paused,
    Alarming,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerView {
    pub id: u64,
    pub name: String,
    pub duration_ms: u64,
    pub remaining_ms: u64,
    pub ring_duration_ms: u64,
    pub last_adjustment_ms: u64,
    pub status: TimerStatus,
}

fn deadline(now: u64, duration: u64) -> Result<u64, String> {
    now.checked_add(duration)
        .filter(|deadline| *deadline <= MAX_MS)
        .ok_or_else(|| "That duration would exceed the supported range.".into())
}

fn duration_ms(seconds: u64, now: u64) -> Result<u64, String> {
    let duration = seconds
        .checked_mul(1000)
        .filter(|duration| *duration > 0)
        .ok_or("Enter a positive duration within the supported range.")?;
    deadline(now, duration)?;
    Ok(duration)
}

fn name(name: String) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err("Give the timer a name.".into())
    } else {
        Ok(trimmed.into())
    }
}

impl Timer {
    pub fn new(id: u64, timer_name: String, seconds: u64, now: u64) -> Result<Self, String> {
        if id == 0 || id >= MAX_MS {
            return Err("Invalid timer ID.".into());
        }
        let timer_name = name(timer_name)?;
        let duration = duration_ms(seconds, now)?;
        Ok(Self {
            state: TimerCheckpoint {
                id,
                name: timer_name,
                duration_ms: duration,
                remaining_ms: duration,
                ring_duration_ms: duration,
                last_adjustment_ms: 60_000,
                deadline_ms: Some(deadline(now, duration)?),
            },
            alarming: false,
        })
    }

    pub fn restore_paused(state: TimerCheckpoint, now: u64) -> Result<Self, String> {
        if state.id == 0
            || state.id >= MAX_MS
            || state.name.trim().is_empty()
            || state.duration_ms == 0
            || state.duration_ms % 1000 != 0
            || deadline(now, state.duration_ms).is_err()
            || deadline(now, state.remaining_ms).is_err()
            || state.ring_duration_ms < state.remaining_ms
            || state.ring_duration_ms == 0
            || state.ring_duration_ms > MAX_MS
            || state.last_adjustment_ms == 0
            || state.last_adjustment_ms % 1000 != 0
            || state.last_adjustment_ms > MAX_MS
            || state.deadline_ms.is_some_and(|deadline| deadline > MAX_MS)
        {
            return Err("Invalid saved timer data.".into());
        }
        let mut timer = Self { state, alarming: false };
        timer.freeze(now);
        Ok(timer)
    }

    pub fn id(&self) -> u64 {
        self.state.id
    }

    pub fn is_running(&self) -> bool {
        self.state.deadline_ms.is_some()
    }

    pub fn has_alarm(&self) -> bool {
        self.alarming
    }

    fn remaining(&self, now: u64) -> u64 {
        self.state.deadline_ms.map_or(self.state.remaining_ms, |deadline| deadline.saturating_sub(now))
    }

    pub fn tick(&mut self, now: u64) -> bool {
        if self.is_running() && self.remaining(now) == 0 {
            self.state.remaining_ms = 0;
            self.state.deadline_ms = None;
            self.alarming = true;
            true
        } else {
            false
        }
    }

    pub fn apply(&mut self, action: TimerAction, now: u64) -> Result<(), String> {
        let mut updated = self.clone();
        updated.tick(now);
        match action {
            TimerAction::Edit { name: timer_name, seconds } => {
                let timer_name = name(timer_name)?;
                let duration = duration_ms(seconds, now)?;
                updated.state.name = timer_name;
                if duration != updated.state.duration_ms {
                    updated.alarming = false;
                    updated.state.duration_ms = duration;
                    updated.state.remaining_ms = duration;
                    updated.state.ring_duration_ms = duration;
                    if updated.is_running() {
                        updated.state.deadline_ms = Some(deadline(now, duration)?);
                    }
                }
            }
            TimerAction::Toggle => {
                if updated.alarming {
                    updated.alarming = false;
                } else if updated.is_running() {
                    updated.state.remaining_ms = updated.remaining(now);
                    updated.state.deadline_ms = None;
                } else {
                    if updated.state.remaining_ms == 0 {
                        updated.state.remaining_ms = updated.state.duration_ms;
                        updated.state.ring_duration_ms = updated.state.duration_ms;
                    }
                    updated.state.deadline_ms = Some(deadline(now, updated.state.remaining_ms)?);
                }
            }
            TimerAction::Adjust { seconds, subtract } => {
                let amount = duration_ms(seconds, now)?;
                let remaining = updated.remaining(now);
                let next = if subtract {
                    remaining.saturating_sub(amount)
                } else {
                    let next = remaining.checked_add(amount).ok_or("That adjustment would exceed the supported duration.")?;
                    deadline(now, next)?;
                    next
                };
                let running = updated.is_running() || (remaining == 0 && next > 0);
                updated.state.remaining_ms = next;
                updated.state.last_adjustment_ms = amount;
                updated.state.ring_duration_ms = updated.state.ring_duration_ms.max(next);
                updated.state.deadline_ms = if running { Some(deadline(now, next)?) } else { None };
                if next > 0 {
                    updated.alarming = false;
                }
            }
        }
        updated.tick(now);
        *self = updated;
        Ok(())
    }

    pub fn view(&self, now: u64) -> TimerView {
        let remaining = self.remaining(now);
        TimerView {
            id: self.state.id,
            name: self.state.name.clone(),
            duration_ms: self.state.duration_ms,
            remaining_ms: remaining,
            ring_duration_ms: self.state.ring_duration_ms.max(remaining),
            last_adjustment_ms: self.state.last_adjustment_ms,
            status: if self.alarming { TimerStatus::Alarming }
                else if remaining == 0 { TimerStatus::Completed }
                else if self.is_running() { TimerStatus::Running }
                else { TimerStatus::Paused },
        }
    }

    pub fn checkpoint(&self, now: u64) -> TimerCheckpoint {
        let mut state = self.state.clone();
        state.remaining_ms = self.remaining(now);
        state.ring_duration_ms = state.ring_duration_ms.max(state.remaining_ms);
        state
    }

    pub fn freeze(&mut self, now: u64) {
        self.state = self.checkpoint(now);
        self.state.deadline_ms = None;
        self.alarming = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer() -> Timer {
        Timer::new(1, "Tea".into(), 5, 1000).unwrap()
    }

    #[test]
    fn timers_pause_resume_and_count_down_independently() {
        let mut first = timer();
        let second = timer();
        first.apply(TimerAction::Toggle, 2000).unwrap();
        assert_eq!(first.view(3000).remaining_ms, 4000);
        assert_eq!(second.view(3000).remaining_ms, 3000);
        first.apply(TimerAction::Toggle, 4000).unwrap();
        assert_eq!(first.view(5000).remaining_ms, 3000);
        assert_eq!(first.view(5000).status, TimerStatus::Running);
    }

    #[test]
    fn rename_preserves_countdown_while_duration_edits_reset_it() {
        let mut timer = timer();
        timer.apply(TimerAction::Edit { name: "Coffee".into(), seconds: 5 }, 2000).unwrap();
        assert_eq!(timer.view(2000).remaining_ms, 4000);
        timer.apply(TimerAction::Edit { name: "Coffee".into(), seconds: 8 }, 3000).unwrap();
        assert_eq!(timer.view(3000).remaining_ms, 8000);
        timer.apply(TimerAction::Toggle, 4000).unwrap();
        timer.apply(TimerAction::Edit { name: "Coffee".into(), seconds: 9 }, 5000).unwrap();
        assert_eq!(timer.view(9000).remaining_ms, 9000);
        assert_eq!(timer.view(9000).status, TimerStatus::Paused);
    }

    #[test]
    fn adjustments_expand_the_ring_clamp_at_zero_and_preserve_restart_duration() {
        let mut timer = timer();
        timer.apply(TimerAction::Adjust { seconds: 6, subtract: false }, 2000).unwrap();
        let view = timer.view(2000);
        assert_eq!(view.remaining_ms, 10000);
        assert_eq!(view.ring_duration_ms, 10000);
        assert_eq!(view.duration_ms, 5000);
        timer.apply(TimerAction::Adjust { seconds: 20, subtract: true }, 2000).unwrap();
        assert_eq!(timer.view(2000).status, TimerStatus::Alarming);
        timer.apply(TimerAction::Toggle, 3000).unwrap();
        assert_eq!(timer.view(3000).status, TimerStatus::Completed);
        assert_eq!(timer.view(3000).remaining_ms, 0);
        timer.apply(TimerAction::Toggle, 3000).unwrap();
        assert_eq!(timer.view(3000).remaining_ms, 5000);
        assert_eq!(timer.view(3000).ring_duration_ms, 5000);
        assert_eq!(timer.view(3000).last_adjustment_ms, 20000);
    }

    #[test]
    fn first_click_at_expiry_acknowledges_even_before_a_worker_tick() {
        let mut timer = timer();
        timer.apply(TimerAction::Toggle, 6000).unwrap();
        assert_eq!(timer.view(6000).status, TimerStatus::Completed);
        assert_eq!(timer.view(6000).remaining_ms, 0);
        timer.apply(TimerAction::Toggle, 7000).unwrap();
        assert_eq!(timer.view(7000).status, TimerStatus::Running);
        assert_eq!(timer.view(7000).remaining_ms, 5000);
    }

    #[test]
    fn completion_is_reported_once_and_requires_acknowledgment_before_restart() {
        let mut timer = timer();
        assert!(timer.tick(6000));
        assert!(!timer.tick(7000));
        assert!(timer.has_alarm());
        timer.apply(TimerAction::Toggle, 7000).unwrap();
        assert_eq!(timer.view(7000).status, TimerStatus::Completed);
        timer.apply(TimerAction::Toggle, 8000).unwrap();
        assert_eq!(timer.view(8000).remaining_ms, 5000);
    }

    #[test]
    fn paused_adjustments_stay_paused_and_adding_to_zero_starts() {
        let mut timer = timer();
        timer.apply(TimerAction::Toggle, 2000).unwrap();
        timer.apply(TimerAction::Adjust { seconds: 1, subtract: false }, 3000).unwrap();
        assert_eq!(timer.view(3000).status, TimerStatus::Paused);
        timer.apply(TimerAction::Adjust { seconds: 10, subtract: true }, 3000).unwrap();
        assert_eq!(timer.view(3000).status, TimerStatus::Completed);
        timer.apply(TimerAction::Adjust { seconds: 2, subtract: false }, 4000).unwrap();
        assert_eq!(timer.view(5000).remaining_ms, 1000);
        assert!(timer.tick(6000));
    }

    #[test]
    fn renaming_keeps_alarm_but_retiming_or_adding_time_clears_it() {
        let mut timer = timer();
        timer.tick(6000);
        timer.apply(TimerAction::Edit { name: "Renamed".into(), seconds: 5 }, 7000).unwrap();
        assert_eq!(timer.view(7000).status, TimerStatus::Alarming);
        let mut adjusted = timer.clone();
        adjusted.apply(TimerAction::Adjust { seconds: 1, subtract: false }, 7000).unwrap();
        assert_eq!(adjusted.view(7000).status, TimerStatus::Running);
        timer.apply(TimerAction::Edit { name: "Renamed".into(), seconds: 10 }, 7000).unwrap();
        assert_eq!(timer.view(7000).status, TimerStatus::Paused);
    }

    #[test]
    fn checkpoints_reconcile_deadlines_and_restore_without_alarms() {
        let live = timer();
        let restored = Timer::restore_paused(live.checkpoint(2000), 3000).unwrap();
        assert_eq!(restored.view(9000).remaining_ms, 3000);
        assert_eq!(restored.view(9000).status, TimerStatus::Paused);
        let expired = Timer::restore_paused(live.checkpoint(2000), 7000).unwrap();
        assert_eq!(expired.view(7000).status, TimerStatus::Completed);
        let mut ringing = live;
        ringing.tick(7000);
        let restored = Timer::restore_paused(ringing.checkpoint(7000), 8000).unwrap();
        assert_eq!(restored.view(8000).status, TimerStatus::Completed);
    }

    #[test]
    fn freeze_captures_remaining_time_and_clears_alarm() {
        let mut live = timer();
        live.freeze(2000);
        assert_eq!(live.view(9000).remaining_ms, 4000);
        assert_eq!(live.view(9000).status, TimerStatus::Paused);
        assert_eq!(live.checkpoint(9000).deadline_ms, None);
        let mut ringing = timer();
        ringing.tick(6000);
        ringing.freeze(7000);
        assert_eq!(ringing.view(7000).status, TimerStatus::Completed);
    }

    #[test]
    fn invalid_changes_are_atomic_even_at_expiry() {
        assert!(Timer::new(1, " ".into(), 5, 1000).is_err());
        assert!(Timer::new(1, "Tea".into(), 0, 1000).is_err());
        assert!(Timer::new(1, "Tea".into(), u64::MAX, 1000).is_err());
        assert_eq!(Timer::new(1, " Tea ".into(), 5, 1000).unwrap().view(1000).name, "Tea");
        let mut timer = timer();
        let before = timer.checkpoint(1000);
        assert!(timer.apply(TimerAction::Edit { name: " ".into(), seconds: 5 }, 7000).is_err());
        assert_eq!(timer.checkpoint(1000), before);
        assert!(timer.tick(7000));
        let before = timer.view(7000);
        assert!(timer.apply(TimerAction::Adjust { seconds: u64::MAX, subtract: false }, 7000).is_err());
        assert_eq!(timer.view(7000), before);
    }
}
