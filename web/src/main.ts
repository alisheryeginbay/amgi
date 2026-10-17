import { mount } from "svelte";
import App from "./App.svelte";
import { post } from "./lib/app.svelte";
import "temml/dist/Temml-Local.css";
import "./app.css";

mount(App, { target: document.getElementById("app")! });
post({ type: "ready" });
