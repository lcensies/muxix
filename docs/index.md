---
layout: home
description: The zero-friction workflow for git worktrees and tmux

hero:
  text: Parallel development in tmux with git worktrees
  tagline: The zero-friction workflow for git worktrees and tmux. Isolate contexts, run parallel AI agents, and merge with a single command.
  image:
    light: /logo.svg
    dark: /logo-dark.svg
  actions:
    - theme: brand
      text: Quick start
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

<div class="code-snippet">

```bash
# Start working on a feature
workmux add my-feature

# Done? Merge and clean up everything
workmux merge
```

</div>

<div class="showcase-container">
  <div class="window-glow"></div>
  <div class="terminal-window">
    <div class="terminal-header">
      <div class="window-controls">
        <span class="control red"></span>
        <span class="control yellow"></span>
        <span class="control green"></span>
      </div>
      <div class="window-title">workmux demo</div>
    </div>
    <div class="video-container">
      <video src="/demo.mp4" controls muted playsinline preload="metadata"></video>
      <button type="button" class="video-play-button" aria-label="Play video"></button>
    </div>
  </div>
</div>

<div class="dashboard-section">
  <h2>Monitor your agents</h2>
  <p>A tmux popup dashboard to track progress across all agents.</p>
  <div class="showcase-container">
    <div class="terminal-window">
      <div class="terminal-header">
        <div class="window-controls">
          <span class="control red"></span>
          <span class="control yellow"></span>
          <span class="control green"></span>
        </div>
        <div class="window-title">workmux dashboard</div>
      </div>
      <img src="/dashboard.webp" alt="workmux dashboard" class="dashboard-img">
    </div>
  </div>
</div>

<script setup>
import { onMounted } from 'vue'
import { data as stars } from './stars.data'

onMounted(() => {
  // Add star count to GitHub hero button
  if (stars) {
    const btn = document.querySelector('.VPHero .actions a[href="https://github.com/raine/workmux"]')
    if (btn && !btn.querySelector('.star-count')) {
      const formatted = stars >= 1000 ? (stars / 1000).toFixed(1) + 'k' : stars
      const span = document.createElement('span')
      span.className = 'star-count'
      span.textContent = `★ ${formatted}`
      btn.appendChild(span)
    }
  }

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

.code-snippet {
  max-width: 500px;
  margin: 0 auto 3rem;
}

.star-count {
  padding-left: 8px;
  border-left: 1px solid var(--vp-c-divider);
  font-size: 0.9em;
  opacity: 0.8;
}

/* Terminal window showcase */
.showcase-container {
  position: relative;
  max-width: 880px;
  margin: 3rem auto;
  padding: 0 1.5rem;
}

@media (max-width: 640px) {
  .showcase-container {
    padding: 0;
  }
}

.window-glow {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 90%;
  height: 90%;
  background: var(--vp-c-brand-1);
  filter: blur(70px);
  opacity: 0.2;
  border-radius: 50%;
  z-index: 0;
  pointer-events: none;
}

.terminal-window {
  position: relative;
  z-index: 1;
  background: #1e1e1e;
  border-radius: 10px;
  box-shadow:
    0 20px 50px -10px rgba(0,0,0,0.3),
    0 0 0 1px rgba(255,255,255,0.1);
  overflow: hidden;
}

.terminal-header {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 28px;
  background: #2d2d2d;
  position: relative;
}

.window-controls {
  position: absolute;
  left: 10px;
  display: flex;
  gap: 6px;
}

.control {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}

.control.red { background-color: #ff5f56; }
.control.yellow { background-color: #ffbd2e; }
.control.green { background-color: #27c93f; }

.window-title {
  font-family: var(--vp-font-family-mono);
  font-size: 0.75rem;
  color: rgba(255, 255, 255, 0.4);
}

.video-container {
  position: relative;
}

.video-container video {
  display: block;
  width: 100%;
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
  background: rgba(255, 255, 255, 0.15);
  backdrop-filter: blur(4px);
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
  background: var(--vp-c-brand-1);
  transform: translate(-50%, -50%) scale(1.05);
}

.video-container.playing .video-play-button {
  display: none;
}

.dashboard-section {
  max-width: 940px;
  margin: 4rem auto 0;
  text-align: center;
  padding: 0 1.5rem;
}

.dashboard-section h2 {
  border: none;
  margin: 0 0 0.75rem;
  padding: 0;
  font-weight: 700;
  font-size: 1.5rem;
}

.dashboard-section p {
  font-size: 1.1rem;
  line-height: 1.6;
  color: var(--vp-c-text-2);
  margin: 0;
}

.dashboard-section .showcase-container {
  margin-top: 1.5rem;
}

@media (max-width: 640px) {
  .dashboard-section {
    padding: 0;
  }
}

.dashboard-img {
  display: block;
  width: 100%;
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
