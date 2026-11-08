export type Timer = {
  id: number;
  name: string;
  durationMs: number;
  remainingMs: number;
  ringDurationMs: number;
  lastAdjustmentMs: number;
  status: "running" | "paused" | "alarming" | "completed";
};

export type Snapshot = { revision: number; timers: Timer[]; error: string | null };
