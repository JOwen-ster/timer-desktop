<script lang="ts">
  import { onMount } from "svelte";
  import TimerCard from "$lib/timers/TimerCard.svelte";
  import TimerEditor from "$lib/timers/TimerEditor.svelte";
  import { createTimerSession, type TimerMutation, type TimerRequests } from "$lib/timers/session";
  import { timerTransport } from "$lib/timers/transport";
  import type { Timer } from "$lib/timers/types";
  import "$lib/timers/styles.css";

  const session = createTimerSession(timerTransport);
  let editor: { open: (timer?: Timer) => Promise<void> };
  let addButton = $state<HTMLButtonElement>();
  let actionError = $state("");
  const snapshot = $derived($session.snapshot);
  const running = $derived(snapshot.timers.filter(timer => timer.status === "running").length);
  const error = $derived(actionError || $session.error);

  onMount(() => {
    void session.connect();
    const onFocus = () => { void session.refresh(); };
    const onVisibility = () => { if (!document.hidden) void session.refresh(); };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
      session.disconnect();
    };
  });

  async function command<K extends TimerMutation>(name: K, args: TimerRequests[K]) {
    actionError = "";
    try { await session.execute(name, args); }
    catch (cause) { actionError = String(cause); }
  }

  async function retry() {
    actionError = "";
    await session.refresh();
  }
</script>

<svelte:head><meta name="description" content="Independent countdown timers for your desktop." /></svelte:head>

<main>
  <header class="page-header">
    <div class="heading">
      <svg class="brand" viewBox="0 0 28 28" fill="none" aria-hidden="true"><circle cx="14" cy="15" r="10" stroke="currentColor" stroke-width="2" /><path d="M14 15V9M11 2h6M14 2v3" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
      <h1>Timers</h1>
    </div>
    <p class="summary">{snapshot.timers.length} saved<span aria-hidden="true"> · </span>{running} running</p>
  </header>

  {#if error || snapshot.error}
    <div class="error-banner" role="alert">
      <p>{error || snapshot.error}</p>
      {#if error}<button onclick={retry}>Retry</button>{/if}
    </div>
  {/if}

  <div class="timer-grid" aria-label="Saved timers" aria-busy={!$session.ready && !error}>
    <button bind:this={addButton} class="card add-card" aria-label="Add a new timer" disabled={!$session.ready || $session.busy} onclick={() => editor.open()}>
      <span class="add-circle" aria-hidden="true"><span class="plus"></span></span>
      <span class="add-label">New timer</span>
    </button>
    {#each snapshot.timers as timer (timer.id)}
      <TimerCard {timer} disabled={!$session.ready || $session.busy}
        onEdit={timer => editor.open(timer)}
        onToggle={id => command("toggle_timer", { id })}
        onAdjust={(id, seconds, subtract) => command("adjust_timer", { id, seconds, subtract })} />
    {/each}
  </div>
  <footer>Timers keep running in the system tray when you close this window.</footer>
</main>

<TimerEditor bind:this={editor} {session} fallbackFocus={addButton} />

<style>
  main { max-width: 1500px; margin: 0 auto; padding: 28px 48px 24px; }
  .page-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 28px; gap: 16px; }
  .heading { display: flex; align-items: center; gap: 10px; }
  .brand { width: 27px; height: 27px; }
  h1 { margin: 0; font-size: 24px; font-weight: 600; letter-spacing: -.7px; }
  .summary { margin: 0; color: #5e6163; font-size: 14px; }
  .summary span { margin: 0 6px; }
  /* At 1280px: 2×48px margins + 3×368px cards + 2×40px gaps = 1280px. */
  .timer-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 40px; }
  .add-card { display: flex; align-items: center; justify-content: center; padding: 0; }
  .add-circle { width: 61%; aspect-ratio: 1; border: 5px solid #30b0df; border-radius: 50%; display: grid; place-items: center; transition: border-color .15s; }
  .plus { position: relative; width: 61%; height: 61%; }
  .plus::before, .plus::after { content: ""; position: absolute; background: #292b2d; border-radius: 20px; }
  .plus::before { width: 100%; height: 12%; top: 44%; left: 0; }
  .plus::after { width: 12%; height: 100%; top: 0; left: 44%; }
  .add-label { position: absolute; bottom: 24px; font-size: 14px; color: #55585a; }
  .add-card:hover { background: #d5d5d5; }
  footer { margin-top: 32px; color: #626668; font-size: 12px; }
  .error-banner { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 18px; margin-bottom: 24px; background: #f8e8e6; border-radius: 8px; color: #84352e; }
  .error-banner p { margin: 0; overflow-wrap: anywhere; }
  .error-banner button { background: transparent; border: 1px solid; border-radius: 999px; padding: 6px 14px; }
  @media (max-width: 1000px) {
    main { padding: 24px 32px; }
    .timer-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 28px; }
  }
  @media (max-width: 620px) {
    main { padding: 24px 20px; }
    .page-header { margin-bottom: 22px; }
    .timer-grid { grid-template-columns: minmax(0, 1fr); gap: 24px; }
    footer { line-height: 1.6; }
  }
  @media (prefers-reduced-motion: reduce) { .add-circle { transition: none; } }
</style>
