import { mount } from "svelte";
import App from "./App.svelte";
import { post } from "./lib/app.svelte";
import "@fontsource-variable/source-serif-4/opsz.css";
import "@fontsource-variable/source-serif-4/opsz-italic.css";
import "@fontsource-variable/brygada-1918/wght.css";
import "@fontsource-variable/brygada-1918/wght-italic.css";
import "temml/dist/Temml-Local.css";
import "./app.css";

mount(App, { target: document.getElementById("app")! });
post({ type: "ready" });
