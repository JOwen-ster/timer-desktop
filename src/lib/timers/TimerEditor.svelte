<script lang="ts">
  import { tick } from "svelte";
  import TimeInput from "./TimeInput.svelte";
  import { formatTime, parseTime, splitTime, type TimeFields } from "./time";
  import type { TimerSession } from "./session";
  import type { Timer } from "./types";

  let { session, fallbackFocus }: { session: TimerSession; fallbackFocus?: HTMLElement } = $props();
  let dialog: HTMLDialogElement;
  let nameInput: HTMLInputElement;
  let editingId = $state<number | null>(null);
  let name = $state("");
  let duration = $state<TimeFields>(splitTime(0));
  let adjustment = $state<TimeFields>(splitTime(0));
  let modalError = $state("");
  let notice = $state("");
  let opener: HTMLElement | null = null;
  const snapshot = $derived($session.snapshot);
  const busy = $derived($session.busy);
  const selected = $derived(snapshot.timers.find(timer => timer.id === editingId));
  const parsedAdjustment = $derived(parseTime(adjustment));
  const adjustmentLabel = $derived(parsedAdjustment.error === null ? formatTime(parsedAdjustment.seconds * 1000, true) : "—");

  export async function open(timer?: Timer) {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    editingId = timer?.id ?? null;
    name = timer?.name ?? "";
    duration = splitTime(timer?.durationMs ?? 300_000);
    adjustment = splitTime(timer?.lastAdjustmentMs ?? 60_000);
    modalError = "";
    notice = "";
    dialog.showModal();
    await tick();
    nameInput.focus();
    nameInput.select();
  }

  function trapFocus(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const controls = Array.from(dialog.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled])"))
      .filter(control => control.getClientRects().length > 0);
    const target = event.shiftKey ? controls.at(-1) : controls[0];
    if (!controls.length || (event.shiftKey && document.activeElement === controls[0])
      || (!event.shiftKey && document.activeElement === controls.at(-1))) {
      event.preventDefault();
      target?.focus();
    }
  }

  async function afterClose() {
    await tick();
    if (opener?.isConnected) opener.focus();
    else fallbackFocus?.focus();
  }

  async function perform(action: () => Promise<boolean>) {
    modalError = "";
    notice = "";
    try { return await action(); }
    catch (cause) { modalError = cause instanceof Error ? cause.message : String(cause); return false; }
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    const parsed = parseTime(duration);
    if (parsed.error !== null) { modalError = parsed.error; return; }
    if (parsedAdjustment.error !== null) { modalError = `Quick adjustment. ${parsedAdjustment.error}`; return; }
    const seconds = parsed.seconds;
    const adjustmentSeconds = parsedAdjustment.seconds;
    const timerName = name.trim() || formatTime(seconds * 1000, true);
    const success = await perform(() => editingId === null
      ? session.execute("create_timer", { name: timerName, seconds, adjustmentSeconds })
      : session.execute("edit_timer", { id: editingId, name: timerName, seconds, adjustmentSeconds }));
    if (success) dialog.close();
  }

  async function adjust(subtract: boolean) {
    if (editingId === null) return;
    if (parsedAdjustment.error !== null) { modalError = parsedAdjustment.error; return; }
    const id = editingId;
    const seconds = parsedAdjustment.seconds;
    if (await perform(() => session.execute("adjust_timer", { id, seconds, subtract }))) {
      notice = `${subtract ? "Removed" : "Added"} ${formatTime(seconds * 1000, true)}. Quick buttons updated.`;
    }
  }

</script>

<dialog bind:this={dialog} onkeydown={trapFocus} onclose={afterClose} oncancel={event => { if (busy) event.preventDefault(); }} aria-labelledby="modal-title">
  <div class="modal-heading">
    <h2 id="modal-title">{editingId === null ? "New timer" : "Edit timer"}</h2>
    <button class="close-button" type="button" aria-label="Close dialog" disabled={busy} onclick={() => dialog.close()}>×</button>
  </div>
  <form onsubmit={save} novalidate>
    <label class="name-label" for="timer-name">Name (optional)</label>
    <input bind:this={nameInput} id="timer-name" class="name-input" bind:value={name} placeholder="Uses the timer duration" autocomplete="off" disabled={busy} />

    <fieldset disabled={busy}>
      <legend>{editingId === null ? "Duration" : "Configured duration"}</legend>
      <TimeInput bind:value={duration} prefix="duration" disabled={busy} />
    </fieldset>
    {#if editingId !== null}
      <p class="field-note">Changing the duration replaces the countdown. Renaming keeps it running as it is.</p>
    {/if}
    <section class="adjust-section" aria-labelledby="adjust-title">
      <div class="section-heading"><h3 id="adjust-title">Quick adjustment</h3>{#if selected}<span>{formatTime(selected.remainingMs)} left</span>{/if}</div>
      <TimeInput bind:value={adjustment} prefix="adjustment" disabled={busy} />
      {#if editingId !== null}
        <div class="adjust-buttons">
          <button type="button" class="pill" disabled={busy} onclick={() => adjust(true)}>− Remove {adjustmentLabel}</button>
          <button type="button" class="pill" disabled={busy} onclick={() => adjust(false)}>+ Add {adjustmentLabel}</button>
        </div>
        <p class="field-note">Add and Remove apply immediately and save the amount. To update only the quick buttons, choose Save changes.</p>
        <p class="notice" role="status">{notice}</p>
      {:else}
        <p class="field-note">Sets how much time the quick buttons add or remove.</p>
      {/if}
    </section>

    {#if modalError || snapshot.error}<p class="modal-error" role="alert">{modalError || snapshot.error}</p>{/if}
    <div class="modal-actions">
      <button class="secondary" type="button" disabled={busy} onclick={() => dialog.close()}>Cancel</button>
      <button class="primary" type="submit" disabled={busy}>{busy ? "Saving…" : editingId === null ? "Create & start" : "Save changes"}</button>
    </div>
  </form>
</dialog>

<style>
  dialog { width: min(480px, calc(100% - 32px)); max-height: calc(100dvh - 32px); border: 1.5px solid #343638; border-radius: 22px; background: #f6f6f6; color: #292b2d; padding: 28px; box-shadow: 0 16px 64px #0002; }
  dialog::backdrop { background: #16191c66; }
  .modal-heading { display: flex; align-items: center; justify-content: space-between; margin-bottom: 24px; }
  h2 { margin: 0; font-size: 24px; font-weight: 600; letter-spacing: -.5px; }
  .close-button { border: 0; background: transparent; font-size: 28px; width: 36px; height: 36px; line-height: 1; border-radius: 50%; }
  .close-button:hover { background: #e3e3e3; }
  .name-label { display: block; margin-bottom: 8px; font-size: 14px; font-weight: 600; }
  .name-input { display: block; width: 100%; }
  fieldset { border: 0; padding: 0; margin: 24px 0 0; min-width: 0; }
  legend { padding: 0; margin-bottom: 12px; font-size: 14px; font-weight: 600; }
  .field-note { font-size: 12px; line-height: 1.6; color: #63676a; margin: 10px 0 0; }
  .adjust-section { border-top: 1px solid #d2d4d5; margin-top: 24px; padding-top: 24px; }
  .section-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 16px; }
  h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .section-heading span { font-size: 12px; color: #5c6366; font-variant-numeric: tabular-nums; }
  .adjust-buttons { display: flex; gap: 12px; margin-top: 16px; }
  .adjust-buttons .pill { flex: 1; font-size: 14px; min-height: 42px; background: #e4e4e4; }
  .adjust-buttons .pill:hover { background: #d7d7d7; }
  .notice { color: #145d79; font-size: 12px; margin: 10px 0 0; min-height: 18px; }
  .modal-error { color: #9a342a; font-size: 14px; line-height: 1.5; overflow-wrap: anywhere; }
  .modal-actions { display: flex; justify-content: flex-end; gap: 12px; margin-top: 28px; }
  .primary, .secondary { border: 1px solid #343638; border-radius: 999px; min-height: 42px; padding: 9px 20px; font-size: 14px; font-weight: 600; }
  .primary { color: #fff; background: #292b2d; }
  .primary:hover { background: #45494b; }
  .secondary { background: transparent; }
  .secondary:hover { background: #e4e4e4; }
  @media (max-width: 620px) {
    dialog { padding: 22px; }
    .adjust-buttons { gap: 8px; }
    .adjust-buttons .pill { padding: 8px; }
  }
</style>
