use crate::mute::Mute;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source, source::SineWave};
use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

const AUDIO_RECOVERY: &str = "Check your audio output, then acknowledge and restart the timer.";
type AudioReport = Arc<dyn Fn(AudioStatus) + Send + Sync>;

pub(crate) enum AudioStatus {
    Playback(Option<String>),
    Muting(Option<String>),
}

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
    pub(crate) fn new(
        report: impl Fn(AudioStatus) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let (commands, receiver) = mpsc::channel();
        let report: AudioReport = Arc::new(report);
        // The device stream stays on its owning thread on every platform.
        let worker = thread::Builder::new()
            .name("alarm-audio".into())
            .spawn(move || {
                let mut output = None;
                let mut mute: Option<Mute> = None;
                let mut mute_error = None;
                let mut report_mute = |error| {
                    if mute_error != error {
                        mute_error = error;
                        report(AudioStatus::Muting(mute_error.clone()));
                    }
                };
                loop {
                    let command = if let Some(active) = mute.as_mut() {
                        match receiver.recv_timeout(Duration::from_millis(250)) {
                            Ok(command) => Some(command),
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                report_mute(active.refresh().err());
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => None,
                        }
                    } else {
                        receiver.recv().ok()
                    };
                    match command {
                        Some(AudioCommand::Play) if output.is_none() => {
                            report(AudioStatus::Playback(None));
                            match open_output(report.clone()) {
                                Ok(stream) => {
                                    output = Some(stream);
                                    match Mute::start() {
                                        Ok(mut active) => {
                                            report_mute(active.refresh().err());
                                            mute = Some(active);
                                        }
                                        Err(error) => report_mute(Some(error)),
                                    }
                                }
                                Err(error) => report(AudioStatus::Playback(Some(error))),
                            }
                        }
                        Some(AudioCommand::Play) => {}
                        Some(AudioCommand::Stop) => {
                            output = None;
                            report(AudioStatus::Playback(None));
                            report_mute(restore(mute.take()));
                        }
                        Some(AudioCommand::Shutdown) | None => {
                            drop(output);
                            restore(mute.take());
                            break;
                        }
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

fn restore(mute: Option<Mute>) -> Option<String> {
    let error = mute.and_then(|active| active.restore().err()).map(|error| {
        format!("Could not restore other apps’ audio: {error}. Check the system mixer.")
    });
    if let Some(error) = &error {
        eprintln!("{error}");
    }
    error
}

fn open_output(report: AudioReport) -> Result<MixerDeviceSink, String> {
    let mut stream = DeviceSinkBuilder::from_default_device()
        .and_then(|builder| {
            builder
                .with_error_callback(move |error| {
                    report(AudioStatus::Playback(Some(format!(
                        "Alarm audio stopped: {error}. {AUDIO_RECOVERY}"
                    ))));
                })
                .open_sink_or_fallback()
        })
        .map_err(|error| format!("Could not play the alarm: {error}. {AUDIO_RECOVERY}"))?;
    stream.log_on_drop(false);
    stream.mixer().add(alarm_tone());
    Ok(stream)
}
