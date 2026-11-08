use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source, source::SineWave};
use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

const AUDIO_RECOVERY: &str = "Check your audio output, then acknowledge and restart the timer.";
type AudioReport = Arc<dyn Fn(Option<String>) + Send + Sync>;

enum AudioCommand {
    Play,
    Stop,
    Shutdown,
}

fn alarm_tone() -> impl Source + Send {
    SineWave::new(880.0)
        .take_duration(Duration::from_millis(300))
        .amplify(0.2)
        .fade_in(Duration::from_millis(10))
        .fade_out(Duration::from_millis(300))
        .delay(Duration::from_millis(400))
        .repeat_infinite()
        .skip_duration(Duration::from_millis(400))
}

pub struct AlarmAudio {
    commands: mpsc::Sender<AudioCommand>,
    worker: Option<JoinHandle<()>>,
}

impl AlarmAudio {
    pub fn new(report: impl Fn(Option<String>) + Send + Sync + 'static) -> Result<Self, String> {
        let (commands, receiver) = mpsc::channel();
        let report: AudioReport = Arc::new(report);
        // The device stream stays on its owning thread on every platform.
        let worker = thread::Builder::new()
            .name("alarm-audio".into())
            .spawn(move || {
                let mut output = None;
                while let Ok(command) = receiver.recv() {
                    match command {
                        AudioCommand::Play if output.is_none() => match open_output(report.clone())
                        {
                            Ok(stream) => {
                                output = Some(stream);
                                report(None);
                            }
                            Err(error) => report(Some(error)),
                        },
                        AudioCommand::Play => {}
                        AudioCommand::Stop => {
                            output = None;
                            report(None);
                        }
                        AudioCommand::Shutdown => break,
                    }
                }
            })
            .map_err(|error| format!("Could not start alarm audio: {error}"))?;
        Ok(Self {
            commands,
            worker: Some(worker),
        })
    }

    pub fn set_playing(&self, playing: bool) -> Result<(), String> {
        self.commands
            .send(if playing {
                AudioCommand::Play
            } else {
                AudioCommand::Stop
            })
            .map_err(|_| "The alarm audio thread stopped unexpectedly.".into())
    }
}

impl Drop for AlarmAudio {
    fn drop(&mut self) {
        let _ = self.commands.send(AudioCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn open_output(report: AudioReport) -> Result<MixerDeviceSink, String> {
    let mut stream = DeviceSinkBuilder::from_default_device()
        .and_then(|builder| {
            builder
                .with_error_callback(move |error| {
                    report(Some(format!(
                        "Alarm audio stopped: {error}. {AUDIO_RECOVERY}"
                    )));
                })
                .open_sink_or_fallback()
        })
        .map_err(|error| format!("Could not play the alarm: {error}. {AUDIO_RECOVERY}"))?;
    stream.log_on_drop(false);
    stream.mixer().add(alarm_tone());
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarm_repeats_audible_pulses_and_silent_gaps_without_clipping() {
        let source = alarm_tone();
        let rate = source.sample_rate().get() as usize;
        let channels = source.channels().get() as usize;
        assert!(source.total_duration().is_none());
        let samples: Vec<_> = source.take(rate * channels * 2).collect();
        assert_eq!(samples.len(), rate * channels * 2);
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
        );
        let mut audible: Vec<_> = samples
            .chunks((rate * channels / 100).max(1))
            .map(|chunk| chunk.iter().any(|sample| sample.abs() > 0.001))
            .collect();
        audible.dedup();
        assert!(
            audible
                .windows(3)
                .any(|sequence| sequence == [true, false, true])
        );
    }

    #[test]
    fn idle_audio_stops_and_shuts_down_without_opening_a_device() {
        let (sender, receiver) = mpsc::channel();
        let audio = AlarmAudio::new(move |error| {
            let _ = sender.send(error);
        })
        .unwrap();
        audio.set_playing(false).unwrap();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(3)).unwrap(), None);
        drop(audio);
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    #[ignore = "requires a real audio output device; plays a short alarm"]
    fn native_alarm_playback_starts_and_stops() {
        let (sender, receiver) = mpsc::channel();
        let audio = AlarmAudio::new(move |error| {
            let _ = sender.send(error);
        })
        .unwrap();
        audio.set_playing(true).unwrap();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), None);
        thread::sleep(Duration::from_millis(800));
        audio.set_playing(false).unwrap();
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), None);
        drop(audio);
    }
}
