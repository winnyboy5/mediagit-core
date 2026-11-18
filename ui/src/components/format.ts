const UNITS = ['B', 'KB', 'MB', 'GB', 'TB']

// Decimal units (1 GB = 1,000,000,000 bytes), like a cloud bill; the command line says GiB for binary.
export function formatBytes(n: number): string {
  let v = n
  let i = 0
  while (v >= 1000 && i < UNITS.length - 1) {
    v /= 1000
    i++
  }
  return `${i === 0 || v >= 100 ? Math.round(v) : v.toFixed(1)} ${UNITS[i]}`
}

export function formatRate(bytesPerSec: number | null | undefined): string {
  return bytesPerSec == null ? '--' : `${formatBytes(bytesPerSec)}/s`
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds == null) return '--'
  const s = Math.round(seconds)
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`
}
