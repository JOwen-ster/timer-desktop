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
- Set the amount for the **Add** and **Remove** quick buttons when creating or editing a timer. The default is one minute. In the edit dialog, **Add** and **Remove** also apply immediately and save the amount, even if you later cancel the dialog.
- To delete a timer, click the red trash icon on its card and confirm with the green checkmark. Click the red X to cancel.
- Closing the window hides it in the system tray. Click the tray icon or choose **Show timers** to return. **Quit** saves timer state and stops the workers. Running timers resume automatically on reopening. Manually paused timers stay paused.

Completion repeats a generated alarm tone through the default audio output until the timer is acknowledged. If several timers finish, the alarm continues until all ringing timers are acknowledged. Adding time, changing the duration, or deleting a ringing timer also ends that timer's alarm; renaming it does not. Quit clears alarms that are already ringing. On Windows, each completed timer also has a system notification: click the notification or choose **Open** to bring up the app, or choose **Snooze** to acknowledge that timer without opening the app. System/app volume controls whether the alarm is audible; audio-device failures appear in the app. The app does not start automatically at login.

While a timer rings, Timers temporarily mutes other apps and continues playing its own tone. It restores the audio it muted when the last ringing timer is cleared or you choose **Quit**. On macOS, allow system audio recording when prompted. Timers discards the audio immediately. On Linux, muting requires PulseAudio or PipeWire's PulseAudio service. On Windows, muting covers shared-mode app audio. If Timers is forcibly closed or an audio stream disappears during an alarm, you may need to unmute that app in the Windows or Linux system mixer.

## Build

```sh
bun run tauri build --no-bundle
```

Build on the target OS. The standalone executable is `src-tauri/target/release/timer-desktop.exe` on Windows and `src-tauri/target/release/timer-desktop` on macOS/Linux. For a native installer/app bundle, omit `--no-bundle`.

## GitHub releases

The desktop workflow builds pull requests, pushes to `main`, and manual runs. Downloads are available as workflow artifacts: Windows x64 NSIS/MSI installers, macOS Apple Silicon and Intel app/DMG bundles, and a Linux x64 AppImage.

To release, update the version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`, refresh the lockfiles, and commit. Push a matching tag, for example:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Tag builds upload their downloads to one draft GitHub Release. Wait for all four jobs to succeed, then publish the draft from the repository's Releases page. Manual runs on branches only build artifacts; selecting a version tag also updates its draft release. The workflow uses the automatic `GITHUB_TOKEN`; no additional secrets are required.

Windows builds are unsigned and macOS builds use ad-hoc signing without notarization, so operating-system download warnings are expected. Trusted distribution requires signing credentials and macOS notarization. The Linux AppImage targets x64; make it executable with `chmod +x Timers_*.AppImage` before launching. A desktop tray/AppIndicator and working audio output are still required.

## Storage

Timer data is saved as `timers.json` in the app-data directory:

| OS | Default location |
| --- | --- |
| Windows | `%APPDATA%/com.jowen-ster.timer-desktop/timers.json` |
| macOS | `~/Library/Application Support/com.jowen-ster.timer-desktop/timers.json` |
| Linux | `$XDG_DATA_HOME/com.jowen-ster.timer-desktop/timers.json`, normally `~/.local/share/com.jowen-ster.timer-desktop/timers.json` |

Writes use a synced temporary file followed by replacement. An unreadable or invalid save is left untouched and reported in the app. Save failures retain changes in memory and prevent a normal quit until saving succeeds.

Timers use system-clock deadlines, so time spent asleep or with the app closed counts toward running timers. If a saved running timer expires while the app is closed, it alarms when you reopen the app. Changing the system clock can affect running timers.
