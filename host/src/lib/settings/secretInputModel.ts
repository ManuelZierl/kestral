export type SecretStatus = "checking" | "not-set" | "set" | "updated-now" | "error";

export function secretStatusFromPresence(present: boolean): SecretStatus {
  return present ? "set" : "not-set";
}

export function secretStatusAfterSave(present: boolean): SecretStatus {
  return present ? "updated-now" : "error";
}

export function secretStatusAfterClear(present: boolean): SecretStatus {
  return present ? "error" : "not-set";
}

export function secretStatusLabel(status: SecretStatus): string {
  switch (status) {
    case "set":
      return "Set";
    case "updated-now":
      return "Updated just now";
    case "not-set":
      return "Not set";
    case "error":
      return "Error";
    case "checking":
      return "Checking...";
  }
}

export function secretInputPlaceholder(status: SecretStatus): string {
  return status === "set" || status === "updated-now" ? "Secret is set" : "Enter secret";
}
