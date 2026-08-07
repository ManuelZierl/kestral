export function supportsRuntime(version: string): boolean {
  const match = /^(\d+)\.(\d+)\.(\d+)/.exec(version);
  if (!match) return false;
  const [, major, minor] = match.map(Number);
  return major > 22 || (major === 22 && minor >= 19);
}

export function assertSupportedRuntime(version = process.versions.node): void {
  if (!supportsRuntime(version)) throw new Error("provider worker requires Node.js 22.19.0 or newer");
}

export function clearAmbientEnvironment(
  environment: NodeJS.ProcessEnv = process.env,
  platform = process.platform,
): void {
  for (const name of Object.keys(environment)) {
    const isWindowsRuntimeVariable = platform === "win32" && ["systemroot", "windir"].includes(name.toLowerCase());
    if (!isWindowsRuntimeVariable) delete environment[name];
  }
}
