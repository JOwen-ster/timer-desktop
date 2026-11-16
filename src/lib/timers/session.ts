import { writable } from "svelte/store";
import type { Snapshot } from "./types";

export type TimerRequests = {
  list_timers: Record<string, never>;
  create_timer: { name: string; seconds: number; adjustmentSeconds: number };
  edit_timer: { id: number; name: string; seconds: number; adjustmentSeconds: number };
  toggle_timer: { id: number };
  adjust_timer: { id: number; seconds: number; subtract: boolean };
  delete_timer: { id: number };
};
export type TimerMutation = Exclude<keyof TimerRequests, "list_timers">;

export interface TimerTransport {
  invoke<K extends keyof TimerRequests>(name: K, args: TimerRequests[K]): Promise<Snapshot>;
  listen(handler: (snapshot: Snapshot) => void): Promise<() => void>;
}

export type SessionState = {
  snapshot: Snapshot;
  ready: boolean;
  busy: boolean;
  error: string;
};

export function createTimerSession(transport: TimerTransport) {
  let state: SessionState = {
    snapshot: { revision: -1, timers: [], error: null },
    ready: false,
    busy: false,
    error: "",
  };
  const store = writable(state);
  let generation = 0;
  let unlisten: (() => void) | undefined;
  let connecting: Promise<void> | undefined;

  function update(next: Partial<SessionState>) {
    state = { ...state, ...next };
    store.set(state);
  }

  function accept(snapshot: Snapshot) {
    if (snapshot.revision >= state.snapshot.revision) update({ snapshot });
  }

  function connect(): Promise<void> {
    if (connecting) return connecting;
    if (state.ready) return Promise.resolve();
    const current = ++generation;
    update({ ready: false, error: "" });
    const attempt = (async () => {
      try {
        const stop = await transport.listen(snapshot => {
          if (current === generation) accept(snapshot);
        });
        if (current !== generation) { stop(); return; }
        unlisten = stop;
        const snapshot = await transport.invoke("list_timers", {});
        if (current === generation) {
          accept(snapshot);
          update({ ready: true });
        }
      } catch (cause) {
        if (current === generation) {
          unlisten?.();
          unlisten = undefined;
          update({ ready: false, error: String(cause) });
        }
      }
    })();
    connecting = attempt;
    void attempt.finally(() => { if (connecting === attempt) connecting = undefined; });
    return attempt;
  }

  async function refresh() {
    if (!state.ready) return connect();
    const current = generation;
    try {
      const snapshot = await transport.invoke("list_timers", {});
      if (current === generation) {
        accept(snapshot);
        update({ error: "" });
      }
    } catch (cause) {
      if (current === generation) update({ error: String(cause) });
    }
  }

  async function execute<K extends TimerMutation>(name: K, args: TimerRequests[K]): Promise<boolean> {
    if (state.busy) return false;
    if (!state.ready) throw new Error("The timer engine is not connected.");
    const current = generation;
    update({ busy: true });
    try {
      const snapshot = await transport.invoke(name, args);
      if (current !== generation) return false;
      accept(snapshot);
      return true;
    } finally {
      if (current === generation) update({ busy: false });
    }
  }

  function disconnect() {
    generation += 1;
    unlisten?.();
    unlisten = undefined;
    connecting = undefined;
    update({ ready: false, busy: false });
  }

  return { subscribe: store.subscribe, connect, refresh, execute, disconnect };
}

export type TimerSession = ReturnType<typeof createTimerSession>;
