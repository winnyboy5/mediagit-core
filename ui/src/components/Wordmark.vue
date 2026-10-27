<script setup lang="ts">
import light from '@/assets/logo-wordmark.svg'
import dark from '@/assets/logo-wordmark-dark.svg'

withDefaults(defineProps<{ height?: number }>(), { height: 32 })
</script>

<template>
  <span class="wordmark" role="img" aria-label="MediaGit">
    <img class="on-light" :src="light" alt="" :height="height" />
    <img class="on-dark" :src="dark" alt="" :height="height" />
  </span>
</template>

<style scoped>
.wordmark {
  display: inline-flex;
}
img {
  display: block;
  width: auto;
}
.on-dark {
  display: none;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme='light']) .on-light {
    display: none;
  }
  :root:not([data-theme='light']) .on-dark {
    display: block;
  }
}
:root[data-theme='dark'] .on-light {
  display: none;
}
:root[data-theme='dark'] .on-dark {
  display: block;
}
</style>
