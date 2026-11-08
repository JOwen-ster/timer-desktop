<script lang="ts">
  import type { TimeFields } from "./time";
  let { value = $bindable(), prefix, disabled = false }: { value: TimeFields; prefix: string; disabled?: boolean } = $props();
</script>

<div class="time-input">
  <label for={`${prefix}-hours`}>Hours
    <input id={`${prefix}-hours`} type="number" min="0" step="1" inputmode="numeric" bind:value={value.hours} onblur={() => value.hours ??= 0} {disabled} />
  </label>
  <span aria-hidden="true">:</span>
  <label for={`${prefix}-minutes`}>Minutes
    <input id={`${prefix}-minutes`} type="number" min="0" max="59" step="1" inputmode="numeric" bind:value={value.minutes} onblur={() => value.minutes ??= 0} {disabled} />
  </label>
  <span aria-hidden="true">:</span>
  <label for={`${prefix}-seconds`}>Seconds
    <input id={`${prefix}-seconds`} type="number" min="0" max="59" step="1" inputmode="numeric" bind:value={value.seconds} onblur={() => value.seconds ??= 0} {disabled} />
  </label>
</div>

<style>
  .time-input { display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) auto minmax(0, 1fr); gap: 8px; align-items: end; }
  label { display: grid; gap: 8px; font-size: 13px; color: #515355; }
  input { width: 100%; min-width: 0; text-align: center; font-variant-numeric: tabular-nums; }
  span { padding-bottom: 12px; color: #727476; }
</style>
