---
layout: home

hero:
  text: Parallel development in tmux
  tagline: Giga opinionated zero-friction workflow tool for managing git worktrees and tmux windows as isolated development environments.
  image:
    light: /logo.svg
    dark: /logo-dark.svg
  actions:
    - theme: brand
      text: Quick Start
      link: /guide/quick-start
    - theme: alt
      text: Installation
      link: /guide/installation
    - theme: alt
      text: GitHub
      link: https://github.com/raine/workmux

features:
  - title: Zero friction
    details: Create worktrees and tmux windows in a single command. Merge and clean up everything just as easily.
  - title: AI agent native
    details: Perfect for running multiple AI agents in parallel. Isolated environments with status tracking in your tmux window list.
  - title: Config as code
    details: Define your tmux layout and setup steps in .workmux.yaml. Customize panes, file operations, and lifecycle hooks.
  - title: Native tmux integration
    details: Works with your existing tmux setup. Your shortcuts, themes, and workflow stay intact.
---

<div style="display: flex; justify-content: center; margin-top: 2rem;">
  <div class="video-container">
    <video src="/demo.mp4" controls muted playsinline preload="metadata"></video>
    <button type="button" class="video-play-button" aria-label="Play video"></button>
  </div>
</div>

<script setup>
import { onMounted } from 'vue'

onMounted(() => {
  const container = document.querySelector('.video-container')
  const video = container?.querySelector('video')
  const playBtn = container?.querySelector('.video-play-button')

  if (video && playBtn) {
    playBtn.addEventListener('click', () => {
      video.play()
      container.classList.add('playing')
    })

    video.addEventListener('pause', () => {
      container.classList.remove('playing')
    })

    video.addEventListener('play', () => {
      container.classList.add('playing')
    })
  }
})
</script>

<style>
.video-container {
  position: relative;
  border-radius: 8px;
  box-shadow: 0 4px 12px rgba(0,0,0,0.15);
  overflow: hidden;
  max-width: 100%;
}

.video-container video {
  display: block;
  max-width: 100%;
  cursor: pointer;
}

.video-play-button {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 80px;
  height: 80px;
  border: none;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.7);
  cursor: pointer;
  transition: background 0.2s, transform 0.2s;
}

.video-play-button::before {
  content: '';
  position: absolute;
  top: 50%;
  left: 55%;
  transform: translate(-50%, -50%);
  border-style: solid;
  border-width: 15px 0 15px 25px;
  border-color: transparent transparent transparent white;
}

.video-play-button:hover {
  background: rgba(0, 0, 0, 0.85);
  transform: translate(-50%, -50%) scale(1.1);
}

.video-container.playing .video-play-button {
  display: none;
}
</style>

## What people are saying

> "I've been using (and loving) workmux which brings together tmux, git worktrees, and CLI agents into an opinionated workflow."
> — @Coolin96 [via Hacker News](https://news.ycombinator.com/item?id=46029809)

> "Thank you so much for your work with workmux! It's a tool I've been wanting to exist for a long time."
> — @rstacruz [via GitHub](https://github.com/raine/workmux/issues/2)
