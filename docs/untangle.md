# Untangle decision log

## Scope and baseline

User selected **Tests only**: remove tests that would not catch a real bug and fix tests that fail without one. General comment cleanup is excluded. Preserve the current UI, six IPC commands, version-1 saved files, insertion order, per-timer threads, deadline-based calculation, immediate adjustments, two-click alarm acknowledgment/restart, background audio, and fallible clean Quit.

The checkout contains uncommitted/untracked application files and an existing deleted `.gitattributes`. Do not reset or overwrite unrelated changes. Git has only the initial attributes commit, so there is no application co-change history to infer from.

Before: `page → Tauri + time helpers + modal drafts`; `Tauri shell → engine → timer rules + JSON/files + audio + Tauri`; test harness reincludes the production source graph. Frontend type checks and time checks pass. Existing Rust tests are being rerun before edits (14 normal tests, one hardware test).

## Decisions and complete inventory

### Keep timer rules, versioned storage, and runtime coordination behind distinct boundaries

- `src-tauri/src/timers.rs:1–208,221–533`: timer transitions, version-1 disk schema, filesystem I/O, worker ownership, Tauri events, and audio hardware change for different reasons but share one module.
- `Engine::open(path, Option<AppHandle>)` uses one desktop handle as the unrelated no-events/no-audio test switch.
- `Timer::toggle` assumes expiration has already been settled; `Engine::change` remembers pre/post settling while the worker repeats the convention.
- Validation, construction, snapshotting, saving, restoring, and shutdown independently sample wall time, and supported-time checks are repeated.
- Loading, checkpointing, and clean Quit each know how to turn a deadline into remaining time. Runtime timer serialization implicitly defines the disk format.
- `load_failed` and mutable `error` encode a linked invariant that makes an unwrap depend on unrelated error handling.

Fix: a pure timer transition boundary receives one timestamp per operation; a version-1 file store owns capture/restore/validation and immutable load protection; the engine coordinates workers and effects. Tauri/audio wiring belongs in the native adapter. Keep validation at the IPC/disk boundaries and preserve current error behavior.

### Give mutations and worker lifetime one owner

- Completion, creation, edits, and deletion repeat save → alarm sync → publish → wake; save returns an error but also mutates an error field relied upon by callers discarding its result.
- Workers hold `Arc<Engine>` while the engine owns their handles, so paused workers form an ownership cycle and every caller must remember shutdown.
- Startup starts workers before tray setup can fail.

Fix: one commit path captures persistence errors and emits the resulting snapshot/alarm state. Worker shared state is separate from the owning engine so dropping the owner stops/joins threads. Keep tick-only updates and save-before-Quit as explicitly different operations. Use a concrete ordered update channel for the native adapter, avoiding callbacks into the engine while its lock is held.

### Make the frontend session and editor own their complete workflows

- `src/routes/+page.svelte:9–77`: IPC names, untyped payloads, revision merging, subscriptions, focus refresh, readiness, mutation serialization, and view errors are interleaved.
- `command(..., inModal)` resets editor messages inside transport code.
- Initial refresh swallows its failure, and mount still marks the connection ready.
- Editor drafts, confirmation, immediate adjustments, name/duration saving, and focus logic are spread around grid code and markup.
- The modal queries the page's `.add-card` selector to recover focus.
- Default duration appears twice; create-only adjustment initialization duplicates a saved timer's existing source of truth.
- Status-to-action meaning, card classes, visible status, and their styles change together but are scattered.
- Live draft formatting treats expected incomplete input as a swallowed parsing exception.

Fix: a typed timer session owns transport, lifecycle, revision acceptance, readiness, and mutation serialization. Colocate the editor and card presentation under the timer frontend feature; the editor owns draft/errors/confirmation/focus. Share actual visual primitives once. Use an explicit parse outcome for both draft labels and submission validation. Preserve focus trap, `tick()` before focus, and immediate-adjustment/cancel semantics.

### Make setup links and remaining code ownership explicit

- Vite binds `127.0.0.1:1420`, while Tauri separately names `localhost:1420`.
- README and CI each own the same Linux package list.
- `static/vite.svg`, `static/tauri.svg`, and `static/svelte.svg` have no callers.
- Desktop-only Cargo builds also emit unused static and dynamic libraries, producing output collisions during release tests.
- Window restoration silently discards show/unminimize/focus errors.

Fix: derive Vite's endpoint from Tauri config, centralize Linux setup, remove dead starter assets and unused library output types, and log best-effort restoration failures. Keep real native assets and platform requirements.

### Clean tests last, through behavior rather than implementation

- `src-tauri/tests/timers.rs` independently redeclares the production module graph with `#[path]`.
- Engine tests mutate private deadlines, handles, flags, and the internal `change` callback.
- The persistence test has a one-second I/O/scheduling budget unrelated to its requirement.
- The save-failure test starts from invalid storage then clears a private protection flag.
- The worker test polls a private field with a 10ms sleep.
- The idle-audio test accepts timeout as well as disconnection, allowing a leaked sender to pass.
- The signal test unnecessarily fixes mono output/exact amplitude rather than audible pulses, gaps, finite samples, and no clipping.
- `scripts/test-time.ts` checks parsing and splitting with a round trip that permits matching mistakes.
- Ignored historical browser QA uses a personal absolute helper path, sleeps, CSS-structure selectors, and a fake that reimplements backend rules.

Fix: import the public production core from integration tests, use deterministic pure timer operations for clock math, observe service events/snapshots for real worker completion, compare paused checkpoints exactly, cause external filesystem failures through public APIs, distinguish disconnection from timeout, and keep meaningful signal checks. Give the frontend session focused portable tests with canned responses/command assertions. Historical browser artifacts remain QA scratch data, not the regression contract.

## Necessary special cases retained

- Alarm acknowledgment requires a separate click from restart. Adding time to zero starts the timer; removing time clamps at zero; renaming preserves countdown/alarm; changed duration resets it.
- Corrupt/unsupported saves remain untouched. Failed persistence blocks normal Quit. Restored timers are paused and alarms never replay from disk.
- JavaScript-safe integer validation is a wire-boundary requirement.
- Timed waits are required for running workers; condition-variable waits are required for paused workers. An event timeout is legitimate for asynchronous tests.
- Native audio device streams stay on their owning thread. The real-device audio test is excluded from ordinary CI and run explicitly when hardware is available.
- Windows test executables need the common-controls manifest. Close-to-tray differs from Quit, and desktop click-event support varies.
- Thin Tauri command functions are required IPC adapters. Rust/TypeScript wire declarations are an explicit cross-language interface, not a reason to add a generic command registry/code generator.

## Verification and review

Pending: implement decisions, perform Tests-only cleanup, rerun frontend/Rust/build checks and browser behavior/layout checks, then have independent subagents inspect the final dependency boundaries and special cases. Repeat until no worthwhile structural issues remain. macOS/Linux native GUI/audio runs cannot be claimed from this Windows environment.
