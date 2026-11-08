# Timers

A desktop countdown app built with Tauri 2, Svelte 5, and Rust, targeting Windows, macOS 14.2+, and Linux desktops. Each saved timer has its own worker thread; the UI displays deadline-based countdowns rather than driving them with JavaScript intervals.

## Run

Install Bun, Rust, and the [Tauri prerequisites for your OS](https://v2.tauri.app/start/prerequisites/), then run:

```sh
bun install
bun run tauri dev
```

The plain browser preview displays the layout but requires the desktop app's Rust engine to run timers.

On Debian/Ubuntu, install the desktop and audio build dependencies first:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libpulse-dev
```

Audio uses `rodio`/CPAL with WASAPI on Windows, CoreAudio on macOS, and PulseAudio (including PipeWire's PulseAudio service) with ALSA fallback on Linux. macOS 14.2 is the audio backend's documented minimum. Linux also needs a graphical desktop with a system tray/AppIndicator implementation for close-to-tray operation. Restore through the tray menu's **Show timers** action; clicking the icon directly is supported where the desktop provides click events.

## Use

- Click the first **+** card to create a timer. Enter a name and hours, minutes, and seconds; **Create & start** begins immediately.
- Click a timer's circle or card body to pause/resume. When it finishes, the first click stops and acknowledges the alarm, leaving it at zero. The next click runs its configured duration again.
- Click its name to edit. Changing the configured duration replaces the remaining time; changing only the name preserves the countdown. Saving preserves running/paused state.
- **Add time** and **Remove time** apply immediately, even if you later cancel the modal. Each timer remembers the last adjustment for its two quick buttons; the initial amount is one minute.
- **Delete timer** requires confirmation inside the edit modal.
- Closing the window hides it in the system tray. Click the tray icon or choose **Show timers** to return. **Quit** saves remaining times and stops the workers; reopening restores timers paused.

Completion repeats a generated alarm tone through the default audio output until the timer is acknowledged. If several timers finish, the alarm continues until all ringing timers are acknowledged. Adding time, changing the duration, or deleting a ringing timer also ends that timer's alarm; renaming it does not. Quit stops all sound, and reopening never replays old alarms. System/app volume controls whether it is audible; audio-device failures appear in the app. There are no desktop notifications or automatic startup at login.

## Build

```sh
bun run tauri build --no-bundle
```

Build on the target OS. The standalone executable is `src-tauri/target/release/timer-desktop.exe` on Windows and `src-tauri/target/release/timer-desktop` on macOS/Linux. For a native installer/app bundle, omit `--no-bundle`.

## Verify

Quit any running debug build from the tray before rebuilding or running Cargo tests; closing its window only hides it, and Windows keeps the executable locked.

```sh
bun run check
bun run test
bun run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

Rust tests cover pause/resume, independent countdowns, editing, adjustments, overflow, completion, worker shutdown, persistence, storage errors, and the generated audio signal. The Windows test target has a common-controls manifest required by Tauri's native dependencies.

The `Desktop checks` GitHub Actions workflow builds and tests on Windows, macOS, and Ubuntu when pushed or run manually. These checks do not require speakers. On each target OS, also run the hardware check below, then verify completion/acknowledgment while the window is hidden in the tray:

```sh
cargo test --manifest-path src-tauri/Cargo.toml native_alarm_playback_starts_and_stops -- --ignored
```

This plays a short alarm through the real audio device and checks start/stop. It is excluded from ordinary tests because CI runners may have no audio hardware.

Timer data is saved as `timers.json` in the app-data directory:

| OS | Default location |
| --- | --- |
| Windows | `%APPDATA%/com.jowen-ster.timer-desktop/timers.json` |
| macOS | `~/Library/Application Support/com.jowen-ster.timer-desktop/timers.json` |
| Linux | `$XDG_DATA_HOME/com.jowen-ster.timer-desktop/timers.json`, normally `~/.local/share/com.jowen-ster.timer-desktop/timers.json` |

Writes use a synced temporary file followed by replacement. An unreadable or invalid save is left untouched and reported in the app. Save failures retain changes in memory and prevent a normal quit until saving succeeds.

Timers use system-clock deadlines, including time spent asleep. No sound can play while the computer is asleep or the app is not running; an expired timer completes when execution resumes. After an unexpected termination, saved deadlines are reconciled on reopening and timers reopen paused. Changing the system clock can affect running timers.

Windows is verified locally. The macOS/Linux implementations and CI jobs are provided, but their native builds, tray behavior, and physical audio still require verification on those systems; adding the workflow does not mean those jobs have already run.
