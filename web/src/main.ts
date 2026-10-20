import { mount } from "svelte";
import App from "./App.svelte";
import { post } from "./lib/app.svelte";
import "@fontsource-variable/source-serif-4/opsz.css";
import "@fontsource-variable/source-serif-4/opsz-italic.css";
import "@fontsource-variable/brygada-1918/wght.css";
import "@fontsource-variable/brygada-1918/wght-italic.css";
import "@fontsource-variable/noto-serif-kr/wght.css";
import "@fontsource-variable/noto-serif-jp/wght.css";
import "@fontsource-variable/noto-serif-sc/wght.css";
import "@fontsource-variable/noto-serif-tc/wght.css";
import "temml/dist/Temml-Local.css";
import "./app.css";

// Chinese, Japanese and Korean share Han characters but draw some differently. Cards
// don't say their language, so the Mac's first CJK language picks the forms.
const han = navigator.languages
  .map((tag) => tag.toLowerCase())
  .map((tag) => (tag.startsWith("ja") ? "ja" : /^zh-(hant|tw|hk|mo)/.test(tag) ? "zh-hant" : tag.startsWith("zh") ? "zh-hans" : tag.startsWith("ko") ? "ko" : null))
  .find(Boolean);
if (han) document.documentElement.dataset.han = han;

mount(App, { target: document.getElementById("app")! });
post({ type: "ready" });
