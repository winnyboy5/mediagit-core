<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  Copy,
  LogIn,
  Play,
  RefreshCw,
  Server,
  Square,
  TriangleAlert,
} from "lucide-vue-next";
import AlertDialog from "@/components/AlertDialog.vue";
import Button from "@/components/Button.vue";
import Checkbox from "@/components/Checkbox.vue";
import Dialog from "@/components/Dialog.vue";
import EmptyState from "@/components/EmptyState.vue";
import ErrorState from "@/components/ErrorState.vue";
import SkeletonRows from "@/components/SkeletonRows.vue";
import TextField from "@/components/TextField.vue";
import type { StorageOpts } from "@/api";
import { useServerStore } from "@/stores/server";
import FolderRow from "./FolderRow.vue";
import StorageFields from "./StorageFields.vue";
import { levelLabel } from "./roles";
import SignInDialog from "./SignInDialog.vue";

const server = useServerStore();
const l = computed(() => server.local);

// First run ------------------------------------------------------------------
const setupOpen = ref(false);
const admin = ref("");
const email = ref("");
const pass = ref("");
const again = ref("");
const storage = ref<StorageOpts>({ kind: "filesystem" });
const mismatch = computed(() =>
  again.value && again.value !== pass.value
    ? "The passwords do not match"
    : undefined,
);
const canSetUp = computed(
  () =>
    !!admin.value &&
    !!email.value &&
    !!pass.value &&
    pass.value === again.value &&
    !server.busy,
);
async function setUp() {
  if (!canSetUp.value) return;
  // The password is handed over and the fields emptied before anything else happens.
  const pw = pass.value;
  const st =
    storage.value.kind === "filesystem" ? undefined : { ...storage.value };
  pass.value = again.value = "";
  storage.value = { kind: "filesystem" };
  if (await server.setUp(admin.value, email.value, pw, st))
    setupOpen.value = false;
}
watch(setupOpen, (o) => {
  if (!o) {
    pass.value = again.value = "";
    storage.value = { kind: "filesystem" };
  }
});

// Network ----------------------------------------------------------------------
const lan = computed({
  get: () => !!l.value?.lan,
  set: (v) => void server.setNetwork(v === true),
});

// Stop / copy ----------------------------------------------------------------------
const stopOpen = ref(false);
const copied = ref(false);
async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    copied.value = true;
    setTimeout(() => (copied.value = false), 1500);
  } catch {
    copied.value = false;
  }
}

const signIn = ref(false);
const who = computed(() => server.me?.identity);
const STATE = {
  running: "Running",
  stopped: "Stopped",
  elsewhere: "Running, started elsewhere",
  not_set_up: "Not set up",
} as const;
</script>

<template>
  <div class="pane">
    <!-- This computer ------------------------------------------------------>
    <template v-if="server.isLocal">
      <ErrorState
        v-if="server.localError"
        :code="server.localError.code"
        :message="server.localError.message"
        @retry="server.refreshLocal()"
      />
      <SkeletonRows v-else-if="!l" :count="4" />
      <template v-else-if="l.state === 'not_set_up'">
        <EmptyState
          title="Run a server on this computer"
          description="Share projects with your team from here. MediaGit sets up a private server and its first admin account. Only this computer can reach it until you say otherwise."
        >
          <template #icon><Server :size="28" aria-hidden="true" /></template>
          <Button variant="primary" @click="setupOpen = true"
            >Set up local server…</Button
          >
        </EmptyState>
        <section class="card" aria-label="Server folder"><FolderRow /></section>
      </template>

      <template v-else>
        <section class="card" aria-labelledby="loc-h" data-testid="local-card">
          <h2 id="loc-h">Local server</h2>
          <p class="state" :class="l.state" data-testid="local-state">
            <span class="dot" aria-hidden="true" />{{ STATE[l.state] }}
          </p>
          <p v-if="l.state === 'elsewhere'" class="muted">
            Another MediaGit server is already answering on port {{ l.port }}.
            This app did not start it, so it will not stop it.
          </p>
          <dl class="kv">
            <dt>Address</dt>
            <dd class="inline">
              <span class="mono">{{ l.url }}</span>
              <Button
                v-if="l.state !== 'stopped'"
                size="sm"
                @click="copy(l.url!)"
                ><Copy :size="14" aria-hidden="true" />
                {{ copied ? "Copied" : "Copy" }}</Button
              >
            </dd>
            <dt>Who can connect</dt>
            <dd>
              {{
                l.lan ? "Other computers on your network" : "This computer only"
              }}
            </dd>
          </dl>
          <FolderRow />
          <div class="btns">
            <Button
              v-if="l.state === 'stopped'"
              variant="primary"
              :loading="server.busy"
              @click="server.start()"
              ><Play :size="16" aria-hidden="true" /> Start server</Button
            >
            <Button
              v-else-if="l.state === 'running'"
              :disabled="server.busy"
              @click="stopOpen = true"
              ><Square :size="16" aria-hidden="true" /> Stop server…</Button
            >
          </div>
          <Checkbox
            v-model="lan"
            label="Let other computers on my network connect"
            :disabled="l.state !== 'stopped' || server.busy"
          />
          <p class="muted small">
            {{
              l.state === "running"
                ? "Stop the server to change this."
                : "Takes effect the next time it starts. Sign-in is always required for this server."
            }}
          </p>
        </section>

        <section
          v-if="l.state === 'running' || l.state === 'elsewhere'"
          class="card"
          aria-labelledby="me-h"
          data-testid="me-card"
        >
          <h2 id="me-h">Your account here</h2>
          <p v-if="who" class="muted" data-testid="me-line">
            Signed in as <strong>{{ who.username }}</strong
            >. Your role: {{ levelLabel(who.role) }}.
          </p>
          <template v-else-if="server.access === 'signed_out'">
            <p class="muted">You are not signed in to this server yet.</p>
            <Button variant="primary" @click="signIn = true"
              ><LogIn :size="16" aria-hidden="true" /> Sign in</Button
            >
          </template>
          <p v-else class="muted">Checking…</p>
        </section>

        <section class="card" aria-labelledby="log-h" data-testid="log-card">
          <h2 id="log-h">What the server printed</h2>
          <pre
            v-if="server.log"
            class="log mono"
            tabindex="0"
            aria-label="Server log"
            >{{ server.log }}</pre>
          <p v-else class="muted">
            Nothing yet. The log fills in once the server starts.
          </p>
          <Button size="sm" @click="server.refreshLog()"
            ><RefreshCw :size="14" aria-hidden="true" /> Refresh</Button
          >
        </section>
      </template>
    </template>

    <!-- Another server ---------------------------------------------------->
    <template v-else>
      <section class="card" aria-labelledby="rem-h" data-testid="remote-card">
        <h2 id="rem-h">{{ server.target.label }}</h2>
        <dl class="kv">
          <dt>Address</dt>
          <dd class="inline">
            <span class="mono">{{ server.target.url }}</span>
            <Button size="sm" @click="copy(server.target.url!)"
              ><Copy :size="14" aria-hidden="true" />
              {{ copied ? "Copied" : "Copy" }}</Button
            >
          </dd>
          <dt>Sign-in</dt>
          <dd>
            {{ server.access === "no_accounts" ? "Not required" : "Required" }}
          </dd>
          <template v-if="who">
            <dt>You</dt>
            <dd data-testid="me-line">
              {{ who.username }}, {{ levelLabel(who.role) }}
            </dd>
          </template>
        </dl>
        <ErrorState
          v-if="server.meError"
          :code="server.meError.code"
          :message="server.meError.message"
          @retry="server.refreshMe()"
        />
        <Button
          v-else-if="server.access === 'signed_out'"
          variant="primary"
          @click="signIn = true"
          ><LogIn :size="16" aria-hidden="true" /> Sign in</Button
        >
        <p v-else-if="server.access === 'member'" class="muted">
          <TriangleAlert :size="14" aria-hidden="true" /> You can see your own
          keys. Managing people and repositories needs admin rights.
        </p>
      </section>
    </template>

    <Dialog
      v-model:open="setupOpen"
      title="Set up local server"
      description="Create the first admin account. You will use it to sign in to this server."
    >
      <form
        id="setup-form"
        class="form"
        autocomplete="off"
        @submit.prevent="setUp"
      >
        <TextField v-model="admin" label="Admin username" />
        <TextField v-model="email" label="Admin email" type="email" />
        <TextField v-model="pass" label="Password" type="password" />
        <TextField
          v-model="again"
          label="Repeat password"
          type="password"
          :error="mismatch"
        />
        <StorageFields v-model="storage" />
        <p v-if="server.actionError" class="err" role="alert">
          {{ server.actionError.message }}
        </p>
      </form>
      <template #footer>
        <Button @click="setupOpen = false">Cancel</Button>
        <Button
          type="submit"
          form="setup-form"
          variant="primary"
          :loading="server.busy"
          :disabled="!canSetUp"
          >Set up and start</Button
        >
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="stopOpen"
      title="Stop the local server?"
      description="People connected to it lose access until you start it again. Their files on their own computers are not affected."
      confirm-label="Stop server"
      destructive
      @confirm="server.stop()"
    />
    <SignInDialog v-model:open="signIn" />
  </div>
</template>

<style scoped src="./panel.css"></style>
<style scoped>
.state {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  font-weight: 600;
}
.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  border: 2px solid var(--text-muted);
}
.state.running .dot {
  border-color: var(--success);
  background: var(--success);
}
.state.elsewhere .dot {
  border-color: var(--modified);
  background: var(--modified);
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-2) var(--space-5);
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
.inline {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.small {
  font-size: var(--fs-sm);
}
.log {
  box-sizing: border-box;
  width: 100%;
  max-height: 240px;
  margin: 0;
  padding: var(--space-3) var(--space-4);
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--surface-2);
  font-size: var(--fs-sm);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.err {
  margin: 0;
  color: var(--danger);
}
</style>
