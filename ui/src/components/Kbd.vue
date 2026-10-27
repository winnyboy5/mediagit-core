<template>
  <kbd><slot /></kbd>
</template>

<style scoped>
kbd {
  display: inline-block;
  min-width: 22px;
  padding: 2px 6px;
  border: 1px solid var(--border-strong);
  border-bottom-width: 2px;
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text-muted);
  font: 500 var(--fs-xs) / 1.3 var(--font-mono);
  text-align: center;
}
</style>
