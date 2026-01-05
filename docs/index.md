---
layout: home

hero:
  text: Parallel development in tmux with git worktrees
  tagline: The zero-friction workflow for git worktrees and tmux. Isolate contexts, run parallel AI agents, and merge with a single command.
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
  - title: Worktrees made simple
    details: Create a git worktree, tmux window, and environment setup in one command. Context switching is instant.
  - title: Parallel AI agents
    details: The missing link for AI coding. Delegate tasks to multiple agents simultaneously in isolated environments.
  - title: Native tmux
    details: Tmux is the interface. No new TUI to learn. Just tmux windows you already know how to use.
  - title: Config as code
    details: Define your tmux layout and setup steps in .workmux.yaml. Customize panes, file operations, and lifecycle hooks.
---

<div class="why-section">
  <h2>Why workmux?</h2>
  <p>
    The core principle is that <strong>tmux is the interface</strong>.
    If you already live in tmux, you shouldn't need yet another interface to manage your tasks.
    workmux turns multi-step git worktree operations into simple commands,
    making parallel workflows practical.
  </p>
</div>

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
.why-section {
  max-width: 800px;
  margin: 5rem auto;
  text-align: center;
  padding: 0 1.5rem;
}

.why-section h2 {
  border: none;
  margin: 0 0 1.5rem;
  padding: 0;
  font-weight: 700;
  font-size: 1.75rem;
}

.why-section p {
  font-size: 1.2rem;
  line-height: 1.8;
  color: var(--vp-c-text-2);
  margin: 0;
}

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

.testimonials-section {
  max-width: 900px;
  margin: 3rem auto 0;
  padding: 0 24px;
}

.testimonials-section h2 {
  text-align: center;
  font-size: 1.5rem;
  font-weight: 600;
  margin-bottom: 1.5rem;
  color: var(--vp-c-text-1);
}

.testimonials {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 1.25rem;
}

.testimonial {
  background: var(--vp-c-bg-soft);
  border-radius: 12px;
  padding: 1.5rem;
  border: 1px solid var(--vp-c-divider);
}

.testimonial-quote {
  font-size: 0.95rem;
  line-height: 1.6;
  color: var(--vp-c-text-1);
  margin: 0 0 1rem 0;
  font-style: italic;
}

.testimonial-author {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.875rem;
  color: var(--vp-c-text-2);
}

.testimonial-author a {
  color: var(--vp-c-brand-1);
  text-decoration: none;
}

.testimonial-author a:hover {
  text-decoration: underline;
}
</style>

<div class="testimonials-section">
  <h2>What people are saying</h2>
  <div class="testimonials">
    <div class="testimonial">
      <p class="testimonial-quote">"I've been using (and loving) workmux which brings together tmux, git worktrees, and CLI agents into an opinionated workflow."</p>
      <div class="testimonial-author">
        — @Coolin96 <a href="https://news.ycombinator.com/item?id=46029809">via Hacker News</a>
      </div>
    </div>
    <div class="testimonial">
      <p class="testimonial-quote">"Thank you so much for your work with workmux! It's a tool I've been wanting to exist for a long time."</p>
      <div class="testimonial-author">
        — @rstacruz <a href="https://github.com/raine/workmux/issues/2">via GitHub</a>
      </div>
    </div>
  </div>
</div>
