import "vitepress/client";

declare module "vitepress/client" {
  interface PageData {
    rawMarkdownBase64?: string;
  }
}
