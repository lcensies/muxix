# VitePress Documentation Reference

## What is VitePress?

VitePress is a Static Site Generator (SSG) designed for building fast,
content-centric websites. It transforms Markdown content into static HTML pages
deployable anywhere.

### Primary Use Cases

- **Documentation Sites**: Powers technical documentation for major projects
  including Vite, Rollup, Pinia, and Vue.js itself
- **Content Platforms**: Supports blogs, portfolios, and marketing websites
  through customizable themes

### Key Features

- **Vite Integration**: Rapid server startup with near-instantaneous hot reload
- **Markdown Extensions**: Frontmatter, tables, syntax highlighting, and
  advanced code block features
- **Vue Compatibility**: Each Markdown page functions as a Vue Single-File
  Component
- **Hybrid Rendering**: Pre-rendered static HTML for initial load, then SPA
  behavior for navigation

VitePress is the modernized successor to VuePress 1, leveraging Vue 3 and Vite
instead of Vue 2 and webpack.

---

## Getting Started

### Requirements

- **Node.js 18 or higher**
- ESM-only: Ensure `package.json` contains `"type": "module"` or use
  `.mjs`/`.mts` extensions

### Installation

```bash
npm add -D vitepress@next
```

### Setup Wizard

```bash
npx vitepress init
```

The wizard prompts for:

- Config location
- Site title and description
- Theme preference
- Whether to add npm scripts

### File Structure

A typical VitePress project scaffolded in `./docs`:

```
docs/
├── .vitepress/
│   ├── config.js          # Site configuration
│   └── theme/
│       └── index.js       # Custom theme (optional)
├── public/                # Static assets (favicon, robots.txt)
├── guide/
│   ├── index.md           # /guide/
│   └── getting-started.md # /guide/getting-started
├── api/
│   └── index.md           # /api/
└── index.md               # Home page
```

The `.vitepress` directory is reserved for config, cache, build output, and
theme customization.

### Running Locally

```bash
npm run docs:dev
```

Dev server runs at `http://localhost:5173` with hot module replacement.

### Building for Production

```bash
npm run docs:build
npm run docs:preview  # Preview at localhost:4173
```

---

## Site Configuration

Configuration is defined in `.vitepress/config.[ext]` (js, ts, mjs, mts).

### Complete Example

```javascript
import { defineConfig } from 'vitepress';

export default defineConfig({
  // Site metadata
  title: 'My Documentation',
  description: 'A VitePress powered documentation site',
  lang: 'en-US',

  // Deployment
  base: '/', // Use '/repo-name/' for GitHub Pages subdirectory

  // URL handling
  cleanUrls: true,

  // Source/output directories
  srcDir: './src', // Default: project root
  outDir: './.vitepress/dist',

  // Git-based timestamps
  lastUpdated: true,

  // Appearance
  appearance: true, // Enable dark mode toggle

  // Markdown configuration
  markdown: {
    lineNumbers: true,
    theme: {
      light: 'github-light',
      dark: 'github-dark',
    },
  },

  // Head tags
  head: [
    ['link', { rel: 'icon', href: '/favicon.ico' }],
    ['meta', { name: 'theme-color', content: '#3eaf7c' }],
  ],

  // Theme configuration
  themeConfig: {
    // ... see Theme Configuration section
  },
});
```

### Site Metadata

| Option        | Description                                |
| ------------- | ------------------------------------------ |
| `title`       | Site name displayed in nav bar             |
| `description` | Meta tag content for SEO                   |
| `lang`        | HTML language attribute (default: `en-US`) |
| `head`        | Additional HTML elements to inject         |
| `base`        | Deployment path for sub-directory hosting  |

### Routing Options

| Option      | Description                        |
| ----------- | ---------------------------------- |
| `cleanUrls` | Removes trailing `.html` from URLs |
| `rewrites`  | Custom directory-to-URL mappings   |

### Build Options

| Option            | Description                        |
| ----------------- | ---------------------------------- |
| `srcDir`          | Markdown source directory location |
| `outDir`          | Build output directory             |
| `assetsDir`       | Generated assets folder name       |
| `ignoreDeadLinks` | Dead link handling behavior        |

### Customization

| Option        | Description                       |
| ------------- | --------------------------------- |
| `markdown`    | Markdown-it parser configuration  |
| `vite`        | Raw Vite config options           |
| `vue`         | Plugin options for Vue files      |
| `appearance`  | Dark mode toggle control          |
| `lastUpdated` | Git-based page timestamp tracking |

### Build Hooks

- `transformPageData` - Modify page metadata during processing
- `transformHead` - Dynamically add head entries per page
- `postRender` - Handle content after SSG rendering
- `buildEnd` - Execute logic after build completion

---

## Default Theme Configuration

Theme options are set under `themeConfig` in your config file.

### Complete Theme Example

```javascript
export default defineConfig({
  themeConfig: {
    // Branding
    logo: '/logo.svg',
    siteTitle: 'My Docs',

    // Navigation
    nav: [
      { text: 'Guide', link: '/guide/' },
      { text: 'API', link: '/api/' },
      {
        text: 'Resources',
        items: [
          { text: 'Blog', link: '/blog/' },
          { text: 'Changelog', link: '/changelog' },
        ],
      },
    ],

    // Sidebar
    sidebar: {
      '/guide/': [
        {
          text: 'Introduction',
          items: [
            { text: 'What is This?', link: '/guide/' },
            { text: 'Getting Started', link: '/guide/getting-started' },
          ],
        },
        {
          text: 'Advanced',
          collapsed: true,
          items: [{ text: 'Configuration', link: '/guide/configuration' }],
        },
      ],
      '/api/': [
        {
          text: 'API Reference',
          items: [{ text: 'Overview', link: '/api/' }],
        },
      ],
    },

    // Social links
    socialLinks: [
      { icon: 'github', link: 'https://github.com/user/repo' },
      { icon: 'twitter', link: 'https://twitter.com/user' },
    ],

    // Footer
    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © 2024',
    },

    // Edit link
    editLink: {
      pattern: 'https://github.com/user/repo/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },

    // Search
    search: {
      provider: 'local',
    },

    // Outline/TOC
    outline: {
      level: [2, 3],
      label: 'On this page',
    },

    // Last updated
    lastUpdated: {
      text: 'Updated at',
      formatOptions: {
        dateStyle: 'medium',
      },
    },
  },
});
```

### Logo & Branding

```javascript
themeConfig: {
  logo: '/logo.svg',
  // Or with light/dark variants:
  logo: { light: '/light-logo.svg', dark: '/dark-logo.svg' },
  siteTitle: 'My Docs',  // Set to false to hide
}
```

### Navigation

```javascript
themeConfig: {
  nav: [
    // Simple link
    { text: 'Guide', link: '/guide/' },

    // Link with active match pattern
    { text: 'Config', link: '/config/', activeMatch: '/config/' },

    // Dropdown menu
    {
      text: 'Dropdown',
      items: [
        { text: 'Item A', link: '/item-a' },
        { text: 'Item B', link: '/item-b' },
        // Grouped items
        {
          items: [
            { text: 'Section', link: '/section' },
          ],
        },
      ],
    },
  ],
}
```

### Sidebar

**Simple sidebar (all pages):**

```javascript
themeConfig: {
  sidebar: [
    {
      text: 'Guide',
      items: [
        { text: 'Introduction', link: '/guide/' },
        { text: 'Getting Started', link: '/guide/getting-started' },
      ],
    },
  ],
}
```

**Multiple sidebars (by path):**

```javascript
themeConfig: {
  sidebar: {
    '/guide/': [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/' },
        ],
      },
    ],
    '/api/': [
      {
        text: 'API',
        items: [
          { text: 'Reference', link: '/api/' },
        ],
      },
    ],
  },
}
```

**Collapsible groups:**

```javascript
{
  text: 'Advanced',
  collapsed: false,  // Open by default
  // collapsed: true,  // Closed by default
  items: [
    { text: 'Item', link: '/advanced/item' },
  ],
}
```

Supports up to 6 levels of nesting.

### Social Links

```javascript
themeConfig: {
  socialLinks: [
    { icon: 'github', link: 'https://github.com/...' },
    { icon: 'twitter', link: 'https://twitter.com/...' },
    { icon: 'discord', link: 'https://discord.gg/...' },
    // Custom SVG icon
    {
      icon: { svg: '<svg>...</svg>' },
      link: 'https://...',
    },
  ],
}
```

Available icons: `discord`, `facebook`, `github`, `instagram`, `linkedin`,
`mastodon`, `npm`, `slack`, `twitter`, `x`, `youtube`.

### Other Theme Options

| Option        | Description                                        |
| ------------- | -------------------------------------------------- |
| `aside`       | Aside panel position (`true`, `'left'`, `false`)   |
| `outline`     | Table of contents display and heading levels       |
| `footer`      | Message and copyright text (no-sidebar pages only) |
| `editLink`    | Links to edit pages on GitHub, etc.                |
| `socialLinks` | Social media icons in navbar                       |

---

## Search

### Local Search (Built-in)

```javascript
themeConfig: {
  search: {
    provider: 'local',
    options: {
      // Customize search
      translations: {
        button: {
          buttonText: 'Search',
          buttonAriaLabel: 'Search',
        },
        modal: {
          noResultsText: 'No results for',
          resetButtonTitle: 'Reset search',
          footer: {
            selectText: 'to select',
            navigateText: 'to navigate',
          },
        },
      },
    },
  },
}
```

### Algolia DocSearch

```javascript
themeConfig: {
  search: {
    provider: 'algolia',
    options: {
      appId: 'YOUR_APP_ID',
      apiKey: 'YOUR_SEARCH_API_KEY',
      indexName: 'YOUR_INDEX_NAME',
    },
  },
}
```

### Exclude Pages from Search

In frontmatter:

```yaml
---
search: false
---
```

---

## Home Page Layout

Set `layout: home` in frontmatter to use the home page layout.

### Complete Home Page Example

```markdown
---
layout: home

hero:
  name: My Project
  text: Build amazing things
  tagline: A powerful toolkit for developers
  image:
    src: /hero-image.png
    alt: Project Logo
  actions:
    - theme: brand
      text: Get Started
      link: /guide/
    - theme: alt
      text: View on GitHub
      link: https://github.com/user/repo

features:
  - icon: ⚡️
    title: Lightning Fast
    details: Built on Vite for instant server start and HMR
    link: /guide/performance
    linkText: Learn more
  - icon: 📝
    title: Markdown Focused
    details: Write content in Markdown with Vue components
  - icon: 🎨
    title: Customizable
    details: Extend with Vue components and custom CSS
---

## Additional Content

You can add markdown content below the frontmatter.
```

### Hero Options

| Option    | Description                                  |
| --------- | -------------------------------------------- |
| `name`    | Product name (displayed in brand color)      |
| `text`    | Main headline (rendered as h1)               |
| `tagline` | Subtitle text                                |
| `image`   | Logo/image with optional light/dark variants |
| `actions` | CTA buttons with `theme: 'brand'` or `'alt'` |

### Features Options

| Option     | Description                        |
| ---------- | ---------------------------------- |
| `icon`     | Emoji or image path (supports SVG) |
| `title`    | Feature title                      |
| `details`  | Feature description                |
| `link`     | Optional link URL                  |
| `linkText` | Link button text                   |

### Custom Hero Colors

```css
/* .vitepress/theme/custom.css */
:root {
  --vp-home-hero-name-color: transparent;
  --vp-home-hero-name-background: linear-gradient(120deg, #bd34fe, #41d1ff);
}
```

---

## Frontmatter Configuration

### Page-Level Options

```yaml
---
title: Page Title
titleTemplate: ':title - Custom Suffix'
description: Page description for SEO
head:
  - - meta
    - name: keywords
      content: vitepress, documentation

# Layout options
layout: doc # doc | home | page
navbar: true
sidebar: true
aside: true # true | false | 'left'
outline: [2, 3] # Heading levels in TOC
lastUpdated: true
editLink: true
footer: true
pageClass: custom-page-class
---
```

### Layout Types

| Layout | Description                                    |
| ------ | ---------------------------------------------- |
| `doc`  | Default documentation layout with all features |
| `home` | Special home page layout with hero/features    |
| `page` | Minimal layout for custom pages                |

### Disable Layout Elements

```yaml
---
navbar: false
sidebar: false
aside: false
footer: false
editLink: false
lastUpdated: false
---
```

---

## Routing

### File-Based Routing

Directory structure determines URL structure:

| File                       | URL                           |
| -------------------------- | ----------------------------- |
| `index.md`                 | `/`                           |
| `guide/index.md`           | `/guide/`                     |
| `guide/getting-started.md` | `/guide/getting-started.html` |

### Linking Between Pages

Use extension-less links:

```markdown
<!-- Good -->

[Link](./getting-started) [Link](/guide/getting-started)

<!-- Avoid -->

[Link](./getting-started.md)
```

### Clean URLs

Enable in config for extension-less URLs (`/path` instead of `/path.html`):

```javascript
export default {
  cleanUrls: true,
};
```

Requires hosting platform support (Netlify, Vercel, GitHub Pages).

### Dynamic Routes

Generate multiple pages from a single template using `[param].md` files paired
with `.paths.js` loader files:

```
docs/
├── posts/
│   ├── [slug].md        # Template
│   └── [slug].paths.js  # Data loader
```

**`[slug].paths.js`:**

```javascript
export default {
  paths() {
    return [
      { params: { slug: 'post-1' }, content: '# Post 1' },
      { params: { slug: 'post-2' }, content: '# Post 2' },
    ];
  },
};
```

**`[slug].md`:**

```markdown
---
title: { { $params.slug } }
---

Dynamic content here
```

---

## Asset Handling

### Public Directory

Place static files in `docs/public/`:

```
docs/
├── public/
│   ├── favicon.ico
│   ├── logo.png
│   ├── robots.txt
│   └── og-image.png
```

Reference with absolute paths (no `/public` prefix):

```markdown
![Logo](/logo.png)
```

### Relative Asset References

```markdown
![Image](./images/screenshot.png)
```

Assets are processed by Vite:

- Small images (<4kb) are base64 inlined
- Hashed filenames in production for cache busting

### Base URL for Assets

When deploying to a subdirectory, set `base` in config:

```javascript
export default {
  base: '/my-repo/',
};
```

For dynamic paths in Vue components, use `withBase`:

```vue
<script setup>
import { withBase } from 'vitepress';
</script>

<template>
  <img :src="withBase('/logo.png')" />
</template>
```

---

## Internationalization (i18n)

### Directory Structure

```
docs/
├── en/
│   ├── index.md
│   └── guide/
├── zh/
│   ├── index.md
│   └── guide/
├── index.md          # Default locale
```

### Configuration

```javascript
export default defineConfig({
  locales: {
    root: {
      label: 'English',
      lang: 'en',
    },
    zh: {
      label: '中文',
      lang: 'zh-CN',
      link: '/zh/',
      themeConfig: {
        nav: [{ text: '指南', link: '/zh/guide/' }],
        sidebar: {
          // Chinese sidebar
        },
      },
    },
  },
});
```

### Per-Locale Configuration

Each locale can override:

- `lang` - HTML lang attribute
- `dir` - Text direction (`'ltr'` or `'rtl'`)
- `title` / `titleTemplate`
- `description`
- `head`
- `themeConfig` - Theme-specific options

### RTL Support

```javascript
locales: {
  ar: {
    label: 'العربية',
    lang: 'ar',
    dir: 'rtl',
  },
}
```

---

## Extending the Default Theme

### Custom CSS

Create `.vitepress/theme/index.js`:

```javascript
import DefaultTheme from 'vitepress/theme';
import './custom.css';

export default DefaultTheme;
```

**`.vitepress/theme/custom.css`:**

```css
/* Override CSS variables */
:root {
  --vp-c-brand-1: #646cff;
  --vp-c-brand-2: #747bff;
}

/* Custom styles */
.vp-doc h1 {
  border-bottom: 2px solid var(--vp-c-brand-1);
}
```

### Register Global Components

```javascript
// .vitepress/theme/index.js
import DefaultTheme from 'vitepress/theme';
import MyComponent from './components/MyComponent.vue';

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('MyComponent', MyComponent);
  },
};
```

### Layout Slots

Inject content into the default layout:

```javascript
// .vitepress/theme/index.js
import DefaultTheme from 'vitepress/theme';
import MyLayout from './MyLayout.vue';

export default {
  extends: DefaultTheme,
  Layout: MyLayout,
};
```

**`MyLayout.vue`:**

```vue
<script setup>
import DefaultTheme from 'vitepress/theme';
const { Layout } = DefaultTheme;
</script>

<template>
  <Layout>
    <template #nav-bar-content-after>
      <CustomNavItem />
    </template>
    <template #doc-before>
      <ArticleMeta />
    </template>
    <template #doc-after>
      <Comments />
    </template>
  </Layout>
</template>
```

**Available slots:**

- `nav-bar-title-before/after`
- `nav-bar-content-before/after`
- `sidebar-nav-before/after`
- `doc-before/after`
- `doc-top/bottom`
- `doc-footer-before`
- `home-hero-before/after`
- `home-features-before/after`

### Custom Fonts

```javascript
// .vitepress/theme/index.js
import DefaultTheme from 'vitepress/theme-without-fonts';
import './fonts.css';

export default DefaultTheme;
```

**`fonts.css`:**

```css
@font-face {
  font-family: 'My Font';
  src: url('/fonts/my-font.woff2') format('woff2');
}

:root {
  --vp-font-family-base: 'My Font', sans-serif;
  --vp-font-family-mono: 'Fira Code', monospace;
}
```

---

## Using Vue in Markdown

### Inline Vue

```markdown
# Counter Example

{{ 1 + 1 }}

<script setup>
import { ref } from 'vue'
const count = ref(0)
</script>

<button @click="count++">Count: {{ count }}</button>
```

### Import Components

```markdown
<script setup>
import CustomComponent from './components/CustomComponent.vue'
</script>

# My Page

<CustomComponent :prop="value" />
```

### Access Page Data

```vue
<script setup>
import { useData } from 'vitepress';
const { page, frontmatter, theme } = useData();
</script>

<template>
  <div>
    <h1>{{ frontmatter.title }}</h1>
    <p>Last updated: {{ page.lastUpdated }}</p>
  </div>
</template>
```

### Component Naming

Components must use PascalCase or hyphenated names to avoid being wrapped in
`<p>` tags:

```markdown
<!-- Good -->
<CustomComponent />
<my-component />

<!-- Bad - will cause hydration errors -->
<customcomponent />
```

### SSR Compatibility

Components must be SSR-safe. For client-only components:

```vue
<script setup>
import { onMounted, ref } from 'vue';

const ClientOnlyComponent = ref(null);

onMounted(async () => {
  ClientOnlyComponent.value = (await import('./ClientOnly.vue')).default;
});
</script>

<template>
  <component :is="ClientOnlyComponent" v-if="ClientOnlyComponent" />
</template>
```

---

## Markdown Features

### Frontmatter

```yaml
---
title: Page Title
description: Page description
---
```

### Header Anchors

Custom anchors with `{#custom-id}` syntax:

```markdown
## My Heading {#custom-anchor}
```

### Table of Contents

Auto-generated with:

```markdown
[[toc]]
```

### Custom Containers

```markdown
::: info This is an info box. :::

::: tip This is a tip. :::

::: warning This is a warning. :::

::: danger This is a dangerous warning. :::

::: details Click to expand Hidden content here. :::
```

### GitHub-Flavored Alerts

```markdown
> [!NOTE] Highlights information.

> [!TIP] Optional information to help.

> [!IMPORTANT] Crucial information.

> [!WARNING] Critical content requiring attention.

> [!CAUTION] Negative potential consequences.
```

### Code Blocks

**Syntax highlighting:**

````markdown
```javascript
const hello = 'world';
```
````

**Line highlighting:**

````markdown
```javascript{4}
// line 4 highlighted
```

```javascript{4,7-13}
// line 4 and lines 7-13 highlighted
```
````

**Focus mode:**

```javascript
export default {
  data() {
    return {
      msg: 'Focused!', // [!code focus]
    };
  },
};
```

**Colored diffs:**

```javascript
export default {
  data() {
    return {
      msg: 'Removed', // [!code --]
      msg: 'Added', // [!code ++]
    };
  },
};
```

**Error/warning markers:**

```javascript
const error = 'This has error'; // [!code error]
const warning = 'This has warning'; // [!code warning]
```

**Line numbers:**

````markdown
```javascript:line-numbers
// line numbers enabled
```

```javascript:line-numbers=5
// starts at line 5
```
````

### Code Groups

````markdown
::: code-group

```javascript [config.js]
export default {};
```

```typescript [config.ts]
export default {};
```

:::
````

### File Imports

```markdown
<<< @/filepath <<< @/filepath{2-10} <<< @/filepath{2-10 javascript}
```

### Math Equations

Requires `markdown-it-mathjax3` plugin:

```markdown
$a^2 + b^2 = c^2$
```

---

## Deployment

### Build Scripts

Add to `package.json`:

```json
{
  "scripts": {
    "docs:dev": "vitepress dev docs",
    "docs:build": "vitepress build docs",
    "docs:preview": "vitepress preview docs"
  }
}
```

### GitHub Pages

**`.github/workflows/deploy.yml`:**

```yaml
name: Deploy VitePress site to Pages

on:
  push:
    branches: [main]

permissions:
  contents: read
  pages: write
  id-token: write

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
      - run: npm ci
      - run: npm run docs:build
      - uses: actions/upload-pages-artifact@v3
        with:
          path: docs/.vitepress/dist

  deploy:
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/deploy-pages@v4
        id: deployment
```

For project repos (`user.github.io/repo`), set base:

```javascript
export default {
  base: '/repo-name/',
};
```

### Netlify

- Build command: `npm run docs:build`
- Publish directory: `docs/.vitepress/dist`
- Node version: 20

### Vercel

- Framework Preset: VitePress
- Build command: `npm run docs:build`
- Output directory: `docs/.vitepress/dist`

### Cloudflare Pages

- Build command: `npm run docs:build`
- Build output directory: `docs/.vitepress/dist`

### Cache Headers

Configure for the `/assets/` directory:

```
Cache-Control: max-age=31536000, immutable
```

---

## Quick Reference: Minimal Docs Site

### 1. Initialize

```bash
npm add -D vitepress
npx vitepress init
```

### 2. Config (`docs/.vitepress/config.js`)

```javascript
import { defineConfig } from 'vitepress';

export default defineConfig({
  title: 'My Project',
  description: 'Project documentation',
  themeConfig: {
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Guide', link: '/guide/' },
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/' },
          { text: 'Getting Started', link: '/guide/getting-started' },
        ],
      },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/...' }],
  },
});
```

### 3. Home Page (`docs/index.md`)

```markdown
---
layout: home

hero:
  name: My Project
  text: Tagline here
  actions:
    - theme: brand
      text: Get Started
      link: /guide/

features:
  - title: Feature One
    details: Description of feature one
  - title: Feature Two
    details: Description of feature two
---
```

### 4. Guide Pages

**`docs/guide/index.md`:**

```markdown
# Introduction

Welcome to the documentation.
```

**`docs/guide/getting-started.md`:**

```markdown
# Getting Started

## Installation

npm install my-project
```

### 5. Run

```bash
npm run docs:dev
```

---

## Links

- Official Documentation: https://vitepress.dev/
- GitHub: https://github.com/vuejs/vitepress
- Default Theme CSS Variables:
  https://github.com/vuejs/vitepress/blob/main/src/client/theme-default/styles/vars.css
