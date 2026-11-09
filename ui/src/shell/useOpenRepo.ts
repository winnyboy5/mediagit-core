import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { useRouter } from 'vue-router'
import { toApiError, type ApiError } from '@/api'
import { useRepoStore } from '@/stores/repo'

/** Ask the OS for a folder; null when cancelled. */
export async function pickFolder(title: string): Promise<string | null> {
  const dir = await openDialog({ directory: true, multiple: false, title })
  return typeof dir === 'string' ? dir : null
}

/** Open a repository (picker or known path) and land on Changes. Returns the error, if any. */
export function useOpenRepo() {
  const repo = useRepoStore()
  const router = useRouter()

  async function openPath(dir: string): Promise<ApiError | null> {
    if (await repo.open(dir)) {
      await router.push('/files/all')
      return null
    }
    return repo.error ?? { code: 'unknown', message: 'Could not open this folder.' }
  }

  async function pickAndOpen(): Promise<ApiError | null> {
    try {
      const dir = await pickFolder('Open MediaGit repository')
      return dir ? await openPath(dir) : null
    } catch (e) {
      return toApiError(e)
    }
  }

  return { openPath, pickAndOpen }
}
