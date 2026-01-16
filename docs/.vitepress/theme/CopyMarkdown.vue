<script setup lang="ts">
import { useData } from "vitepress";
import { ref } from "vue";

const { page } = useData();
const copied = ref(false);

async function copyMarkdown() {
  const base64 = (page.value as any).rawMarkdownBase64;
  if (base64) {
    const markdown = atob(base64);
    await navigator.clipboard.writeText(markdown);
    copied.value = true;
    setTimeout(() => (copied.value = false), 2000);
  }
}
</script>

<template>
  <div class="copy-markdown-wrapper">
    <button @click="copyMarkdown" class="copy-markdown-btn">
      {{ copied ? "Copied!" : "Copy page" }}
    </button>
  </div>
</template>
