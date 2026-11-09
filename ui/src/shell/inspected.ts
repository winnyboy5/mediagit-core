import { computed } from 'vue'
import { useRoute } from 'vue-router'
import type { InspectFile } from '@/lib/inspect'
import { useChangesStore } from '@/stores/changes'
import { useFilesStore } from '@/stores/files'
import { useHistoryStore } from '@/stores/history'

/** The files the details panel describes: History shows one file of a version, Files the picked rows. */
export function useInspectedFiles() {
  const route = useRoute()
  const changes = useChangesStore()
  const files = useFilesStore()
  const history = useHistoryStore()
  return computed<InspectFile[]>(() => {
    if (route.meta.hub === 'history') return history.inspectFile ? [history.inspectFile] : []
    const picked = route.params.section === 'all' ? files.selectedFiles : changes.selectedFiles
    return picked.map((f) => ({ ...f, rev: null }))
  })
}
