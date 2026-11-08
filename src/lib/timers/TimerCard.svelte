<script lang="ts">
  import { formatTime } from "./time";
  import type { Timer } from "./types";

  let { timer, disabled, onEdit, onToggle, onAdjust }: {
    timer: Timer;
    disabled: boolean;
    onEdit: (timer: Timer) => void;
    onToggle: (id: number) => void;
    onAdjust: (id: number, seconds: number, subtract: boolean) => void;
  } = $props();

  function actionLabel(timer: Timer) {
    if (timer.status === "alarming") return "Stop alarm for";
    return timer.status === "running" ? "Pause" : timer.status === "completed" ? "Run again" : "Resume";
  }
</script>

      <article class="card timer-card" class:paused={timer.status === "paused"} class:completed={timer.status === "completed" || timer.status === "alarming"} class:alarming={timer.status === "alarming"} aria-label={`${timer.name} timer`}>
        <button class="timer-body" disabled={disabled} aria-label={`${actionLabel(timer)} ${timer.name}, ${formatTime(timer.remainingMs)} remaining`} onclick={() => onToggle(timer.id)}>
          <span class="countdown">
            <svg viewBox="0 0 220 220" aria-hidden="true">
              <circle class="ring-track" cx="110" cy="110" r="100" />
              <circle class="ring-progress" cx="110" cy="110" r="100" pathLength="1" stroke-dasharray="1" stroke-dashoffset={1 - Math.min(1, timer.remainingMs / timer.ringDurationMs)} />
            </svg>
            <span class="time" class:long-time={formatTime(timer.remainingMs).length > 8}>{formatTime(timer.remainingMs)}</span>
          </span>
          <span class="timer-status">
            {#if timer.status === "running"}<span class="pause-icon" aria-hidden="true"></span>Running
            {:else if timer.status === "paused"}<span class="play-icon" aria-hidden="true"></span>Paused
            {:else if timer.status === "alarming"}<span aria-hidden="true">♪</span>Time’s up · Stop alarm
            {:else}<span aria-hidden="true">↻</span>Finished · Run again{/if}
          </span>
        </button>
        <button class="timer-name" title={`Edit ${timer.name}`} aria-label={`Edit ${timer.name}`} disabled={disabled} onclick={() => onEdit(timer)}>
          <span>{timer.name}</span>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 5 4 4M5 15l-1 5 5-1L20 8a2.83 2.83 0 0 0-4-4ZM4 23h16" /></svg>
        </button>
        <div class="quick-actions">
          <button class="pill" disabled={disabled} aria-label={`Remove ${formatTime(timer.lastAdjustmentMs, true)} from ${timer.name}`} onclick={() => onAdjust(timer.id, timer.lastAdjustmentMs / 1000, true)}>−{formatTime(timer.lastAdjustmentMs, true)}</button>
          <button class="pill" disabled={disabled} aria-label={`Add ${formatTime(timer.lastAdjustmentMs, true)} to ${timer.name}`} onclick={() => onAdjust(timer.id, timer.lastAdjustmentMs / 1000, false)}>+{formatTime(timer.lastAdjustmentMs, true)}</button>
        </div>
      </article>

<style>
  .timer-body { position: absolute; inset: 0; width: 100%; height: 100%; border: 0; padding: 0; background: transparent; display: flex; align-items: center; justify-content: center; }
  .timer-body:hover { background: #00000004; }
  .timer-body:focus-visible { outline-offset: -7px; border-radius: 24px; }
  .countdown { position: absolute; width: 60%; aspect-ratio: 1; top: 18%; left: 20%; display: grid; place-items: center; }
  .countdown svg { position: absolute; width: 100%; height: 100%; transform: rotate(-90deg); overflow: visible; }
  .countdown circle { fill: none; stroke-width: 4.5; }
  .ring-track { stroke: #c6c9ca; }
  .ring-progress { stroke: #30b0df; stroke-linecap: butt; transition: stroke-dashoffset .24s linear; }
  .time { position: relative; font-size: clamp(28px, 3.8vw, 58px); letter-spacing: -1.8px; font-variant-numeric: tabular-nums; font-weight: 400; }
  .time.long-time { font-size: clamp(20px, 2.7vw, 40px); letter-spacing: -1px; }
  .timer-name { position: absolute; top: 12px; left: 12px; max-width: calc(100% - 24px); min-height: 36px; border: 1.5px solid #343638; border-radius: 999px; background: #ddd; padding: 3px 13px 5px; display: flex; align-items: center; gap: 8px; font-size: 18px; font-weight: 650; line-height: 1.3; }
  .timer-name span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .timer-name svg { width: 18px; height: 18px; flex: none; }
  .timer-name:hover { background: #f0f0f0; }
  .timer-status { position: absolute; bottom: 17%; display: flex; align-items: center; gap: 7px; font-size: 12px; color: #575b5e; }
  .pause-icon { width: 8px; height: 9px; border-left: 2px solid; border-right: 2px solid; }
  .play-icon { width: 0; height: 0; border-top: 5px solid transparent; border-bottom: 5px solid transparent; border-left: 7px solid; }
  .quick-actions { position: absolute; bottom: 10px; left: 10px; right: 10px; display: flex; justify-content: space-between; gap: 16px; pointer-events: none; }
  .quick-actions .pill { width: 47%; padding-left: 4px; padding-right: 4px; pointer-events: auto; font-size: clamp(12px, 1.5vw, 21px); }
  .paused .ring-progress { stroke: #858c90; }
  .completed .ring-track { stroke: #30b0df; }
  .completed .timer-status { color: #145d79; font-weight: 600; }
  .alarming { background: #d1e7ef; }
  @media (min-width: 1500px) { .time { font-size: 58px; } }
  @media (max-width: 1000px) {
    .time { font-size: clamp(32px, 5.5vw, 58px); }
    .time.long-time { font-size: 32px; }
    .quick-actions .pill { font-size: 18px; }
  }
  @media (max-width: 620px) {
    .time { font-size: clamp(36px, 10vw, 58px); }
    .time.long-time { font-size: 32px; }
    .timer-name { min-height: 44px; }
    .quick-actions .pill { min-height: 44px; font-size: 18px; }
    .timer-status { bottom: 17%; }
  }
  @media (prefers-reduced-motion: reduce) { .ring-progress { transition: none; } }
</style>
