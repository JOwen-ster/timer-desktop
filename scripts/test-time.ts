import { formatTime, secondsFrom, splitTime } from "../src/lib/time";

function assert(condition: boolean, message: string) { if (!condition) throw new Error(message); }
assert(formatTime(0) === "00:00", "zero");
assert(formatTime(1) === "00:01", "round remaining milliseconds up");
assert(formatTime(59_999) === "01:00", "minute boundary");
assert(formatTime(3_661_000) === "01:01:01", "hours");
assert(formatTime(150_000, true) === "00:02:30", "adjustment label");
assert(secondsFrom(splitTime(3_661_000)) === 3661, "duration round trip");
for (const fields of [
  { hours: 0, minutes: 0, seconds: 0 },
  { hours: 0, minutes: 60, seconds: 0 },
  { hours: -1, minutes: 0, seconds: 0 },
  { hours: 0, minutes: 0, seconds: 1.5 },
  { hours: undefined, minutes: 0, seconds: 1 },
  { hours: Number.MAX_SAFE_INTEGER, minutes: 0, seconds: 0 },
]) {
  let rejected = false;
  try { secondsFrom(fields); } catch { rejected = true; }
  assert(rejected, "invalid duration rejected");
}
console.log("Time formatting and input checks passed.");
