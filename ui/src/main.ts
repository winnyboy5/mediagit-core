import { createApp } from 'vue'
import { createPinia } from 'pinia'
import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-sans/500.css'
import '@fontsource/ibm-plex-sans/600.css'
import '@fontsource/ibm-plex-mono/400.css'
import './styles/tokens.css'
import './styles/base.css'
import Root from './Root.vue'
import { router } from './router'
import { setStillLockedHandler, setUnlockHandler } from './api'
import { useLookStore } from './stores/settings'
import { useUnlockStore } from './stores/unlock'

const pinia = createPinia()
// Apply the saved theme and density before anything is shown.
useLookStore(pinia)
// An encrypted repository that needs its passphrase asks for it, whichever view hit it.
setUnlockHandler((repo) => useUnlockStore(pinia).unlockRepo(repo))
setStillLockedHandler((repo) => useUnlockStore(pinia).markLocked(repo))
createApp(Root).use(pinia).use(router).mount('#app')
