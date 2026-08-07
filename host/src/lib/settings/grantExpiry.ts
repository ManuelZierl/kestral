// Grant expiry entered as a value + unit instead of raw seconds.
//
// People reason in minutes/hours/days, not "604800 seconds". These helpers
// convert between the human value+unit and the seconds the grant model stores,
// and back again when an existing grant is edited. Kept pure for unit testing.

export type ExpiryUnit = "minutes" | "hours" | "days";

const FACTOR: Record<ExpiryUnit, number> = {
  minutes: 60,
  hours: 3600,
  days: 86400,
};

// Matches the kernel's NonZeroU32 seconds ceiling.
const MAX_SECONDS = 4_294_967_295;

export type ExpiryParse =
  | { kind: "never" }
  | { kind: "seconds"; seconds: number }
  | { kind: "invalid" };

/** Interpret a value+unit. Empty value means "never expires". */
export function parseExpiry(value: string | number | null | undefined, unit: ExpiryUnit): ExpiryParse {
  const trimmed = value == null ? "" : String(value).trim();
  if (trimmed === "") return { kind: "never" };
  const amount = Number(trimmed);
  if (!Number.isInteger(amount) || amount < 1) return { kind: "invalid" };
  const seconds = amount * FACTOR[unit];
  if (seconds > MAX_SECONDS) return { kind: "invalid" };
  return { kind: "seconds", seconds };
}

/** Present a span of seconds as the largest whole unit that fits it. */
export function secondsToExpiry(seconds: number): { value: string; unit: ExpiryUnit } {
  for (const unit of ["days", "hours", "minutes"] as ExpiryUnit[]) {
    if (seconds % FACTOR[unit] === 0) {
      return { value: String(seconds / FACTOR[unit]), unit };
    }
  }
  // Not evenly divisible by a minute — round up so the shown value never
  // understates the real expiry.
  return { value: String(Math.max(1, Math.ceil(seconds / 60))), unit: "minutes" };
}
