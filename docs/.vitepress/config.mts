import { defineConfig } from "vitepress";

export default defineConfig({
  title: "workmux - Parallel development in tmux with git worktrees",
  description: "Parallel development in tmux with git worktrees",
  lang: "en-US",
  lastUpdated: true,
  cleanUrls: true,
  sitemap: {
    hostname: "https://workmux.raine.dev",
  },

  head: [
    ["link", { rel: "icon", href: "/branch-icon.svg" }],
    [
      "meta",
      { name: "algolia-site-verification", content: "3CFC51B41FBBDD13" },
    ],
  ],

  vite: {
    resolve: {
      preserveSymlinks: true,
    },
    server: {
      fs: {
        allow: [".."],
      },
    },
  },

  themeConfig: {
    logo: { light: "/icon.svg", dark: "/icon-dark.svg" },
    siteTitle: "workmux",

    search: {
      provider: "algolia",
      options: {
        appId: "LE5BQE6V5G",
        apiKey: "5155e711e5233eab82a26f248b60b61b",
        indexName: "Workmux website",
      },
    },

    nav: [
      { text: "Guide", link: "/guide/" },
      { text: "Changelog", link: "/changelog" },
    ],

    sidebar: [
      {
        text: "Getting Started",
        items: [
          { text: "What is workmux?", link: "/guide/" },
          { text: "Installation", link: "/guide/installation" },
          { text: "Quick start", link: "/guide/quick-start" },
          { text: "Configuration", link: "/guide/configuration" },
        ],
      },
      {
        text: "Guides",
        items: [
          { text: "Nerdfont icons", link: "/guide/nerdfont-icons" },
          { text: "direnv", link: "/guide/direnv" },
          { text: "Git worktree caveats", link: "/guide/git-worktree-caveats" },
        ],
      },
      {
        text: "AI Agents",
        items: [
          { text: "Overview", link: "/guide/agents" },
          { text: "Claude Code", link: "/guide/claude-code" },
          { text: "Status tracking", link: "/guide/status-tracking" },
          { text: "Dashboard", link: "/guide/dashboard" },
          { text: "Delegating tasks", link: "/guide/delegating-tasks" },
        ],
      },
      {
        text: "Commands",
        items: [
          { text: "add", link: "/reference/commands/add" },
          { text: "merge", link: "/reference/commands/merge" },
          { text: "remove", link: "/reference/commands/remove" },
          { text: "list", link: "/reference/commands/list" },
          { text: "open", link: "/reference/commands/open" },
          { text: "close", link: "/reference/commands/close" },
          { text: "path", link: "/reference/commands/path" },
          { text: "dashboard", link: "/reference/commands/dashboard" },
          { text: "init", link: "/reference/commands/init" },
          { text: "claude prune", link: "/reference/commands/claude" },
          { text: "completions", link: "/reference/commands/completions" },
          { text: "docs", link: "/reference/commands/docs" },
        ],
      },
    ],

    socialLinks: [{ icon: "github", link: "https://github.com/raine/workmux" }],

    footer: {
      message: "Released under the MIT License.",
    },

    editLink: {
      pattern: "https://github.com/raine/workmux/edit/main/docs/:path",
      text: "Edit this page on GitHub",
    },
  },
});
