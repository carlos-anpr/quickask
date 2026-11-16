// markdown.ts — Markdown -> HTML con marked + DOMPurify + highlight.js.
import { marked } from "marked";
import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import json from "highlight.js/lib/languages/json";
import rust from "highlight.js/lib/languages/rust";
import typescript from "highlight.js/lib/languages/typescript";
import javascript from "highlight.js/lib/languages/javascript";
import python from "highlight.js/lib/languages/python";
import sql from "highlight.js/lib/languages/sql";
import yaml from "highlight.js/lib/languages/yaml";
import toml from "highlight.js/lib/languages/ini";
import diff from "highlight.js/lib/languages/diff";
import markdownLang from "highlight.js/lib/languages/markdown";
import java from "highlight.js/lib/languages/java";
import xml from "highlight.js/lib/languages/xml";

hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sh", bash);
hljs.registerLanguage("shell", bash);
hljs.registerLanguage("json", json);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("ts", typescript);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("js", javascript);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("python", python);
hljs.registerLanguage("py", python);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("yml", yaml);
hljs.registerLanguage("toml", toml);
hljs.registerLanguage("ini", toml);
hljs.registerLanguage("diff", diff);
hljs.registerLanguage("markdown", markdownLang);
hljs.registerLanguage("md", markdownLang);
hljs.registerLanguage("java", java);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("html", xml);

marked.setOptions({ breaks: true, gfm: true });

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function highlightCode(code: string, lang: string | undefined): string {
  const language = (lang ?? "").toLowerCase();
  try {
    if (language && hljs.getLanguage(language)) {
      return hljs.highlight(code, { language }).value;
    }
  } catch {
    // fallback a escape plano
  }
  return escapeHtml(code);
}

export const COPY_SVG =
  '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" ' +
  'stroke="currentColor" stroke-width="1.5" aria-hidden="true">' +
  '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/>' +
  '<path d="M10.5 5.5v-2a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 3.5v5A1.5 1.5 0 0 0 4 10h1.5"/></svg>';

/** Renderer propio: code con cabecera (lenguaje + Copiar) y clase hljs, enlaces con target/rel. */
function buildHtml(md: string): string {
  const renderer = new marked.Renderer();
  renderer.code = ({ text, lang }: { text: string; lang?: string }) => {
    const language = (lang ?? "").trim().toLowerCase() || "texto";
    const highlighted = highlightCode(text, language === "texto" ? undefined : language);
    const cls =
      language !== "texto"
        ? ` class="hljs language-${escapeHtml(language)}"`
        : ` class="hljs"`;
    const langAttr = ` data-lang="${escapeHtml(language)}"`;
    return (
      `<div class="codeblock"${langAttr}>` +
      `<div class="code-head"><span class="code-lang">${escapeHtml(language)}</span>` +
      `<button type="button" class="copy-btn" data-copy="${escapeHtml(text)}" ` +
      `aria-label="Copiar" title="Copiar">${COPY_SVG}</button></div>` +
      `<pre><code${cls}>${highlighted}</code></pre></div>`
    );
  };
  renderer.codespan = ({ text }: { text: string }) => `<code>${escapeHtml(text)}</code>`;
  renderer.link = ({ href, title, text }: { href: string; title?: string | null; text: string }) => {
    const safeHref = escapeHtml(href ?? "");
    const safeTitle = title ? ` title="${escapeHtml(title)}"` : "";
    return `<a href="${safeHref}" target="_blank" rel="noopener"${safeTitle}>${text}</a>`;
  };
  const raw = marked.parse(md, { renderer }) as string;
  return DOMPurify.sanitize(raw, {
    ADD_ATTR: ["target", "rel", "data-copy", "data-lang"],
    FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form"],
    FORBID_ATTR: ["onerror", "onload", "onclick", "onmouseover", "style"],
  });
}

/** Markdown -> HTML saneado, con highlight y botón copiar. */
export function renderMarkdown(md: string): string {
  return buildHtml(md);
}

// ---- render incremental con memoización por bloque (R8) ----
// Divide en bloques por vallas ```: los bloques ya cerrados se memoizan y no
// se re-crean mientras el último (a medias) sigue cambiando en streaming.
const blockCache = new Map<string, string>();

export function splitBlocks(md: string): string[] {
  const lines = md.split("\n");
  const blocks: string[] = [];
  let cur: string[] = [];
  let inFence = false;
  for (const line of lines) {
    if (line.trimStart().startsWith("```")) {
      if (!inFence) {
        if (cur.length > 0) {
          blocks.push(cur.join("\n"));
          cur = [];
        }
        inFence = true;
      } else {
        inFence = false;
      }
      cur.push(line);
      if (!inFence) {
        blocks.push(cur.join("\n"));
        cur = [];
      }
    } else {
      cur.push(line);
    }
  }
  if (cur.length > 0) blocks.push(cur.join("\n"));
  return blocks.filter((b) => b.length > 0);
}

function renderBlock(block: string): string {
  let html = blockCache.get(block);
  if (html === undefined) {
    html = buildHtml(block);
    if (blockCache.size > 400) blockCache.clear();
    blockCache.set(block, html);
  }
  return html;
}

/** Como renderMarkdown pero memoizando bloques cerrados entre llamadas. */
export function renderMarkdownMemo(md: string): string {
  return splitBlocks(md).map(renderBlock).join("");
}

/** Vacía la caché de bloques (tests). */
export function clearBlockCache(): void {
  blockCache.clear();
}

/**
 * Render incremental con throttle rAF: re-renderiza solo el segmento activo.
 * Uso desde view.ts: scheduleRender(el, md) actualiza el innerHTML como
 * mucho una vez por frame.
 */
const pendingRaf = new WeakMap<HTMLElement, number>();
const pendingMd = new WeakMap<HTMLElement, string>();

export function scheduleRender(el: HTMLElement, md: string): void {
  pendingMd.set(el, md);
  if (pendingRaf.has(el)) return;
  const raf =
    typeof requestAnimationFrame !== "undefined"
      ? requestAnimationFrame
      : (cb: FrameRequestCallback) => setTimeout(() => cb(performance.now()), 16);
  const id = raf(() => {
    pendingRaf.delete(el);
    const latest = pendingMd.get(el);
    if (latest !== undefined) {
      el.innerHTML = renderMarkdown(latest);
    }
  });
  pendingRaf.set(el, id as unknown as number);
}

/** Esquemas que se pueden abrir fuera: el resto de enlaces se ignora. */
export function isOpenableUrl(href: string): boolean {
  return /^(https?:|mailto:)/i.test(href.trim());
}

/**
 * El webview no navega a enlaces externos: cualquier clic (normal, Ctrl o central)
 * en un enlace se intercepta y se abre en el navegador del sistema.
 */
export function installLinkHandler(root: ParentNode, open: (url: string) => void): void {
  const handle = (e: Event) => {
    if (e instanceof MouseEvent && e.button > 1) return;
    const a = (e.target as HTMLElement | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    const href = a.getAttribute("href") ?? "";
    if (isOpenableUrl(href)) open(href);
  };
  root.addEventListener("click", handle);
  root.addEventListener("auxclick", handle);
}

/** Delegación de clics para botones Copiar (llamar una vez en main.ts). */
export function installCopyHandler(root: ParentNode = document): void {
  root.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement | null)?.closest?.(".copy-btn") as HTMLElement | null;
    if (!btn) return;
    const text = btn.getAttribute("data-copy") ?? "";
    const original = btn.innerHTML;
    const done = () => {
      btn.textContent = "Copiado";
      setTimeout(() => {
        btn.innerHTML = original;
      }, 1200);
    };
    try {
      const p = navigator.clipboard?.writeText(text);
      if (p && typeof p.then === "function") {
        void p.then(done, done);
      } else {
        done();
      }
    } catch {
      done();
    }
  });
}
