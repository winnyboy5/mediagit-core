// localStorage can be missing or throw (private windows, blocked storage):
// every caller must work without it.
export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw == null ? fallback : (JSON.parse(raw) as T)
  } catch {
    return fallback
  }
}

export function save(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // Not persisting is acceptable; the app state in memory is still right.
  }
}

export function forget(key: string): void {
  try {
    localStorage.removeItem(key)
  } catch {
    // see save()
  }
}

/** Last path segment of a folder path, either slash style. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}

/** "2h ago" style label; `now` is injectable for tests. */
export function timeAgo(then: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - then) / 1000))
  if (s < 60) return 'just now'
  const units: [number, string][] = [
    [60, 'm'],
    [3600, 'h'],
    [86400, 'd'],
    [604800, 'w'],
    [2592000, 'mo'],
  ]
  let label = `${Math.floor(s / 2592000)}mo`
  for (let i = 0; i < units.length; i++) {
    const [size, suffix] = units[i]!
    const next = units[i + 1]?.[0]
    if (next === undefined || s < next) {
      label = `${Math.floor(s / size)}${suffix}`
      break
    }
  }
  return `${label} ago`
}
