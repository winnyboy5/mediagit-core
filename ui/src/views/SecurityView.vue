<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { Copy, KeyRound, Lock, LockOpen, LogIn, ShieldCheck } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import ErrorState from '@/components/ErrorState.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import { useRepoStore } from '@/stores/repo'
import { useSecurityStore } from '@/stores/security'
import { useUnlockStore } from '@/stores/unlock'

const sec = useSecurityStore()
const repo = useRepoStore()
const unlock = useUnlockStore()
const isUnlocked = computed(() => !!repo.path && unlock.unlocked.has(repo.path))
onMounted(() => void sec.refresh())

const a = computed(() => sec.auth)
const k = computed(() => sec.key)
const loading = computed(() => sec.loading && !a.value && !k.value && !sec.authError && !sec.keyError)

// Sign in -----------------------------------------------------------------
const loginOpen = ref(false)
const server = ref('')
const username = ref('')
const password = ref('')
const serverError = computed(() => (server.value && !/^https?:\/\/[^\s/]+/.test(server.value) ? 'Use an http(s) address' : undefined))
const canLogin = computed(() => !!password.value && !serverError.value && !sec.busy)
async function login() {
  if (!canLogin.value) return
  // The password is handed over and the field emptied before anything else happens.
  const pw = password.value
  password.value = ''
  loginOpen.value = false
  await sec.login(server.value, username.value, pw)
}
const signOutOpen = ref(false)

// Create account (`auth register`) -------------------------------------------
const regOpen = ref(false)
const regServer = ref('')
const regUser = ref('')
const regEmail = ref('')
const regPass = ref('')
const regAgain = ref('')
const regServerError = computed(() => (regServer.value && !/^https?:\/\/[^\s/]+/.test(regServer.value) ? 'Use an http(s) address' : undefined))
const regMismatch = computed(() => (regAgain.value && regAgain.value !== regPass.value ? 'The passwords do not match' : undefined))
const canRegister = computed(() => !!regUser.value && !!regEmail.value && !!regPass.value && regPass.value === regAgain.value && !regServerError.value && !sec.busy)
async function register() {
  if (!canRegister.value) return
  const pw = regPass.value
  regPass.value = regAgain.value = ''
  regOpen.value = false
  await sec.register(regServer.value, regUser.value, regEmail.value, pw)
}

// Change password (`auth passwd`) ---------------------------------------------
const pwOpen = ref(false)
const pwServer = ref('')
const pwCurrent = ref('')
const pwNew = ref('')
const pwAgain = ref('')
const pwServerError = computed(() => (pwServer.value && !/^https?:\/\/[^\s/]+/.test(pwServer.value) ? 'Use an http(s) address' : undefined))
const pwMismatch = computed(() => (pwAgain.value && pwAgain.value !== pwNew.value ? 'The passwords do not match' : undefined))
const canChangePw = computed(() => !!pwCurrent.value && !!pwNew.value && pwNew.value === pwAgain.value && !pwServerError.value && !sec.busy)
async function changePassword() {
  if (!canChangePw.value) return
  const [cur, next] = [pwCurrent.value, pwNew.value]
  pwCurrent.value = pwNew.value = pwAgain.value = ''
  pwOpen.value = false
  await sec.changePassword(pwServer.value, cur, next)
}

// Replace the master key (`key rotate-master`) -------------------------------------
const rotOpen = ref(false)
const rotMode = ref('passphrase')
const rotKeyfile = ref('')
const rotCurrent = ref('')
const rotNew = ref('')
const currentIsPassphrase = computed(() => /passphrase/i.test(sec.key?.source ?? ''))
const canRotate = computed(() => (rotMode.value === 'keyfile' ? !!rotKeyfile.value.trim() : !!rotNew.value) && (!currentIsPassphrase.value || !!rotCurrent.value) && !sec.busy)
async function rotate() {
  if (!canRotate.value) return
  const keyfile = rotMode.value === 'keyfile' ? rotKeyfile.value : ''
  const next = rotMode.value === 'passphrase' ? rotNew.value : ''
  const cur = currentIsPassphrase.value ? rotCurrent.value : ''
  rotNew.value = rotCurrent.value = ''
  rotOpen.value = false
  await sec.rotateMaster(keyfile, cur, next)
}

// Encryption --------------------------------------------------------------
const initOpen = ref(false)
const initPass = ref('')
async function enable() {
  const p = initPass.value
  initPass.value = ''
  initOpen.value = false
  await sec.enableEncryption(p)
}
const recoverOpen = ref(false)
const code = ref('')
const recoverPass = ref('')
async function recover() {
  if (!code.value || !recoverPass.value) return
  const c = code.value
  const p = recoverPass.value
  code.value = recoverPass.value = ''
  recoverOpen.value = false
  await sec.recover(c, p)
}

// Whatever was typed is gone when its dialog closes.
watch(loginOpen, (o) => !o && (password.value = ''))
watch(initOpen, (o) => !o && (initPass.value = ''))
watch(regOpen, (o) => !o && (regPass.value = regAgain.value = ''))
watch(pwOpen, (o) => !o && (pwCurrent.value = pwNew.value = pwAgain.value = ''))
watch(rotOpen, (o) => !o && (rotNew.value = rotCurrent.value = ''))
watch(recoverOpen, (o) => !o && (code.value = recoverPass.value = ''))

// The one-time recovery code ------------------------------------------------
const saved = ref<boolean | 'indeterminate'>(false)
const copied = ref(false)
const showCode = computed({
  get: () => sec.recoveryCode !== null,
  // Only "I saved it" closes it: no Escape, no click outside, no X.
  set: () => {},
})
watch(showCode, (o) => o && ((saved.value = false), (copied.value = false)))
async function copy() {
  try {
    await navigator.clipboard.writeText(sec.recoveryCode ?? '')
    copied.value = true
  } catch {
    copied.value = false
  }
}
function done() {
  copied.value = false
  sec.dismissRecovery()
}

const sourceText = (s: string | null) => s ?? 'No key available on this machine'
</script>

<template>
  <section class="sec" :aria-busy="sec.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">Security</h2>
      </div>
    </header>

    <div v-if="sec.actionError" class="banner" role="alert">
      <code class="mono">{{ sec.actionError.code }}</code>
      <span>{{ sec.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="sec.actionError = null">Dismiss</Button>
    </div>

    <div v-if="loading" class="pad"><SkeletonRows :count="5" /></div>
    <div v-else class="grid">
      <section class="card" aria-labelledby="acct-h" data-testid="account">
        <h2 id="acct-h">Account</h2>
        <ErrorState v-if="sec.authError" :code="sec.authError.code" :message="sec.authError.message" @retry="sec.refresh()" />
        <template v-else-if="a">
          <p v-if="!a.auth_enabled" class="muted">This server does not ask for sign-in.</p>
          <template v-else-if="a.logged_in && a.identity">
            <p class="who"><ShieldCheck :size="18" aria-hidden="true" /> Signed in as <strong>{{ a.identity.username }}</strong></p>
            <dl class="kv">
              <dt>Email</dt><dd>{{ a.identity.email }}</dd>
              <dt>Role</dt><dd>{{ a.identity.role }}</dd>
              <dt>Server</dt><dd class="mono">{{ a.server }}</dd>
              <dt>Credential</dt><dd>{{ a.credential_tier }}</dd>
            </dl>
            <ul v-if="a.identity.grants.length" class="grants" aria-label="Projects you can reach">
              <li v-for="g in a.identity.grants" :key="g.repo"><span class="mono">{{ g.repo }}</span> <span class="chip">{{ g.level }}</span></li>
            </ul>
            <div class="btns">
              <Button :disabled="sec.busy" @click="pwOpen = true">Change password…</Button>
              <Button variant="danger" :disabled="sec.busy" @click="signOutOpen = true">Sign out…</Button>
            </div>
          </template>
          <template v-else>
            <p class="muted">You are not signed in{{ a.server ? ` to ${a.server}` : '' }}.</p>
            <div class="btns">
              <Button variant="primary" :disabled="sec.busy" @click="loginOpen = true"><LogIn :size="16" aria-hidden="true" /> Sign in</Button>
              <Button :disabled="sec.busy" @click="regOpen = true">Create account…</Button>
            </div>
          </template>
        </template>
      </section>

      <section class="card" aria-labelledby="enc-h" data-testid="encryption">
        <h2 id="enc-h">Encryption</h2>
        <ErrorState v-if="sec.keyError" :code="sec.keyError.code" :message="sec.keyError.message" @retry="sec.refresh()" />
        <template v-else-if="k">
          <template v-if="k.enabled">
            <p class="who"><KeyRound :size="18" aria-hidden="true" /> This repository is encrypted.</p>
            <dl class="kv">
              <dt>Unlocks with</dt><dd>{{ sourceText(k.source) }}</dd>
              <dt>Recovery code</dt><dd>{{ k.recovery_slot ? 'Saved' : 'Not set up' }}</dd>
            </dl>
            <p v-if="!k.source" class="muted">This machine has no key for it, so its files cannot be read here. Recover it with your recovery code.</p>
            <p v-if="isUnlocked" class="who" data-testid="unlocked"><LockOpen :size="18" aria-hidden="true" /> Unlocked for this session. MediaGit keeps the passphrase in memory only.</p>
            <div class="btns">
              <Button v-if="isUnlocked" @click="unlock.lock(repo.path!)"><Lock :size="16" aria-hidden="true" /> Lock now</Button>
              <Button :disabled="sec.busy" @click="recoverOpen = true">Recover with a code…</Button>
              <Button :disabled="sec.busy || !k.source" @click="rotOpen = true">Replace master key…</Button>
            </div>
          </template>
          <template v-else>
            <p class="muted">Files in this repository are stored as they are. Turn on encryption to protect new data at rest.</p>
            <Button :disabled="sec.busy" @click="initOpen = true">Turn on encryption…</Button>
          </template>
        </template>
      </section>
    </div>

    <Dialog v-model:open="loginOpen" title="Sign in" description="Sign in to a MediaGit server to share this project.">
      <form id="login-form" class="form" autocomplete="off" @submit.prevent="login">
        <TextField v-model="server" label="Server address (optional)" placeholder="https://mediagit.studio.example" hint="Leave empty to use this repository's remote" :error="serverError" />
        <TextField v-model="username" label="Username or email" />
        <TextField v-model="password" label="Password" type="password" />
      </form>
      <template #footer>
        <Button @click="loginOpen = false">Cancel</Button>
        <Button type="submit" form="login-form" variant="primary" :disabled="!canLogin">Sign in</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="regOpen" title="Create account" description="Make an account on a MediaGit server that allows sign-up. Then sign in with it.">
      <form id="reg-form" class="form" autocomplete="off" @submit.prevent="register">
        <TextField v-model="regServer" label="Server address (optional)" placeholder="https://mediagit.studio.example" hint="Leave empty to use this repository's remote (--server)" :error="regServerError" />
        <TextField v-model="regUser" label="Username" />
        <TextField v-model="regEmail" label="Email" type="email" />
        <TextField v-model="regPass" label="Password" type="password" />
        <TextField v-model="regAgain" label="Repeat password" type="password" :error="regMismatch" />
      </form>
      <template #footer>
        <Button @click="regOpen = false">Cancel</Button>
        <Button type="submit" form="reg-form" variant="primary" :disabled="!canRegister">Create account</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="pwOpen" title="Change password" description="Your old password stops working at once on that server.">
      <form id="pw-form" class="form" autocomplete="off" @submit.prevent="changePassword">
        <TextField v-model="pwServer" label="Server address (optional)" placeholder="https://mediagit.studio.example" hint="Leave empty to use this repository's remote (--server)" :error="pwServerError" />
        <TextField v-model="pwCurrent" label="Current password" type="password" />
        <TextField v-model="pwNew" label="New password" type="password" />
        <TextField v-model="pwAgain" label="Repeat new password" type="password" :error="pwMismatch" />
      </form>
      <template #footer>
        <Button @click="pwOpen = false">Cancel</Button>
        <Button type="submit" form="pw-form" variant="primary" :disabled="!canChangePw">Change password</Button>
      </template>
    </Dialog>

    <Dialog
      v-model:open="rotOpen"
      title="Replace the master key"
      description="The master key protects this repository's own key. Replacing it rewrites no files and your recovery code keeps working. It does not re-encrypt the data: if the old master key leaked together with a copy of the files, those files stay readable by it."
    >
      <form id="rot-form" class="form" autocomplete="off" @submit.prevent="rotate">
        <SegmentedControl
          v-model="rotMode"
          label="New master key"
          :options="[{ value: 'passphrase', label: 'A passphrase' }, { value: 'keyfile', label: 'A key file' }]"
        />
        <TextField v-if="currentIsPassphrase" v-model="rotCurrent" label="Current passphrase" type="password" />
        <TextField v-if="rotMode === 'passphrase'" v-model="rotNew" label="New passphrase" type="password" hint="Write it down: without it and your recovery code the data cannot be read." />
        <TextField v-else v-model="rotKeyfile" label="Key file" placeholder="C:\Keys\hero-film.key" hint="Full path to the new key file (--new-keyfile). It must differ from the current one." mono />
      </form>
      <template #footer>
        <Button @click="rotOpen = false">Cancel</Button>
        <Button type="submit" form="rot-form" variant="primary" :disabled="!canRotate">Replace master key</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="signOutOpen"
      title="Sign out?"
      description="The saved sign-in for this server is removed from this computer. You can sign in again any time."
      confirm-label="Sign out"
      destructive
      @confirm="sec.logout(false)"
    />

    <Dialog v-model:open="initOpen" title="Turn on encryption" description="New data in this repository is encrypted. You get a recovery code once, and it is the only way back in if this computer's key is lost.">
      <form id="init-form" class="form" autocomplete="off" @submit.prevent="enable">
        <TextField v-model="initPass" label="Passphrase (optional)" type="password" hint="Protects the key on this computer. You will be asked for it when needed." />
      </form>
      <template #footer>
        <Button @click="initOpen = false">Cancel</Button>
        <Button type="submit" form="init-form" variant="primary" :disabled="sec.busy">Turn on encryption</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="recoverOpen" title="Recover the key" description="Enter the recovery code you saved when you turned encryption on.">
      <form id="recover-form" class="form" autocomplete="off" @submit.prevent="recover">
        <TextField v-model="code" label="Recovery code" type="password" mono />
        <TextField v-model="recoverPass" label="New passphrase" type="password" hint="Protects the recovered key on this computer. Required." />
      </form>
      <template #footer>
        <Button @click="recoverOpen = false">Cancel</Button>
        <Button type="submit" form="recover-form" variant="primary" :disabled="!code || !recoverPass || sec.busy">Recover</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="showCode" :dismissable="false" title="Save your recovery code" description="This is the only time it is shown. MediaGit does not keep it. Store it somewhere safe, like a password manager.">
      <div class="code" data-testid="recovery-code"><code class="mono">{{ sec.recoveryCode }}</code></div>
      <p v-if="sec.keySource" class="muted" data-testid="key-source">On this computer the key unlocks with: {{ sec.keySource }}.</p>
      <div class="row">
        <Button @click="copy"><Copy :size="16" aria-hidden="true" /> {{ copied ? 'Copied' : 'Copy code' }}</Button>
      </div>
      <Checkbox v-model="saved" label="I saved it" />
      <template #footer>
        <Button variant="primary" :disabled="saved !== true" @click="done">Done</Button>
      </template>
    </Dialog>
  </section>
</template>

<style scoped>
.sec {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
  overflow-y: auto;
}
.head {
  display: flex;
  align-items: center;
}
.ttl {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}
.pt {
  margin: 0;
  font-size: var(--fs-xl);
}
h2 {
  margin: 0;
  font-size: var(--fs-lg);
}
.ttl p,
.muted {
  margin: 0;
  color: var(--text-muted);
}
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.banner span {
  flex: 1;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
  gap: var(--space-4);
  align-items: start;
}
.card {
  display: grid;
  gap: var(--space-3);
  justify-items: start;
  padding: var(--space-4) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.card > :deep(*) {
  max-width: 100%;
}
.who {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
  margin: 0;
  font-size: var(--fs-sm);
}
.kv dt {
  color: var(--text-muted);
}
.kv dd {
  margin: 0;
  overflow-wrap: anywhere;
}
.grants {
  display: grid;
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: var(--fs-sm);
}
.chip {
  padding: 2px var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.form {
  display: grid;
  gap: var(--space-3);
}
.code {
  padding: var(--space-4);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface-2);
  font-size: var(--fs-lg);
  overflow-wrap: anywhere;
  user-select: all;
}
.btns {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.row {
  display: flex;
  margin: var(--space-3) 0;
}
</style>
