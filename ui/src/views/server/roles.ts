/** Plain words first, the server's term second. */
export const LEVELS = [
  { value: 'read', label: 'Can look (read)' },
  { value: 'write', label: 'Can change files (write)' },
  { value: 'admin', label: 'Can manage (admin)' },
]

/** The full label: the plain words and the term. */
export const levelLabel = (v: string) => LEVELS.find((l) => l.value === v)?.label ?? v
