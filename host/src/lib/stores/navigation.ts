import { writable } from "svelte/store";
import { currentTab } from "$lib/stores/hostState";

export interface ActivityTarget {
  request: number;
  runId: string;
  grantId: string | null;
}

export type PermissionTarget =
  | { request: number; kind: "grant"; grantId: string }
  | { request: number; kind: "app"; appId: string };

export interface AppSettingsTarget {
  request: number;
  appId: string;
  displayName: string;
}

export interface ArtifactTarget {
  request: number;
  artifactId: string;
}

let nextRequest = 0;

export const activityTarget = writable<ActivityTarget | null>(null);
export const permissionTarget = writable<PermissionTarget | null>(null);
export const appSettingsTarget = writable<AppSettingsTarget | null>(null);
export const artifactTarget = writable<ArtifactTarget | null>(null);

export function openActivity(runId: string, grantId: string | null = null) {
  activityTarget.set({ request: ++nextRequest, runId, grantId });
  currentTab.set("system");
}

export function openPermission(grantId: string) {
  permissionTarget.set({ request: ++nextRequest, kind: "grant", grantId });
  currentTab.set("settings");
}

export function openAppPermissions(appId: string) {
  permissionTarget.set({ request: ++nextRequest, kind: "app", appId });
  currentTab.set("settings");
}

export function openAppSettings(appId: string, displayName: string) {
  appSettingsTarget.set({ request: ++nextRequest, appId, displayName });
  currentTab.set("settings");
}

export function openArtifact(artifactId: string) {
  artifactTarget.set({ request: ++nextRequest, artifactId });
  currentTab.set("stuff");
}
