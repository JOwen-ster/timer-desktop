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
