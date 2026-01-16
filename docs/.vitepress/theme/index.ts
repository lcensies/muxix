import DefaultTheme from "vitepress/theme";
import CopyMarkdown from "./CopyMarkdown.vue";
import { h } from "vue";
import "./custom.css";

export default {
  extends: DefaultTheme,
  Layout() {
    return h(DefaultTheme.Layout, null, {
      "doc-before": () => h(CopyMarkdown),
    });
  },
};
