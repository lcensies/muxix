import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'workmux',
  description: 'Parallel development in tmux with git worktrees',
  lang: 'en-US',
  lastUpdated: true,
  cleanUrls: true,

  // GitHub Pages subdirectory
  base: '/workmux/',

  head: [
    ['link', { rel: 'icon', href: '/workmux/logo.svg' }]
  ],

  themeConfig: {
    logo: { light: '/logo.svg', dark: '/logo-dark.svg' },
    siteTitle: 'workmux',

    search: {
      provider: 'local'
    },

    nav: [
      { text: 'Guide', link: '/guide/' },
      { text: 'Reference', link: '/reference/commands' },
      { text: 'Changelog', link: 'https://github.com/raine/workmux/blob/main/CHANGELOG.md' }
    ],

    sidebar: {
      '/guide/': [
        {
          text: 'Introduction',
          items: [
            { text: 'What is workmux?', link: '/guide/' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Quick Start', link: '/guide/quick-start' },
          ]
        },
        {
          text: 'Usage',
          items: [
            { text: 'Configuration', link: '/guide/configuration' },
            { text: 'AI Agents', link: '/guide/agents' },
            { text: 'Tips & Tricks', link: '/guide/tips' },
            { text: 'Caveats', link: '/guide/caveats' },
          ]
        }
      ],
      '/reference/': [
        {
          text: 'CLI Reference',
          items: [
            { text: 'Commands', link: '/reference/commands' },
          ]
        }
      ]
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/raine/workmux' }
    ],

    footer: {
      message: 'Released under the MIT License.'
    },

    editLink: {
      pattern: 'https://github.com/raine/workmux/edit/main/docs/:path',
      text: 'Edit this page on GitHub'
    }
  }
})
