// tools.ts — etiquetas amables de herramientas + detección de confirmación.
// Funciones puras (FASE-2 FRONT).
const svg = (inner: string): string =>
  `<svg width="13" height="13" viewBox="0 0 16 16" fill="none" ` +
  `stroke="currentColor" stroke-width="1.4" stroke-linecap="round" ` +
  `stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;

export const ICONS = {
  globe: svg(`<circle cx="8" cy="8" r="6.5"/><path d="M1.5 8h13M8 1.5c-4 4-4 9 0 13M8 1.5c4 4 4 9 0 13"/>`),
  book: svg(`<path d="M4 2h7a1 1 0 0 1 1 1v10H5a1 1 0 0 0-1 1z"/><path d="M4 2v12"/>`),
  tag: svg(`<path d="M2.5 2.5H8l5.5 5.5-5.5 5.5L2.5 8z"/><circle cx="6" cy="6" r="1"/>`),
  code: svg(`<path d="M6 4 2.5 8 6 12M10 4l3.5 4L10 12"/>`),
  board: svg(`<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="M6 5.5v5M10 5.5v5"/>`),
  search: svg(`<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/>`),
  branches: svg(`<circle cx="4.5" cy="4.5" r="1.8"/><circle cx="4.5" cy="11.5" r="1.8"/><circle cx="11.5" cy="8" r="1.8"/><path d="M4.5 6.3v3.4M6.3 4.5H9a1 1 0 0 1 1 1v1"/>`),
  doc: svg(`<path d="M4 1.5h5.5L12.5 5v9.5h-8.5z"/><path d="M9.5 1.5V5H12.5"/>`),
  gear: svg(`<circle cx="8" cy="8" r="2.2"/><path d="M8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6 5 5M11 11l1.4 1.4M12.4 3.6 11 5M5 11l-1.4 1.4"/>`),
};

export interface ToolLabel {
  icon: string;
  label: string;
}

// Comandos de navegador (BACK B7: title sin el binario, p.ej. "click e12").
const BROWSER_CMD =
  /^(click|fill|snapshot|goto|open|press|eval|select|hover|type|check|uncheck|drag|upload|reload|run-code|route|tab-|tab |cookie|localstorage|console|network|pdf|dialog|screenshot|state-|go-back|go-forward)\b/;

// Servicios conocidos: si el title empieza por el servidor, se recorta en la etiqueta.
const SERVICE_META: Record<string, { icon: string; label: string }> = {
  freshservice: { icon: ICONS.tag, label: "Freshservice" },
  gitlab: { icon: ICONS.code, label: "GitLab" },
  jira: { icon: ICONS.board, label: "Jira" },
  exa: { icon: ICONS.search, label: "Exa" },
  context7: { icon: ICONS.book, label: "Context7" },
  discord: { icon: ICONS.tag, label: "Discord" },
  drawio: { icon: ICONS.doc, label: "Drawio" },
  markitdown: { icon: ICONS.doc, label: "Markitdown" },
};

export function toolLabel(name: string, title?: string): ToolLabel {
  const n = (name ?? "").toLowerCase();
  const t = (title ?? "").trim();
  const tl = t.toLowerCase();
  const hay = `${n} ${tl}`;
  const disp = t || name;
  if (n.includes("shell") && (/^playwright-cli\s+/i.test(t) || BROWSER_CMD.test(tl))) {
    return { icon: ICONS.globe, label: `Navegador: ${t.replace(/^playwright-cli\s+/i, "")}` };
  }
  // Nuevo formato execute (R10): "<servidor> <método> <args>" → recorta el servidor.
  const svc = /^\s*(freshservice|gitlab|jira|exa|context7|discord|drawio|markitdown)\b\s*(.*)$/i.exec(t);
  if (svc) {
    const meta = SERVICE_META[svc[1].toLowerCase()];
    const rest = svc[2].trim();
    return { icon: meta.icon, label: `${meta.label}: ${rest || disp}` };
  }
  if (hay.includes("freshservice")) return { icon: ICONS.tag, label: `Freshservice: ${disp}` };
  if (hay.includes("gitlab") || hay.includes("glab"))
    return { icon: ICONS.code, label: `GitLab: ${disp}` };
  if (hay.includes("jira")) return { icon: ICONS.board, label: `Jira: ${disp}` };
  if (n === "skill") return { icon: ICONS.book, label: `Skill: ${disp}` };
  if (/(^|[^a-z])(exa|websearch|webfetch)([^a-z]|$)/.test(hay))
    return { icon: ICONS.search, label: `Web: ${disp}` };
  if (n.includes("subagent")) return { icon: ICONS.branches, label: `Subagente: ${disp}` };
  if (/^(read|write|edit)\b/.test(n)) return { icon: ICONS.doc, label: disp };
  return { icon: ICONS.gear, label: t ? `Código: ${t}` : "Código" };
}

// ¿Termina la respuesta con pregunta de confirmación? Solo último párrafo.
const CONFIRM_RE = /¿[^?]{0,140}?\b(hago|publico|env[ií]o|confirmo|ejecuto|procedo|borro|compr[ao])\b[^?]{0,40}?\?/i;

// ¿Mostrar "Sí, hazlo / No"? Solo en el último turno, con outcome, sin busy.
export function showConfirm(opts: {
  outcome?: string;
  lastText: string | null;
  isLast: boolean;
  busy: boolean;
}): boolean {
  return (
    opts.isLast &&
    !opts.busy &&
    !!opts.outcome &&
    opts.lastText !== null &&
    needsConfirm(opts.lastText)
  );
}

export function needsConfirm(text: string): boolean {
  const paras = text
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  const last = paras[paras.length - 1] ?? "";
  return CONFIRM_RE.test(last);
}

// ---- formato es-ES (R8) ----
import type { Variant } from "./contract";

/** 2681 -> "2,7k"; 370 -> "370". */
export function fmtTokens(n: number): string {
  if (n >= 1000) {
    const v = (n / 1000).toFixed(1).replace(".", ",").replace(",0", "");
    return `${v}k`;
  }
  return String(Math.round(n));
}

/** 0.0008 -> "$0,0008". */
export function fmtCost(c: number): string {
  return `$${c.toFixed(4).replace(".", ",")}`;
}

/** 3400 -> "3,4 s"; 65000 -> "1:05". */
export function fmtElapsed(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1).replace(".", ",")} s`;
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

/** Salida estimada durante streaming: round(caracteres / 4). */
export function estOutTokens(chars: number): number {
  return Math.round(chars / 4);
}

export function shortModel(model: string): string {
  const i = model.lastIndexOf("/");
  return i >= 0 ? model.slice(i + 1) : model;
}

export const VARIANT_LABELS: Record<string, string> = {
  "": "Estándar",
  none: "Sin razonamiento",
  minimal: "Mínimo",
  low: "Bajo",
  medium: "Medio",
  high: "Alto",
  xhigh: "Muy alto",
  max: "Máx",
  thinking: "Razonando",
};

/** Etiqueta en español de un nivel de esfuerzo; desconocido → la palabra con mayúscula inicial. */
export function variantLabel(v: Variant): string {
  return VARIANT_LABELS[v] ?? v.charAt(0).toUpperCase() + v.slice(1);
}

/** Minúsculas y sin tildes, para buscar. */
export function fold(text: string): string {
  return text.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
}

/** Modelos cuyo label o id contienen la búsqueda (sin tildes ni mayúsculas). */
export function filterModels<T extends { id: string; label: string }>(models: ReadonlyArray<T>, query: string): T[] {
  const q = fold(query.trim());
  return q ? models.filter((m) => fold(`${m.label} ${m.id}`).includes(q)) : [...models];
}

export interface UsageTotals {
  cost: number;
  tokensIn: number;
  tokensOut: number;
}

/** Totales de la conversación: suma de los usage de todos los turnos. */
export function convoTotals(
  turns: ReadonlyArray<{ usage?: UsageTotals | undefined }>,
): UsageTotals {
  return turns.reduce<UsageTotals>(
    (acc, t) => ({
      cost: acc.cost + (t.usage?.cost ?? 0),
      tokensIn: acc.tokensIn + (t.usage?.tokensIn ?? 0),
      tokensOut: acc.tokensOut + (t.usage?.tokensOut ?? 0),
    }),
    { cost: 0, tokensIn: 0, tokensOut: 0 },
  );
}

/** Coste con relevo gratis (R11): free y 0 -> "gratis". */
export function fmtCostFor(cost: number, free: boolean): string {
  return free && cost === 0 ? "gratis" : fmtCost(cost);
}

export interface NamedModel {
  id: string;
  label: string;
  free: boolean;
}

/** Etiqueta del modelo ("Flash Free") o id corto si no está en la lista. */
export function modelLabelFor(models: ReadonlyArray<NamedModel>, id: string): string {
  return models.find((m) => m.id === id)?.label ?? shortModel(id);
}

/** ¿Es un modelo gratuito? */
export function isFreeModel(models: ReadonlyArray<NamedModel>, id: string): boolean {
  return models.find((m) => m.id === id)?.free ?? false;
}
