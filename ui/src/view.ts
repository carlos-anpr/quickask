// view.ts — DOM y eventos, sin frameworks. Render keyed (R8): nodos
// persistentes por clave, un solo paint por frame, stick-to-bottom suave.
import { needsScreenLocal, type Backend } from "./bridge";
import { installCopyHandler, installLinkHandler, renderMarkdownMemo } from "./markdown";
import { initialState, reduce, type AppState, type Turn } from "./state";
import {
  convoTotals,
  estOutTokens,
  fmtCostFor,
  fmtElapsed,
  fmtTokens,
  isFreeModel,
  modelLabelFor,
  showConfirm,
  filterModels,
  toolLabel,
  variantLabel,
} from "./tools";
import type { ScreenMode, UiEvent } from "./contract";

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function toolStatusMark(status: string): string {
  if (status === "ok") return "✓";
  if (status === "error") return "✕";
  return "";
}

const raf: (cb: () => void) => number =
  typeof requestAnimationFrame !== "undefined"
    ? (cb) => requestAnimationFrame(cb)
    : (cb) => window.setTimeout(cb, 16);
const caf: (id: number) => void =
  typeof cancelAnimationFrame !== "undefined"
    ? (id) => cancelAnimationFrame(id)
    : (id) => window.clearTimeout(id);

const SUGGESTIONS = [
  "Explícame lo que hay en pantalla",
  "Resume este error",
  "Busca en la web…",
];

const MONITOR_SVG =
  '<svg width="14" height="14" viewBox="0 0 16 16" fill="none" ' +
  'stroke="currentColor" stroke-width="1.5" aria-hidden="true">' +
  '<rect x="1.5" y="3" width="13" height="9" rx="1.5"/>' +
  '<path d="M6 14.5h4M8 12v2.5"/></svg>';

const NEW_SVG =
  '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" ' +
  'stroke="currentColor" stroke-width="1.5" stroke-linecap="round" ' +
  'stroke-linejoin="round" aria-hidden="true">' +
  '<rect x="2" y="2" width="12" height="12" rx="2.5"/>' +
  '<path d="M11 5.2 10.8 4.8 6.5 9.1 6 11.5l2.4-.5z"/></svg>';

const BRAIN_SVG =
  '<svg width="12" height="12" viewBox="0 0 16 16" fill="none" ' +
  'stroke="currentColor" stroke-width="1.4" stroke-linecap="round" aria-hidden="true">' +
  '<circle cx="8" cy="8" r="6"/><path d="M8 4.5v3l2 2"/></svg>';

const RELAY_SVG =
  '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" ' +
  'stroke="currentColor" stroke-width="1.5" stroke-linecap="round" ' +
  'stroke-linejoin="round" aria-hidden="true">' +
  '<path d="M13 8A5 5 0 1 1 8 3c1.8 0 3.3.9 4.2 2.3"/><path d="M12.5 1.8v3.7H8.8"/></svg>';


interface SegNodes {
  div: HTMLElement;
  lastSrc: string;
}

interface TurnNodes {
  art: HTMLElement;
  headQ: HTMLElement;
  head: HTMLElement;
  thumbImg?: HTMLImageElement;
  screenDiv?: HTMLElement;
  details?: HTMLDetailsElement;
  summary?: HTMLElement;
  reasonBody?: HTMLElement;
  reasonShown: string | null;
  toolsDiv?: HTMLElement;
  activity?: HTMLDetailsElement;
  activitySum?: HTMLElement;
  activityClosed?: boolean;
  toolMap: Map<string, { chip: HTMLElement; status: string; label: string }>;
  permsDiv?: HTMLElement;
  permSet: Set<string>;
  noticesDiv?: HTMLElement;
  noticeCount: number;
  answersDiv?: HTMLElement;
  segMap: Map<string, SegNodes>;
  segOrder: string[];
  confirmDiv?: HTMLElement;
  foot?: HTMLElement;
  durSpan?: HTMLElement;
  useSpan?: HTMLElement;
  errSpan?: HTMLElement;
  lastFootKey: string;
}

export function createApp(root: HTMLElement, backend: Backend): {
  handleEvent: (ev: UiEvent) => void;
  getState: () => AppState;
  destroy: () => void;
} {
  let appState: AppState = initialState();
  let screenMode: ScreenMode = "auto";
  let alive = true;
  let stick = true;
  let paintQueued = false;
  let loopId = 0;
  let loopOn = false;
  const openReasoning = new Set<string>();
  const turnMap = new Map<string, TurnNodes>();
  const times = new Map<string, { start: number; end?: number }>();
  const turnModel = new Map<string, string>();
  let lastResizeHeight = 0;
  let lastResizeAt = 0;
  let toastTimer = 0;
  let debounceTimer = 0;
  let lastEffortLabel = "";
  let lastModelKey = "";
  // Nodos persistentes del centro: el spinner NO se re-crea en cada tick.
  let centerMode = "";
  let spinEl: HTMLElement | null = null;
  let timeEl: HTMLElement | null = null;
  let liveEl: HTMLElement | null = null;
  let totalsEl: HTMLElement | null = null;

  // ---- estructura ----
  const card = el("div", "card");
  const toast = el("div", "toast");
  toast.hidden = true;
  toast.setAttribute("role", "status");

  const composer = el("header", "composer");
  const icon = el("div", "composer-icon", "✦");
  icon.setAttribute("aria-hidden", "true");
  const textarea = document.createElement("textarea");
  textarea.className = "input";
  textarea.rows = 1;
  textarea.placeholder = "¿Qué quieres preguntar hoy?";
  textarea.setAttribute("aria-label", "Pregunta a quickask");
  const side = el("div", "composer-side");
  const screenChip = document.createElement("button");
  screenChip.type = "button";
  screenChip.className = "chip-screen";
  screenChip.title = "Pantalla: auto (Ctrl+Shift+S)";
  screenChip.innerHTML =
    `${MONITOR_SVG}<span class="chip-label">Auto</span>` +
    `<span class="pulse" aria-hidden="true"></span>`;
  const newBtn = document.createElement("button");
  newBtn.type = "button";
  newBtn.className = "new-btn gone";
  newBtn.title = "Nueva conversación (Ctrl+N)";
  newBtn.setAttribute("aria-label", "Nueva conversación");
  newBtn.innerHTML = NEW_SVG;
  const sendBtn = document.createElement("button");
  sendBtn.type = "button";
  sendBtn.className = "btn-send";
  sendBtn.setAttribute("aria-label", "Enviar");

  side.append(screenChip, newBtn, sendBtn);
  composer.append(icon, textarea, side);

  const divider = el("div", "divider");
  divider.setAttribute("aria-hidden", "true");

  const suggestions = el("div", "suggestions");
  suggestions.setAttribute("aria-label", "Sugerencias");
  for (const s of SUGGESTIONS) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "suggestion";
    b.textContent = s;
    b.addEventListener("click", () => {
      textarea.value = s;
      textarea.focus();
      autoGrow();
      updateScreenChip();
    });
    suggestions.append(b);
  }

  const turnsEl = el("section", "turns");
  turnsEl.setAttribute("aria-live", "off");

  const scrollBtn = document.createElement("button");
  scrollBtn.type = "button";
  scrollBtn.className = "scroll-btn";
  scrollBtn.textContent = "↓";
  scrollBtn.setAttribute("aria-label", "Volver al fondo");
  scrollBtn.addEventListener("click", () => {
    stick = true;
    ensureLoop();
  });

  const statusbar = el("footer", "statusbar");
  const dot = el("span", "backend-dot");
  dot.title = "backend";
  const center = el("span", "center tabular");
  const context = el("span", "context-usage tabular", "ctx —");
  context.title = "Contexto consumido: tokens del último paso, incluida la caché";
  // ---- menús de modelo y de esfuerzo: se construyen al abrirse con el catálogo real ----
  const modelWrap = el("span", "model-wrap");
  const modelBtn = document.createElement("button");
  modelBtn.type = "button";
  modelBtn.className = "model-btn";
  const modelMenu = el("div", "model-menu menu");
  modelMenu.hidden = true;
  const modelSearch = document.createElement("input");
  modelSearch.type = "search";
  modelSearch.className = "model-search";
  modelSearch.placeholder = "Buscar modelo…";
  modelSearch.setAttribute("aria-label", "Buscar modelo");
  const modelList = el("div", "model-list");
  modelList.setAttribute("role", "listbox");
  modelMenu.append(modelSearch, modelList);
  let modelItems: typeof appState.models = [];
  let modelActive = 0;

  const effortWrap = el("span", "effort-wrap");
  const effortBtn = document.createElement("button");
  effortBtn.type = "button";
  effortBtn.className = "effort";
  const effortMenu = el("div", "effort-menu menu");
  effortMenu.hidden = true;

  function currentModelInfo(): (typeof appState.models)[number] | undefined {
    return appState.models.find((m) => m.id === appState.model);
  }
  function closeMenus(): void {
    modelMenu.hidden = true;
    effortMenu.hidden = true;
  }
  function chooseModel(id: string): void {
    closeMenus();
    if (id !== appState.model) void backend.setModel(id).catch((e) => showToast(String(e)));
  }
  function markActive(): void {
    const rows = modelList.querySelectorAll<HTMLElement>(".menu-row");
    rows.forEach((r, i) => r.classList.toggle("active", i === modelActive));
    const row = rows[modelActive];
    if (row) {
      // Desplazar solo la lista (scrollIntoView movería también otros contenedores).
      if (row.offsetTop < modelList.scrollTop) modelList.scrollTop = row.offsetTop;
      else if (row.offsetTop + row.offsetHeight > modelList.scrollTop + modelList.clientHeight)
        modelList.scrollTop = row.offsetTop + row.offsetHeight - modelList.clientHeight;
    }
  }
  function renderModelList(): void {
    modelItems = filterModels(appState.models, modelSearch.value);
    modelList.innerHTML = "";
    if (modelItems.length === 0) {
      modelList.append(el("div", "menu-empty", "Sin resultados"));
      return;
    }
    modelActive = Math.max(0, Math.min(modelActive, modelItems.length - 1));
    modelItems.forEach((m, i) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "menu-row";
      row.setAttribute("role", "option");
      row.dataset.id = m.id;
      row.classList.toggle("sel", m.id === appState.model);
      const text = el("span", "row-text");
      const sub = m.id.slice(m.id.indexOf("/") + 1);
      text.append(el("span", "row-name", m.label), el("span", "row-sub", sub));
      const tags = el("span", "row-tags");
      if (m.free) tags.append(el("span", "tag tag-free", "gratis"));
      if (!m.vision) tags.append(el("span", "tag", "sin visión"));
      tags.append(el("span", "row-check", m.id === appState.model ? "✓" : ""));
      row.append(text, tags);
      row.addEventListener("mousemove", () => {
        if (modelActive !== i) {
          modelActive = i;
          markActive();
        }
      });
      row.addEventListener("click", (e) => {
        e.stopPropagation();
        chooseModel(m.id);
      });
      modelList.append(row);
    });
    markActive();
  }
  function fitMenu(menu: HTMLElement, list: HTMLElement): void {
    // Se abre hacia arriba si cabe; si no (tarjeta corta), hacia abajo por el hueco
    // transparente de la ventana. Alto mínimo útil 200 px.
    const bar = statusbar.getBoundingClientRect();
    const above = bar.top - 16;
    const below = window.innerHeight - bar.bottom - 16;
    const down = above < 260 && below > above;
    menu.classList.toggle("down", down);
    const extra = menu === modelMenu ? 48 : 12;
    list.style.maxHeight = `${Math.max(160, Math.min(420, (down ? below : above) - extra))}px`;
  }
  function openModelMenu(): void {
    effortMenu.hidden = true;
    modelSearch.value = "";
    modelActive = Math.max(0, appState.models.findIndex((m) => m.id === appState.model));
    modelMenu.hidden = false;
    modelBtn.title = "";
    fitMenu(modelMenu, modelList);
    renderModelList();
    modelSearch.focus();
  }
  function openEffortMenu(): void {
    const info = currentModelInfo();
    if (!info || info.variants.length === 0) return;
    modelMenu.hidden = true;
    effortMenu.innerHTML = "";
    for (const v of ["", ...info.variants]) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "menu-row";
      row.classList.toggle("sel", v === appState.variant);
      row.append(el("span", "row-name", variantLabel(v)), el("span", "row-check", v === appState.variant ? "✓" : ""));
      row.addEventListener("click", (e) => {
        e.stopPropagation();
        closeMenus();
        if (v !== appState.variant) void backend.setVariant(v).catch((err) => showToast(String(err)));
      });
      effortMenu.append(row);
    }
    effortMenu.hidden = false;
    fitMenu(effortMenu, effortMenu);
    (effortMenu.querySelector<HTMLElement>(".menu-row.sel") ?? effortMenu.querySelector<HTMLElement>(".menu-row"))?.focus();
  }
  modelBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (modelMenu.hidden) openModelMenu();
    else closeMenus();
  });
  effortBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (effortMenu.hidden) openEffortMenu();
    else closeMenus();
  });
  modelMenu.addEventListener("click", (e) => e.stopPropagation());
  modelSearch.addEventListener("input", () => {
    modelActive = 0;
    renderModelList();
  });
  modelSearch.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = modelItems.length;
      if (n) modelActive = (modelActive + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      markActive();
    } else if (e.key === "Enter") {
      e.preventDefault();
      const m = modelItems[modelActive];
      if (m) chooseModel(m.id);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenus();
      modelBtn.focus();
    }
  });
  effortMenu.addEventListener("keydown", (e) => {
    const rows = [...effortMenu.querySelectorAll<HTMLElement>(".menu-row")];
    const i = rows.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      rows[(i + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length]?.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenus();
      effortBtn.focus();
    }
  });
  document.addEventListener("click", closeMenus);
  const more = el("span", "more", "?");
  more.title =
    "Enter enviar · Shift+Enter salto · Ctrl+N nueva · Esc cerrar · " +
    "Ctrl+Shift+S pantalla · Ctrl+O abrir en terminal";
  modelWrap.append(modelBtn, modelMenu);
  effortWrap.append(effortBtn, effortMenu);

  statusbar.append(dot, center, modelWrap, effortWrap, context, more);
  card.append(toast, composer, divider, suggestions, turnsEl, scrollBtn, statusbar);
  root.append(card);

  const lightbox = el("div", "lightbox");
  lightbox.hidden = true;
  const lightImg = document.createElement("img");
  lightImg.className = "lightbox-img";
  lightbox.append(lightImg);
  card.append(lightbox);
  lightbox.addEventListener("click", () => {
    lightbox.hidden = true;
  });

  installCopyHandler(card);
  installLinkHandler(card, (url) => {
    void backend.openUrl(url).catch((err) => showToast(String(err)));
  });

  // ---- helpers ----
  function showToast(msg: string): void {
    toast.textContent = msg;
    toast.hidden = false;
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      toast.hidden = true;
    }, 4000);
  }

  // Captura de esta invocación pendiente: en modo auto se adjunta a la primera pregunta.
  let freshShot = false;

  function updateScreenChip(): void {
    const labels: Record<ScreenMode, string> = { auto: "Auto", on: "Sí", off: "No" };
    screenChip.dataset.mode = screenMode;
    screenChip.title =
      screenMode === "auto"
        ? "Pantalla: auto — se adjunta a la primera pregunta tras abrir (Ctrl+Shift+S cambia)"
        : `Pantalla: ${labels[screenMode].toLowerCase()} (auto/sí/no, Ctrl+Shift+S)`;
    const labelEl = screenChip.querySelector(".chip-label");
    if (labelEl) labelEl.textContent = labels[screenMode];
    const live =
      screenMode === "on" ||
      (screenMode === "auto" && (freshShot || needsScreenLocal(textarea.value)));
    screenChip.classList.toggle("active", live);
    screenChip.classList.toggle(
      "needs",
      screenMode === "auto" && needsScreenLocal(textarea.value),
    );
  }

  function scheduleChipUpdate(): void {
    window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(updateScreenChip, 150);
  }

  function autoGrow(): void {
    // Recalcular siempre desde 1 línea: con la ventana oculta (daemon al
    // arrancar) las medidas son basura (~130 px) y hay que desecharlas.
    textarea.style.height = "28px";
    if (textarea.clientWidth === 0) {
      textarea.style.overflowY = "hidden";
      return;
    }
    const lineHeight = 28;
    const max = lineHeight * 6;
    textarea.style.height = `${Math.max(lineHeight, Math.min(textarea.scrollHeight, max))}px`;
    textarea.style.overflowY = textarea.scrollHeight > max ? "auto" : "hidden";
  }

  function maybeResize(force = false): void {
    const now = Date.now();
    if (!force && now - lastResizeAt < 60) return;
    const h = Math.ceil(card.getBoundingClientRect().height + 24);
    if (Math.abs(h - lastResizeHeight) > 4 || force) {
      lastResizeHeight = h;
      lastResizeAt = now;
      void backend.resize(h).catch(() => undefined);
    }
  }


  function isNearBottom(px = 48): boolean {
    return turnsEl.scrollHeight - turnsEl.scrollTop - turnsEl.clientHeight < px;
  }

  function currentTurn(): Turn | undefined {
    for (let i = appState.turns.length - 1; i >= 0; i--) {
      if (appState.turns[i].outcome === undefined) return appState.turns[i];
    }
    return undefined;
  }

  // Modelo con el que se preguntó cada turno (para el "gratis" del pie).
  function turnFree(turnId: string): boolean {
    const id = turnModel.get(turnId) ?? appState.model;
    return isFreeModel(appState.models, id);
  }

  // ---- nodos keyed ----
  function getTurnNodes(t: Turn): TurnNodes {
    let n = turnMap.get(t.turnId);
    if (n) return n;
    const art = el("article", "turn");
    art.dataset.turn = t.turnId;
    const head = el("div", "turn-head");
    const headQ = el("div", "q");
    const marker = el("span", "q-marker", "›");
    marker.setAttribute("aria-hidden", "true");
    headQ.append(marker, document.createTextNode(` ${t.text}`));
    head.append(headQ);
    art.append(head);
    n = {
      art,
      headQ,
      head,
      reasonShown: null,
      toolMap: new Map(),
      permSet: new Set(),
      noticesDiv: undefined,
      noticeCount: 0,
      segMap: new Map(),
      segOrder: [],
      lastFootKey: "",
    };
    turnMap.set(t.turnId, n);
    return n;
  }

  function setText(node: { textContent: string | null }, s: string): void {
    if (node.textContent !== s) node.textContent = s;
  }

  function updateTurn(t: Turn, n: TurnNodes, isLast: boolean): void {
    // miniatura / estado de captura
    if (t.screenStatus === "attached" && t.thumb && !n.thumbImg) {
      const img = document.createElement("img");
      img.className = "thumb-mini";
      img.src = t.thumb;
      img.alt = "Captura de pantalla adjunta (clic para ampliar)";
      img.addEventListener("click", () => {
        lightImg.src = t.thumb ?? "";
        lightbox.hidden = false;
      });
      n.head.append(img);
      n.thumbImg = img;
    }
    if ((t.screenStatus === "capturing" || t.screenStatus === "failed") && !n.screenDiv) {
      n.screenDiv = el("div", "screen-status");
      n.art.append(n.screenDiv);
    }
    if (n.screenDiv) {
      if (t.screenStatus === "capturing") setText(n.screenDiv, "Capturando pantalla…");
      else if (t.screenStatus === "failed") {
        n.screenDiv.classList.add("error");
        setText(n.screenDiv, t.screenError || "No se pudo adjuntar la pantalla");
      } else if (t.screenStatus === "attached") {
        // La captura ya está: quitar el aviso con fade-out (una sola vez).
        const gone = n.screenDiv;
        if (!gone.classList.contains("fade-out")) {
          gone.classList.add("fade-out");
          window.setTimeout(() => {
            gone.remove();
            if (n.screenDiv === gone) n.screenDiv = undefined;
          }, 150);
        }
      }
    }

    // reasoning: una línea + contador; cuerpo solo si se despliega
    if (t.reasoning && !n.details) {
      const details = document.createElement("details");
      details.className = "reasoning";
      if (openReasoning.has(t.turnId)) details.open = true;
      details.addEventListener("toggle", () => {
        if (details.open) {
          openReasoning.add(t.turnId);
          if (n.reasonBody) {
            n.reasonBody.textContent = t.reasoning;
            n.reasonShown = t.reasoning;
          }
        } else {
          openReasoning.delete(t.turnId);
          if (n.reasonBody) {
            n.reasonBody.textContent = "";
            n.reasonShown = "";
          }
        }
      });
      const summary = el("summary", "");
      const body = el("div", "reasoning-body");
      details.append(summary, body);
      n.details = details;
      n.summary = summary;
      n.reasonBody = body;
      n.art.append(details);
    }
    if (n.summary) {
      const tm = times.get(t.turnId);
      const secs = tm ? Math.max(0, Math.floor(((tm.end ?? Date.now()) - tm.start) / 1000)) : 0;
      setText(n.summary, t.outcome ? "Pensamiento" : `Pensando… · ${secs} s`);
      n.summary.classList.toggle("shimmer", !t.outcome);
    }
    if (n.reasonBody && n.details?.open && n.reasonShown !== t.reasoning) {
      n.reasonBody.textContent = t.reasoning;
      n.reasonShown = t.reasoning;
    }

    // tools por callId
    // Bloque "Actividad": siempre ANTES de la respuesta, abierto mientras trabaja y
    // plegado al terminar para que la respuesta quede a la vista.
    if (t.tools.length > 0 && !n.toolsDiv) {
      const box = document.createElement("details");
      box.className = "activity";
      box.open = true;
      const sum = el("summary", "activity-sum");
      n.toolsDiv = el("div", "tools");
      box.append(sum, n.toolsDiv);
      n.activity = box;
      n.activitySum = sum;
      if (n.answersDiv) n.art.insertBefore(box, n.answersDiv);
      else n.art.append(box);
    }
    if (n.activity && n.activitySum) {
      const count = t.tools.length;
      setText(n.activitySum, `${t.outcome ? "Actividad" : "Trabajando"} · ${count} ${count === 1 ? "acción" : "acciones"}`);
      if (t.outcome && !n.activityClosed) {
        n.activity.open = false;
        n.activityClosed = true;
      }
    }
    if (n.toolsDiv) {
      for (const tool of t.tools) {
        const { icon, label } = toolLabel(tool.name, tool.title);
        const short = label.slice(0, 80);
        const mark = toolStatusMark(tool.status);
        const cached = n.toolMap.get(tool.callId);
        if (cached) {
          if (cached.status !== tool.status || cached.label !== short) {
            cached.chip.className = `tool tool-${tool.status}`;
            const tw = cached.chip.querySelector(".tool-text");
            if (tw) tw.textContent = `${short} ${mark}`;
            cached.status = tool.status;
            cached.label = short;
          }
          // El spinner se ELIMINA del DOM al terminar (solo queda ✓/✕).
          const wantSpin = tool.status === "running" || tool.status === "pending";
          const first = cached.chip.firstElementChild;
          const hasSpin = !!first && first.classList.contains("spin");
          if (!wantSpin && hasSpin) (first as HTMLElement).remove();
          else if (wantSpin && !hasSpin) {
            const spin = el("span", "spin");
            spin.setAttribute("aria-hidden", "true");
            cached.chip.prepend(spin);
          }
          cached.chip.title = tool.title ?? tool.name;
        } else {
          const chip = el("span", `tool tool-${tool.status} chip-in`);
          if (tool.status === "running" || tool.status === "pending") {
            const spin = el("span", "spin");
            spin.setAttribute("aria-hidden", "true");
            chip.append(spin);
          }
          const iconWrap = el("span", "tool-icon");
          iconWrap.innerHTML = icon;
          const tw = el("span", "tool-text", `${short} ${mark}`);
          chip.append(iconWrap, tw);
          chip.title = tool.title ?? tool.name;
          n.toolsDiv.append(chip);
          n.toolMap.set(tool.callId, { chip, status: tool.status, label: short });
        }
      }
    }

    // permisos por requestId
    if (t.permissions.length > 0 && !n.permsDiv) {
      n.permsDiv = el("div", "perms");
      n.art.append(n.permsDiv);
    }
    if (n.permsDiv) {
      for (const p of t.permissions) {
        if (n.permSet.has(p.requestId)) continue;
        const permCard = el("div", "permission");
        permCard.dataset.req = p.requestId;
        permCard.append(el("div", "perm-title", "Permiso necesario"));
        const desc = el("div", "perm-desc");
        desc.append(el("code", "", p.action), document.createTextNode(` ${p.resources.join(" ")}`));
        permCard.append(desc);
        const btns = el("div", "perm-btns");
        const mk = (txt: string, d: "once" | "always" | "reject"): HTMLButtonElement => {
          const b = document.createElement("button");
          b.type = "button";
          b.textContent = txt;
          b.addEventListener("click", () => {
            void backend.replyPermission(p.requestId, d).catch((e) => showToast(String(e)));
          });
          return b;
        };
        btns.append(mk("Permitir una vez", "once"), mk("Permitir siempre", "always"), mk("Rechazar", "reject"));
        permCard.append(btns);
        n.permsDiv.append(permCard);
        n.permSet.add(p.requestId);
      }
      // retirar cerradas
      const live = new Set(t.permissions.map((p) => p.requestId));
      for (const req of [...n.permSet]) {
        if (!live.has(req)) {
          n.permsDiv.querySelector(`[data-req="${req}"]`)?.remove();
          n.permSet.delete(req);
        }
      }
    }

    // notices de relevo: línea discreta con icono
    if (t.notices.length > 0 && !n.noticesDiv) {
      n.noticesDiv = el("div", "notices");
      if (n.answersDiv) n.art.insertBefore(n.noticesDiv, n.answersDiv);
      else n.art.append(n.noticesDiv);
    }
    if (n.noticesDiv) {
      while (n.noticeCount < t.notices.length) {
        const line = el("div", "notice notice-in");
        line.innerHTML = RELAY_SVG;
        line.append(document.createTextNode(t.notices[n.noticeCount]));
        n.noticesDiv.append(line);
        n.noticeCount++;
      }
    }

    // segmentos por messageId:ordinal (memo por bloque, sin recrear nodos)
    if (t.segments.length > 0 && !n.answersDiv) {
      n.answersDiv = el("div", "answers");
      n.answersDiv.setAttribute("aria-live", "polite");
      n.art.append(n.answersDiv);
    }
    if (n.answersDiv) {
      const order: string[] = [];
      for (const seg of t.segments) {
        const key = `${seg.messageId}:${seg.ordinal}`;
        order.push(key);
        let sn = n.segMap.get(key);
        if (!sn) {
          const div = document.createElement("div");
          div.className = "md seg-in";
          div.dataset.seg = `${t.turnId}:${key}`;
          n.answersDiv.append(div);
          sn = { div, lastSrc: "" };
          n.segMap.set(key, sn);
        }
        if (sn.lastSrc !== seg.text) {
          sn.div.innerHTML = renderMarkdownMemo(seg.text);
          sn.lastSrc = seg.text;
        }
        sn.div.classList.toggle("streaming", !seg.final && !t.outcome);
      }
      n.segOrder = order;
    }

    // confirmación: solo último turno, con outcome y sin busy
    const lastSeg = t.segments[t.segments.length - 1];
    const wantConfirm =
      showConfirm({
        outcome: t.outcome,
        lastText: lastSeg ? lastSeg.text : null,
        isLast,
        busy: appState.busy,
      }) && !!lastSeg;
    if (wantConfirm && !n.confirmDiv) {
      const confirm = el("div", "confirm");
      const yes = document.createElement("button");
      yes.type = "button";
      yes.className = "confirm-yes";
      yes.textContent = "Sí, hazlo";
      yes.addEventListener("click", () => {
        void backend.ask("Sí, hazlo.", "off").catch((e) => showToast(String(e)));
      });
      const no = document.createElement("button");
      no.type = "button";
      no.className = "confirm-no";
      no.textContent = "No";
      no.addEventListener("click", () => {
        void backend.ask("No, no lo hagas.", "off").catch((e) => showToast(String(e)));
      });
      confirm.append(yes, no);
      n.art.append(confirm);
      n.confirmDiv = confirm;
    } else if (!wantConfirm && n.confirmDiv) {
      n.confirmDiv.remove();
      n.confirmDiv = undefined;
    }

    // pie: solo con outcome (en curso ya hay temporizador en la barra)
    if (!n.foot) {
      const foot = el("div", "foot");
      const dur = el("span", "dur");
      const use = el("span", "usage");
      const err = el("span", "error");
      err.hidden = true;
      foot.append(dur, use, err);
      n.foot = foot;
      n.durSpan = dur;
      n.useSpan = use;
      n.errSpan = err;
    }
    n.foot.style.display = t.outcome ? "" : "none";
    n.art.append(n.foot);
    const tm = times.get(t.turnId);
    const durTxt = tm ? fmtElapsed((tm.end ?? Date.now()) - tm.start) : "";
    if (n.durSpan && n.durSpan.textContent !== durTxt) n.durSpan.textContent = durTxt;
    const useKey = t.usage
      ? `${t.usage.cost}|${t.usage.tokensIn}|${t.usage.tokensOut}|${t.usage.tokensReasoning}`
      : "";
    if (useKey !== n.lastFootKey) {
      n.lastFootKey = useKey;
      if (n.useSpan) {
        n.useSpan.innerHTML = "";
        if (t.usage) {
          n.useSpan.append(
            document.createTextNode(`↑ ${fmtTokens(t.usage.tokensIn)} · ↓ ${fmtTokens(t.usage.tokensOut)}`),
          );
          if (t.usage.tokensReasoning > 0) {
            const r = el("span", "reas");
            r.innerHTML =
              `${BRAIN_SVG}<span> ${fmtTokens(t.usage.tokensReasoning)}</span>`;
            n.useSpan.append(document.createTextNode(" · "), r);
          }
          n.useSpan.append(document.createTextNode(` · ${fmtCostFor(t.usage.cost, turnFree(t.turnId))}`));
        }
      }
      if (n.errSpan) {
        if (t.outcome === "failed") {
          n.errSpan.hidden = false;
          n.errSpan.textContent = t.error || "Falló el turno";
        } else if (t.outcome === "interrupted") {
          n.errSpan.hidden = false;
          n.errSpan.textContent = "Interrumpido";
          n.errSpan.className = "interrupted";
        } else {
          n.errSpan.hidden = true;
        }
      }
    }
  }

  // ---- paint (máximo 1 por frame) ----
  function paint(): void {
    paintQueued = false;
    if (!alive) return;
    // turnos: crear solo los nuevos, en orden, sin reconstruir
    const ids = new Set(appState.turns.map((t) => t.turnId));
    for (const id of [...turnMap.keys()]) {
      if (!ids.has(id)) {
        turnMap.get(id)?.art.remove();
        turnMap.delete(id);
      }
    }
    appState.turns.forEach((t, i) => {
      const n = getTurnNodes(t);
      const ref = turnsEl.children[i] ?? null;
      if (n.art !== ref) turnsEl.insertBefore(n.art, ref);
      updateTurn(t, n, i === appState.turns.length - 1);
    });
    turnsEl.hidden = appState.turns.length === 0;

    // cabecera
    sendBtn.textContent = appState.busy ? "■" : "↑";
    sendBtn.classList.toggle("stop", appState.busy);
    sendBtn.setAttribute("aria-label", appState.busy ? "Parar" : "Enviar");
    sendBtn.title = appState.busy ? "Parar (interrumpe el turno)" : "Enviar (Enter)";
    textarea.placeholder = appState.greeting || "¿Qué quieres preguntar hoy?";
    card.classList.toggle("is-busy", appState.busy);
    const hasTurns = appState.turns.length > 0;
    card.classList.toggle("has-turns", hasTurns);
    divider.hidden = !hasTurns;
    suggestions.hidden = hasTurns;
    newBtn.classList.toggle("gone", !hasTurns);

    // statusbar: punto + centro + modelo + esfuerzo
    dot.dataset.status = appState.backend;
    dot.textContent = appState.backend === "ready" ? "●" : appState.backend === "connecting" ? "◌" : "●";
    // chips de modelo y de esfuerzo (los menús se construyen al abrirse)
    {
      const label = modelLabelFor(appState.models, appState.model);
      const free = isFreeModel(appState.models, appState.model);
      const key = `${label}|${free ? 1 : 0}`;
      if (key !== lastModelKey) {
        lastModelKey = key;
        modelBtn.innerHTML = "";
        modelBtn.append(el("span", "model-name", label.length > 22 ? `${label.slice(0, 21)}…` : label));
        if (free) modelBtn.append(el("span", "free-tag", "gratis"));
      }
      const fb = appState.models.find((m) => m.id === appState.fallbackModel);
      // Sin tooltip con el menú abierto: taparía el buscador.
      modelBtn.title = modelMenu.hidden
        ? `${appState.model}${fb ? ` · si falla, se reintenta con ${fb.label}` : ""}`
        : "";
      const info = currentModelInfo();
      const noVariants = !info || info.variants.length === 0;
      const effortLabel = `${variantLabel(appState.variant)}|${noVariants ? 1 : 0}`;
      if (effortLabel !== lastEffortLabel) {
        lastEffortLabel = effortLabel;
        effortBtn.textContent = variantLabel(noVariants ? "" : appState.variant);
        effortBtn.classList.toggle("muted", noVariants);
        effortBtn.title = noVariants ? "Este modelo no tiene niveles de esfuerzo" : "";
      }
    }
    context.textContent = `ctx ${appState.contextTokens === undefined ? "—" : fmtTokens(appState.contextTokens)}`;
    const cur = currentTurn();
    if (appState.busy && cur) {
      if (centerMode !== "busy") {
        centerMode = "busy";
        center.innerHTML = "";
        spinEl = el("span", "spin");
        spinEl.setAttribute("aria-hidden", "true");
        timeEl = el("span", "live-time");
        liveEl = el("span", "");
        center.append(spinEl, timeEl, liveEl);
        totalsEl = null;
      }
      const tm = times.get(cur.turnId);
      const el2 = tm ? fmtElapsed(Date.now() - tm.start) : "";
      const chars = cur.reasoning.length + cur.segments.reduce((a, s) => a + s.text.length, 0);
      const hasUsage = !!cur.usage;
      const liveTxt = `${hasUsage ? `↑ ${fmtTokens(cur.usage?.tokensIn ?? 0)}` : "↑ —"} ↓ ${
        cur.usage ? fmtTokens(cur.usage.tokensOut) : `~${fmtTokens(estOutTokens(chars))}`
      }`;
      if (timeEl && timeEl.textContent !== el2) timeEl.textContent = el2;
      if (liveEl && liveEl.textContent !== liveTxt) liveEl.textContent = liveTxt;
    } else {
      const tot = convoTotals(appState.turns);
      const totTxt = `Σ ↑ ${fmtTokens(tot.tokensIn)} · ↓ ${fmtTokens(tot.tokensOut)} · ${fmtCostFor(tot.cost, isFreeModel(appState.models, appState.model))}`;
      if (centerMode !== "idle") {
        centerMode = "idle";
        center.innerHTML = "";
        totalsEl = el("span", "", totTxt);
        center.append(totalsEl);
        spinEl = timeEl = liveEl = null;
      } else if (totalsEl && totalsEl.textContent !== totTxt) {
        totalsEl.textContent = totTxt;
      }
    }

    // stick-to-bottom suave (solo avanza; los scrolls del usuario mandan)
    if (stick) {
      const target = turnsEl.scrollHeight - turnsEl.clientHeight;
      const d = target - turnsEl.scrollTop;
      if (Math.abs(d) > 0.5) {
        turnsEl.scrollTop += d * 0.22;
      }
    }
    scrollBtn.classList.toggle("show", hasTurns && !stick);

    updateScreenChip();
    autoGrow();
    maybeResize();
  }

  function queuePaint(): void {
    if (paintQueued || !alive) return;
    paintQueued = true;
    raf(() => paint());
  }

  function ensureLoop(): void {
    if (loopOn || !alive) return;
    loopOn = true;
    const step = (): void => {
      if (!alive) {
        loopOn = false;
        return;
      }
      paint();
      const settled = !stick || Math.abs(turnsEl.scrollHeight - turnsEl.clientHeight - turnsEl.scrollTop) <= 0.5;
      if (appState.busy || !settled) {
        loopId = raf(step);
      } else {
        loopOn = false;
      }
    };
    loopId = raf(step);
  }

  // ---- eventos ----
  function handleEvent(ev: UiEvent): void {
    if (ev.kind === "summoned") {
      card.classList.remove("enter");
      void card.offsetWidth;
      card.classList.add("enter");
      freshShot = true;
      textarea.focus();
      textarea.select();
      autoGrow();
    } else if (ev.kind === "turnStarted") {
      times.set(ev.turnId, { start: Date.now() });
      turnModel.set(ev.turnId, appState.model);
      stick = true;
    } else if (ev.kind === "done") {
      const tm = times.get(ev.turnId);
      if (tm) tm.end = Date.now();
      ensureLoop(); // asentar el fondo tras el último contenido
    } else if (ev.kind === "conversationReset") {
      times.clear();
      turnModel.clear();
      stick = true;
    }
    appState = reduce(appState, ev);
    if (appState.busy) ensureLoop();
    else queuePaint();
  }

  // La intención del usuario se detecta por sus entradas (rueda, teclado, arrastre de la
  // barra), no por el evento scroll: el glide mueve scrollTop en cada frame mientras genera.
  let lastUserInput = 0;
  let pointerDown = false;
  const markUser = (): void => {
    lastUserInput = Date.now();
  };
  const onWheel = (e: WheelEvent): void => {
    markUser();
    if (e.deltaY < 0) {
      stick = false;
      queuePaint();
    }
  };
  const onScrollKey = (e: KeyboardEvent): void => {
    if (["PageUp", "ArrowUp", "Home"].includes(e.key)) {
      markUser();
      stick = false;
      queuePaint();
    } else if (["PageDown", "ArrowDown", "End"].includes(e.key)) {
      markUser();
    }
  };
  const onPointerDown = (): void => {
    pointerDown = true;
    markUser();
  };
  const onPointerMove = (): void => {
    if (pointerDown) markUser();
  };
  const onPointerUp = (): void => {
    pointerDown = false;
  };
  turnsEl.addEventListener("wheel", onWheel, { passive: true });
  turnsEl.addEventListener("keydown", onScrollKey);
  turnsEl.addEventListener("pointerdown", onPointerDown);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  let prevTop = 0;
  const onScroll = (): void => {
    const top = turnsEl.scrollTop;
    const movedUp = top < prevTop - 0.5;
    prevTop = top;
    // Solo cuentan los scrolls del usuario: subir suelta siempre; se re-engancha
    // únicamente al llegar bajando al fondo exacto (no por estar "cerca").
    if (Date.now() - lastUserInput < 400 || pointerDown) {
      if (movedUp) stick = false;
      else if (isNearBottom(4)) stick = true;
    }
    queuePaint();
  };
  turnsEl.addEventListener("scroll", onScroll);

  const onResize = (): void => {
    autoGrow();
    maybeResize(true);
  };
  window.addEventListener("resize", onResize);

  textarea.addEventListener("input", () => {
    autoGrow();
    scheduleChipUpdate();
    maybeResize();
  });

  async function submit(): Promise<void> {
    const text = textarea.value.trim();
    if (!text || appState.busy) return;
    try {
      await backend.ask(text, screenMode);
      freshShot = false;
      textarea.value = "";
      autoGrow();
      updateScreenChip();
      stick = true;
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e));
    }
  }

  sendBtn.addEventListener("click", () => {
    if (appState.busy) {
      void backend.stop().catch((e) => showToast(String(e)));
    } else {
      void submit();
    }
  });

  screenChip.addEventListener("click", () => {
    screenMode = screenMode === "auto" ? "on" : screenMode === "on" ? "off" : "auto";
    updateScreenChip();
  });

  newBtn.addEventListener("click", () => {
    void backend.newConversation().catch((e) => showToast(String(e)));
  });

  const onKey = (e: KeyboardEvent): void => {
    if (e.key === "Escape") {
      if (!effortMenu.hidden || !modelMenu.hidden) {
        effortMenu.hidden = true;
        modelMenu.hidden = true;
        return;
      }
      if (document.activeElement === textarea && textarea.value) {
        textarea.value = "";
        autoGrow();
        updateScreenChip();
      } else {
        void backend.hide().catch((err) => showToast(String(err)));
      }
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      void backend.newConversation().catch((err) => showToast(String(err)));
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
      e.preventDefault();
      void backend.openInTerminal().catch((err) => showToast(String(err)));
      return;
    }
    if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "s") {
      e.preventDefault();
      screenMode = screenMode === "auto" ? "on" : screenMode === "on" ? "off" : "auto";
      updateScreenChip();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && document.activeElement === textarea) {
      e.preventDefault();
      void submit();
    }
  };
  document.addEventListener("keydown", onKey);

  // ---- init ----
  void backend
    .getState()
    .then((s) => {
      appState = {
        ...appState,
        backend: s.backend,
        busy: s.busy,
        variant: s.variant ?? "",
        model: s.model || appState.model,
        fallbackModel: s.fallbackModel ?? appState.fallbackModel,
        models:
          s.models && s.models.length > 0
            ? s.models.map((m) => ({ ...m }))
            : appState.models,
      };
      if (s.theme === "dark" || s.theme === "light") {
        document.documentElement.dataset.theme = s.theme;
      } else {
        delete document.documentElement.dataset.theme;
      }
      queuePaint();
    })
    .catch((e) => {
      showToast(String(e));
      queuePaint();
    });

  queuePaint();
  return {
    handleEvent,
    getState: () => appState,
    destroy: () => {
      alive = false;
      caf(loopId);
      document.removeEventListener("keydown", onKey);
      turnsEl.removeEventListener("scroll", onScroll);
      turnsEl.removeEventListener("wheel", onWheel);
      turnsEl.removeEventListener("keydown", onScrollKey);
      turnsEl.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("resize", onResize);
      window.clearTimeout(toastTimer);
      window.clearTimeout(debounceTimer);
    },
  };
}
