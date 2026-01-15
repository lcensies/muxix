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
---

<div class="why-section">
  <h2>Why workmux?</h2>
  <div class="why-grid">
    <div class="why-item">
      <div class="why-header">
        <div class="why-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" y1="3" x2="6" y2="15"></line><circle cx="18" cy="6" r="3"></circle><circle cx="6" cy="18" r="3"></circle><path d="M18 9a9 9 0 0 1-9 9"></path></svg>
        </div>
        <h3>Parallel workflows</h3>
      </div>
      <p>Work on multiple features, hotfixes, or AI agents at the same time. No stashing, no branch switching, no conflicts.</p>
    </div>
    <div class="why-item">
      <div class="why-header">
        <div class="why-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect><line x1="9" y1="3" x2="9" y2="21"></line></svg>
        </div>
        <h3>One window per task</h3>
      </div>
      <p>A natural mental model. Each has its own terminal state, editor session, and dev server. Context switching is switching tabs.</p>
    </div>
    <div class="why-item">
      <div class="why-header">
        <div class="why-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"></polyline><line x1="12" y1="19" x2="20" y2="19"></line></svg>
        </div>
        <h3>tmux is the interface</h3>
      </div>
      <p>For existing and new tmux users. If you already live in tmux, it fits your workflow. If you don't, it's worth picking up.</p>
    </div>
  </div>
</div>

<div class="demo-section">
  <h2>See it in action</h2>
  <p>Spin up worktrees, develop in parallel, merge and clean up.</p>
  <div class="showcase-container main-demo">
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
</div>

<div class="code-snippet">

```bash
# Start working on a feature
workmux add my-feature

# Done? Merge and clean up everything
workmux merge
```

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
.demo-section {
  max-width: 940px;
  margin: 0 auto 4rem;
  padding: 0 1.5rem;
}

.demo-section h2 {
  text-align: center;
  border: none;
  margin: 0 0 0.75rem;
  padding: 0;
  font-weight: 700;
  font-size: 1.75rem;
}

.demo-section > p {
  text-align: center;
  font-size: 1.1rem;
  line-height: 1.6;
  color: var(--vp-c-text-2);
  margin: 0 0 2rem;
}

.demo-section .showcase-container.main-demo {
  margin-top: 0;
  margin-bottom: 0;
}

.why-section {
  max-width: 1100px;
  margin: 2rem auto 4rem;
  padding: 0 1.5rem;
}

.why-section h2 {
  text-align: center;
  border: none;
  margin: 0 0 2.5rem;
  padding: 0;
  font-weight: 700;
  font-size: 1.75rem;
}

.why-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 1.5rem;
}

.why-item {
  background-color: var(--vp-c-bg-soft);
  border: 1px solid var(--vp-c-divider);
  border-radius: 12px;
  padding: 28px;
}

.why-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 0.75rem;
}

.why-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  border-radius: 8px;
  background-color: var(--vp-c-brand-soft);
  color: var(--vp-c-brand-1);
}

.why-item h3 {
  font-size: 1.1rem;
  font-weight: 600;
  margin: 0;
  color: var(--vp-c-text-1);
}

.why-item p {
  font-size: 0.95rem;
  line-height: 1.6;
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
