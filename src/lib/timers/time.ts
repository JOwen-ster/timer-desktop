export type TimeFields = { hours: number | undefined; minutes: number | undefined; seconds: number | undefined };
export type ParsedTime = { seconds: number; error: null } | { seconds: null; error: string };

export function splitTime(milliseconds: number): TimeFields {
  const seconds = Math.floor(milliseconds / 1000);
  return { hours: Math.floor(seconds / 3600), minutes: Math.floor(seconds / 60) % 60, seconds: seconds % 60 };
}

export function parseTime(fields: TimeFields): ParsedTime {
  const { hours = 0, minutes = 0, seconds = 0 } = fields;
  if ([hours, minutes, seconds].some(value => !Number.isSafeInteger(value) || value < 0)
    || minutes > 59 || seconds > 59) {
    return { seconds: null, error: "Use whole numbers: hours ≥ 0, minutes and seconds from 0 to 59." };
  }
  const total = hours * 3600 + minutes * 60 + seconds;
  if (total <= 0 || !Number.isSafeInteger(total * 1000)) {
    return { seconds: null, error: "Enter a positive time within the supported range." };
  }
  return { seconds: total, error: null };
}

export function formatTime(milliseconds: number, alwaysHours = false): string {
  const total = Math.max(0, Math.ceil(milliseconds / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = total % 60;
  return [ ...(hours > 0 || alwaysHours ? [hours] : []), minutes, seconds ]
    .map(value => String(value).padStart(2, "0")).join(":");
}
