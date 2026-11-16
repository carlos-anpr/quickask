// main.ts — arranque: getState, suscripción a qa://event, teclado.
import "./styles.css";
import { backend } from "./bridge";
import { createApp } from "./view";

const root = document.querySelector<HTMLElement>("#app");
if (!root) {
  throw new Error("Falta #app en index.html");
}

const app = createApp(root, backend);
backend.onEvent((ev) => app.handleEvent(ev));
