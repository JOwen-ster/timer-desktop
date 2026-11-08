import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Snapshot } from "./types";
import type { TimerTransport } from "./session";

export const timerTransport: TimerTransport = {
  invoke: (name, args) => invoke<Snapshot>(name, args),
  async listen(handler) {
    if (!isTauri()) throw new Error("Open the Timers desktop app to create and run timers.");
    return listen<Snapshot>("timer-update", event => handler(event.payload));
  },
};
