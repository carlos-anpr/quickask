import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

describe("markdown", () => {
  it("elimina script y onerror", () => {
    const html = renderMarkdown('hola <script>alert(1)</script> <img src="x" onerror="alert(1)">');
    expect(html).not.toContain("<script");
    expect(html).not.toContain("onerror");
  });

  it("enlaces con rel noopener y target blank", () => {
    const html = renderMarkdown("[Tauri](https://tauri.app)");
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener"');
  });

  it("bloque de código con botón copiar (icono) y clase hljs", () => {
    const html = renderMarkdown("```rust\nfn main() {}\n```");
    expect(html).toContain("copy-btn");
    expect(html).toContain("hljs");
    expect(html).toContain('aria-label="Copiar"');
    expect(html).toContain("code-head");
    expect(html).toContain("rust");
  });
});

describe("installLinkHandler", () => {
  it("abre fuera los enlaces web con clic, Ctrl+clic y clic central; ignora otros esquemas", async () => {
    const { installLinkHandler } = await import("./markdown");
    const root = document.createElement("div");
    root.innerHTML =
      '<a href="https://gitlab.example/x"><b>MR</b></a><a href="file:///etc/passwd">f</a>';
    const opened: string[] = [];
    installLinkHandler(root, (u) => opened.push(u));
    const [web, file] = Array.from(root.querySelectorAll("a"));
    const b = web.querySelector("b")!;
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    b.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    web.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ctrlKey: true }));
    web.dispatchEvent(new MouseEvent("auxclick", { bubbles: true, cancelable: true, button: 1 }));
    const blocked = new MouseEvent("click", { bubbles: true, cancelable: true });
    file.dispatchEvent(blocked);
    expect(blocked.defaultPrevented).toBe(true);
    expect(opened).toEqual(Array(3).fill("https://gitlab.example/x"));
  });
});
