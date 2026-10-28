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
import { useLookStore } from './stores/settings'

const pinia = createPinia()
// Apply the saved theme and density before anything is shown.
useLookStore(pinia)
createApp(Root).use(pinia).use(router).mount('#app')
