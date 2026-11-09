<script setup lang="ts">
// "Save a version": the dock under both Files sections. Included changes become the next version.
import { computed } from 'vue'
import Button from '@/components/Button.vue'
import Kbd from '@/components/Kbd.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import TextField from '@/components/TextField.vue'
import Textarea from '@/components/Textarea.vue'
import { useChangesStore } from '@/stores/changes'

const changes = useChangesStore()
const busy = computed(() => changes.busy !== null)
const canCommit = computed(() => changes.canSave && (!changes.needsMessage || changes.message.trim().length > 0) && !busy.value)
const hint = computed(() =>
  !changes.canSave ? 'Include at least one change to save a version.' : changes.needsMessage && !changes.message.trim() ? 'Describe what changed to save.' : '',
)
</script>

<template>
  <form class="dock" data-testid="save-dock" @submit.prevent="changes.commit()">
    <div class="top">
      <h2>Save a version</h2>
      <p class="count" data-testid="include-count">
        {{ changes.includedCount }} of {{ changes.list.length }} {{ changes.list.length === 1 ? 'change' : 'changes' }} included
        <button v-if="changes.includedCount < changes.list.length" type="button" class="link" :disabled="busy" @click="changes.stageAll()">Include all</button>
        <button v-if="changes.includedCount" type="button" class="link" :disabled="busy" @click="changes.unstageAll()">Exclude all</button>
      </p>
    </div>
    <Textarea
      v-model="changes.message"
      label="Message"
      placeholder="What changed? For example: warmer ground texture on the dragon"
      :rows="2"
      :disabled="busy"
      @keydown.ctrl.enter.prevent="changes.commit()"
    />
    <MoreOptions data-testid="commit-more">
      <TextField v-model="changes.commitForm.messageFile" label="Take the description from a file" hint="Full path of a text file. The box above is then not needed." flag="--file" mono />
      <OptionCheck v-model="changes.commitForm.all" label="Stage every changed tracked file first" flag="--all" />
      <label class="inc">
        <span>Also stage these files</span>
        <textarea v-model="changes.commitForm.include" rows="2" placeholder="e.g. art/hero.psd" aria-describedby="inc-hint" class="mono"></textarea>
        <span id="inc-hint" class="hint">One path per line. <code class="mono flag">--include</code></span>
      </label>
      <TextField v-model="changes.commitForm.author" label="Author" hint="Name &lt;email&gt;, instead of your configured identity." flag="--author" placeholder="e.g. Ana Ruiz <ana@studio.example>" mono />
      <TextField v-model="changes.commitForm.date" label="Date" hint="Like 2026-01-15T10:30:00Z." flag="--date" placeholder="e.g. 2026-01-15T10:30:00Z" mono />
      <OptionCheck v-model="changes.commitForm.allowEmpty" label="Allow a version with no changes" flag="--allow-empty" />
      <OptionCheck v-model="changes.commitForm.signoff" label="Add a Signed-off-by line" flag="--signoff" />
      <OptionCheck v-model="changes.commitForm.dryRun" label="Preview only" hint="Show what would be saved; nothing is." flag="--dry-run" />
    </MoreOptions>
    <p v-if="changes.preview" class="preview" role="status" data-testid="preview-note">{{ changes.preview }}</p>
    <div class="row">
      <p class="hint"><template v-if="hint">{{ hint }}</template><template v-else><Kbd>Ctrl</Kbd> + <Kbd>Enter</Kbd> to save</template></p>
      <Button type="submit" variant="primary" :disabled="!canCommit" :loading="changes.busy === 'commit'">Save version</Button>
    </div>
  </form>
</template>

<style scoped>
.dock {
  display: grid;
  flex: none;
  gap: var(--space-3);
  max-height: 60%;
  padding: var(--space-4) var(--space-6);
  overflow-y: auto;
  border-top: 1px solid var(--border);
  background: var(--surface);
}
.top {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-1) var(--space-4);
}
h2 {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}
.count {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-md);
}
.link {
  margin-left: var(--space-2);
  padding: 0;
  border: 0;
  border-bottom: 2px solid var(--accent);
  background: none;
  color: var(--text);
  font: 600 var(--fs-md) / 1.3 var(--font-sans);
  cursor: pointer;
}
.link:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}
.preview {
  margin: 0;
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
.inc {
  display: grid;
  gap: var(--space-2);
  font-size: var(--fs-sm);
  font-weight: 500;
}
.inc textarea {
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-family: var(--font-mono);
}
.inc .hint {
  color: var(--text-muted);
  font-weight: 400;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}
.hint {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
