import { useSyncExternalStore } from "react"

const storageKey = "downloader.playback"
const defaults = { volume: 0.1, muted: false }
type Playback = typeof defaults

function readPlayback(): Playback {
  try {
    const saved = JSON.parse(localStorage.getItem(storageKey) ?? "null")
    if (
      typeof saved?.volume === "number" &&
      Number.isFinite(saved.volume) &&
      saved.volume >= 0 &&
      saved.volume <= 1 &&
      typeof saved.muted === "boolean"
    ) {
      return { volume: saved.volume, muted: saved.muted }
    }
  } catch {
    // Playback remains available when storage is blocked or contains old data.
  }
  return defaults
}

let playback = readPlayback()
const listeners = new Set<() => void>()

function notify(next: Playback) {
  if (next.volume === playback.volume && next.muted === playback.muted) return
  playback = next
  listeners.forEach((listener) => listener())
}

function onStorage(event: StorageEvent) {
  if (event.key === storageKey || event.key === null) notify(readPlayback())
}

function subscribe(listener: () => void) {
  if (listeners.size === 0) window.addEventListener("storage", onStorage)
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) window.removeEventListener("storage", onStorage)
  }
}

export function setPlayback(volume: number, muted: boolean) {
  if (!Number.isFinite(volume)) return
  const next = { volume: Math.max(0, Math.min(1, volume)), muted }
  notify(next)
  try {
    localStorage.setItem(storageKey, JSON.stringify(next))
  } catch {
    // Keep the shared session setting even if it cannot be persisted.
  }
}

export function usePlayback() {
  return useSyncExternalStore(
    subscribe,
    () => playback,
    () => defaults
  )
}
