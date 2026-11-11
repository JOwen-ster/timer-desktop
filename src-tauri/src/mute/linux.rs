use super::sessions::Sessions;
use libpulse_binding::{
    callbacks::ListResult,
    context::{Context, FlagSet, State},
    error::{Code, PAErr},
    mainloop::standard::Mainloop,
    operation::{Operation, State as OperationState},
    proplist::properties::APPLICATION_PROCESS_ID,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

const PULSE_TIMEOUT: Duration = Duration::from_secs(1);
type StreamId = (u32, Option<u32>);

pub(crate) struct Mute {
    pulse: Pulse,
    sessions: Sessions<StreamId, ()>,
}

struct Pulse {
    context: Context,
    mainloop: Mainloop,
}

impl Mute {
    pub(crate) fn start() -> Result<Self, String> {
        Ok(Self {
            pulse: Pulse::connect().map_err(|error| {
                format!("Could not mute other apps. A PulseAudio-compatible service is required: {error}")
            })?,
            sessions: Sessions::new(),
        })
    }

    pub(crate) fn refresh(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + PULSE_TIMEOUT;
        let result = (|| {
            let own_clients = self.pulse.own_clients(deadline)?;
            let streams = self.pulse.streams(deadline)?;
            let own_pid = std::process::id().to_string();
            let mut result = Ok(());
            for (id, pid, muted) in streams {
                if pid.as_deref() == Some(own_pid.as_str())
                    || id.1.is_some_and(|client| own_clients.contains(&client))
                {
                    continue;
                }
                let mut uncertain = None;
                let observed = self.sessions.observe(id, || {
                    if muted {
                        return Ok(None);
                    }
                    match self.pulse.set_mute(id.0, true, deadline) {
                        Ok(changed) => Ok(changed.then_some(())),
                        Err((error, true)) => {
                            // The server can apply a mute even when its reply times out.
                            uncertain = Some(error);
                            Ok(Some(()))
                        }
                        Err((error, false)) => Err(error),
                    }
                });
                result = result.and(observed).and(uncertain.map_or(Ok(()), Err));
            }
            result
        })();
        result.map_err(|error| format!("Could not mute other apps: {error}"))
    }

    pub(crate) fn restore(mut self) -> Result<(), String> {
        let deadline = Instant::now() + PULSE_TIMEOUT;
        let streams = self.pulse.streams(deadline)?;
        self.sessions.restore(|id, ()| {
            if streams
                .iter()
                .any(|(stream, _, muted)| *stream == id && *muted)
            {
                self.pulse
                    .set_mute(id.0, false, deadline)
                    .map_err(|(error, _)| error)?;
            }
            Ok(())
        })
    }
}

impl Pulse {
    fn connect() -> Result<Self, String> {
        let mainloop = Mainloop::new().ok_or("Could not create the audio main loop")?;
        let context = Context::new(&mainloop, "Timers audio control")
            .ok_or("Could not create the audio connection")?;
        let mut pulse = Self { context, mainloop };
        pulse
            .context
            .connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(error_message)?;
        let deadline = Instant::now() + PULSE_TIMEOUT;
        while pulse.context.get_state() != State::Ready {
            pulse.poll(deadline)?;
        }
        Ok(pulse)
    }

    fn own_clients(&mut self, deadline: Instant) -> Result<Vec<u32>, String> {
        self.check(deadline)?;
        let pid = std::process::id();
        // CPAL's native protocol client may only advertise the PID in its name.
        let cpal_name = format!("cpal-pulseaudio-{pid}");
        let pid = pid.to_string();
        let clients = Rc::new(RefCell::new(Vec::new()));
        let complete = Rc::new(Cell::new(false));
        let found = clients.clone();
        let finished = complete.clone();
        let op = self
            .context
            .introspect()
            .get_client_info_list(move |item| match item {
                ListResult::Item(client) => {
                    if client.name.as_deref() == Some(cpal_name.as_str())
                        || client.proplist.get_str(APPLICATION_PROCESS_ID).as_deref()
                            == Some(pid.as_str())
                    {
                        found.borrow_mut().push(client.index);
                    }
                }
                ListResult::End => finished.set(true),
                ListResult::Error => {}
            });
        self.wait(op, deadline)?;
        if !complete.get() {
            return Err(error_message(self.context.errno()));
        }
        Ok(clients.take())
    }

    fn streams(
        &mut self,
        deadline: Instant,
    ) -> Result<Vec<(StreamId, Option<String>, bool)>, String> {
        self.check(deadline)?;
        let streams = Rc::new(RefCell::new(Vec::new()));
        let complete = Rc::new(Cell::new(false));
        let found = streams.clone();
        let finished = complete.clone();
        let op = self
            .context
            .introspect()
            .get_sink_input_info_list(move |item| match item {
                ListResult::Item(stream) => found.borrow_mut().push((
                    (stream.index, stream.client),
                    stream.proplist.get_str(APPLICATION_PROCESS_ID),
                    stream.mute,
                )),
                ListResult::End => finished.set(true),
                ListResult::Error => {}
            });
        self.wait(op, deadline)?;
        if !complete.get() {
            return Err(error_message(self.context.errno()));
        }
        Ok(streams.take())
    }

    fn set_mute(
        &mut self,
        index: u32,
        muted: bool,
        deadline: Instant,
    ) -> Result<bool, (String, bool)> {
        self.check(deadline).map_err(|error| (error, false))?;
        let applied = Rc::new(Cell::new(false));
        let reply = applied.clone();
        let op = self.context.introspect().set_sink_input_mute(
            index,
            muted,
            Some(Box::new(move |success| reply.set(success))),
        );
        self.wait(op, deadline).map_err(|error| (error, true))?;
        if applied.get() {
            Ok(true)
        } else if Code::try_from(self.context.errno()) == Ok(Code::NoEntity) {
            Ok(false)
        } else {
            Err((error_message(self.context.errno()), false))
        }
    }

    fn wait<T: ?Sized>(&mut self, mut op: Operation<T>, deadline: Instant) -> Result<(), String> {
        while op.get_state() == OperationState::Running {
            if let Err(error) = self.poll(deadline) {
                op.cancel();
                return Err(error);
            }
        }
        Ok(())
    }

    fn check(&self, deadline: Instant) -> Result<(), String> {
        if matches!(self.context.get_state(), State::Failed | State::Terminated) {
            Err(error_message(self.context.errno()))
        } else if Instant::now() >= deadline {
            Err("The audio server did not respond in time".into())
        } else {
            Ok(())
        }
    }

    fn poll(&mut self, deadline: Instant) -> Result<(), String> {
        self.check(deadline)?;
        if !self.mainloop.iterate(false).is_success() {
            return Err(error_message(self.context.errno()));
        }
        thread::sleep(Duration::from_millis(2));
        Ok(())
    }
}

fn error_message(error: PAErr) -> String {
    error
        .to_string()
        .unwrap_or_else(|| format!("PulseAudio error {}", error.0))
}
