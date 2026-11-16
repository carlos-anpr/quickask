import { afterEach, describe, expect, it } from "vitest";
import { createMockBackend } from "./bridge";
import { createApp } from "./view";

function tick(ms = 60): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

describe("render keyed", () => {
  let apps: Array<{ destroy: () => void; root: HTMLElement }> = [];
  afterEach(() => {
    for (const a of apps) {
      a.destroy();
      a.root.remove();
    }
    apps = [];
  });

  it("un text delta NO recrea el nodo del turno ni del segmento", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });

    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    await tick();
    const turn1 = root.querySelector('[data-turn="t1"]');
    expect(turn1).not.toBeNull();

    app.handleEvent({ kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "Hola " });
    await tick();
    expect(root.querySelector('[data-turn="t1"]')).toBe(turn1);
    const seg1 = root.querySelector('[data-seg="t1:m1:0"]');
    expect(seg1).not.toBeNull();

    app.handleEvent({ kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "mundo" });
    await tick();
    expect(root.querySelector('[data-turn="t1"]')).toBe(turn1);
    expect(root.querySelector('[data-seg="t1:m1:0"]')).toBe(seg1);
    expect(seg1?.textContent).toContain("mundo");
  });

  it("ctx sigue al esfuerzo y se limpia al reiniciar", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });
    await tick();
    const ctx = root.querySelector(".context-usage")!;
    expect(ctx.previousElementSibling?.classList.contains("effort-wrap")).toBe(true);
    expect(ctx.textContent).toBe("ctx —");
    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    app.handleEvent({ kind: "usage", turnId: "t1", cost: 0, tokensIn: 99999,
      tokensOut: 100, tokensReasoning: 0, tokensCache: 0, tokensContext: 23106 });
    await tick();
    expect(ctx.textContent).toBe("ctx 23,1k");
    app.handleEvent({ kind: "done", turnId: "t1", outcome: "succeeded" });
    app.handleEvent({ kind: "turnStarted", turnId: "t2", text: "otra", screen: false });
    await tick();
    expect(ctx.textContent).toBe("ctx 23,1k");
    app.handleEvent({ kind: "conversationReset" });
    await tick();
    expect(ctx.textContent).toBe("ctx —");
  });

  it("un segundo turno no toca el nodo del primero", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });

    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "uno", screen: false });
    await tick();
    const turn1 = root.querySelector('[data-turn="t1"]');
    app.handleEvent({ kind: "turnStarted", turnId: "t2", text: "dos", screen: false });
    await tick();
    expect(root.querySelector('[data-turn="t1"]')).toBe(turn1);
    expect(root.querySelectorAll(".turn")).toHaveLength(2);
  });
});

describe("rev9", () => {
  let apps: Array<{ destroy: () => void; root: HTMLElement }> = [];
  afterEach(() => {
    for (const a of apps) {
      a.destroy();
      a.root.remove();
    }
    apps = [];
  });

  function setup() {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });
    return { app, root };
  }

  it("capturing→attached quita el aviso y el pie solo sale con outcome", async () => {
    const { app, root } = setup();
    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "¿qué hay en pantalla?", screen: true });
    app.handleEvent({ kind: "screen", turnId: "t1", status: "capturing" });
    await tick();
    expect(root.querySelector(".screen-status")?.textContent).toContain("Capturando");
    // pie oculto mientras no hay outcome
    const foot = root.querySelector('[data-turn="t1"] .foot') as HTMLElement;
    expect(foot.style.display).toBe("none");

    app.handleEvent({ kind: "screen", turnId: "t1", status: "attached", thumb: "data:image/png;base64,x" });
    await tick(250);
    expect(root.querySelector(".screen-status")).toBeNull();
    expect(root.querySelector(".thumb-mini")).not.toBeNull();

    app.handleEvent({ kind: "done", turnId: "t1", outcome: "succeeded" });
    await tick();
    expect((root.querySelector('[data-turn="t1"] .foot') as HTMLElement).style.display).toBe("");
  });

  it("en busy sin usage se ve '↑ —' y con usage el valor real", async () => {    const { app, root } = setup();
    await tick(); // deja que getState() inicial resuelva antes del turno
    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    await tick();
    expect(root.querySelector(".center")?.textContent).toContain("↑ —");
    app.handleEvent({
      kind: "usage", turnId: "t1", cost: 0.0008, tokensIn: 2681,
      tokensOut: 370, tokensReasoning: 0, tokensCache: 0,
    });
    await tick();
    const center = root.querySelector(".center")?.textContent ?? "";
    expect(center).toContain("↑ 2,7k");
    expect(center).not.toContain("↑ —");
  });
});

describe("rev10", () => {
  let apps: Array<{ destroy: () => void; root: HTMLElement }> = [];
  afterEach(() => {
    for (const a of apps) {
      a.destroy();
      a.root.remove();
    }
    apps = [];
  });

  function setup() {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });
    return { app, root };
  }

  it("al pasar a ok el spinner desaparece del chip", async () => {
    const { app, root } = setup();
    await tick();
    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    app.handleEvent({ kind: "tool", turnId: "t1", callId: "c1", name: "shell", status: "running", title: "date" });
    await tick();
    const chip = root.querySelector('[data-turn="t1"] .tool');
    expect(chip?.querySelector(".spin")).not.toBeNull();
    app.handleEvent({ kind: "tool", turnId: "t1", callId: "c1", name: "shell", status: "ok", title: "date" });
    await tick();
    expect(root.querySelector('[data-turn="t1"] .tool .spin')).toBeNull();
    expect(root.querySelector('[data-turn="t1"] .tool')?.textContent).toContain("✓");
  });

  it("el spinner de la barra persiste entre ticks del temporizador", async () => {
    const { app, root } = setup();
    await tick();
    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    await tick(150);
    const spin1 = root.querySelector(".center .spin");
    expect(spin1).not.toBeNull();
    app.handleEvent({ kind: "reasoning", turnId: "t1", messageId: "m1", delta: "pensando " });
    await tick(150);
    expect(root.querySelector(".center .spin")).toBe(spin1);
  });
});

describe("rev11", () => {
  let apps: Array<{ destroy: () => void; root: HTMLElement }> = [];
  afterEach(() => {
    for (const a of apps) {
      a.destroy();
      a.root.remove();
    }
    apps = [];
  });

  function setup() {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });
    return { app, root };
  }

  it("chip de modelo: el gratuito por defecto; con evento model cambia y muestra el relevo", async () => {
    const { app, root } = setup();
    await tick();
    const btn = root.querySelector(".model-btn") as HTMLElement;
    expect(btn.textContent).toContain("gratis");
    expect(btn.title).not.toContain("si falla");
    app.handleEvent({
      kind: "model",
      model: "opencode-go/deepseek-v4.1-flash",
      fallbackModel: "opencode/deepseek-v4.1-flash-free",
    });
    await tick();
    expect(btn.textContent).toContain("DeepSeek V4.1 Flash");
    expect(btn.textContent).not.toContain("gratis");
    expect(btn.title).toContain("DeepSeek V4.1 Flash Free");
  });

  it("menú de modelos: busca, navega con teclado y elige", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const chosen: string[] = [];
    const orig = backend.setModel.bind(backend);
    backend.setModel = async (m: string) => {
      chosen.push(m);
      await orig(m);
    };
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });
    await tick();
    (root.querySelector(".model-btn") as HTMLElement).click();
    const menu = root.querySelector(".model-menu") as HTMLElement;
    expect(menu.hidden).toBe(false);
    expect(menu.querySelectorAll(".menu-row").length).toBeGreaterThanOrEqual(8);
    const search = menu.querySelector(".model-search") as HTMLInputElement;
    search.value = "KIMI";
    search.dispatchEvent(new Event("input"));
    const rows = menu.querySelectorAll(".menu-row");
    expect([...rows].map((r) => (r as HTMLElement).dataset.id)).toEqual(["opencode-go/kimi-k2.7-code", "opencode-go/kimi-k3"]);
    search.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown" }));
    search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await tick();
    expect(chosen).toEqual(["opencode-go/kimi-k3"]);
    expect(menu.hidden).toBe(true);
  });

  it("esfuerzo: opciones del modelo actual y chip atenuado si no tiene", async () => {
    const { app, root } = setup();
    await tick();
    app.handleEvent({ kind: "model", model: "opencode-go/claude-haiku-5-5", fallbackModel: "opencode/deepseek-v4.1-flash-free" });
    await tick();
    const effort = root.querySelector(".effort") as HTMLElement;
    effort.click();
    const menu = root.querySelector(".effort-menu") as HTMLElement;
    expect([...menu.querySelectorAll(".row-name")].map((r) => r.textContent)).toEqual([
      "Estándar", "Sin razonamiento", "Bajo", "Medio", "Alto", "Muy alto", "Máx",
    ]);
    app.handleEvent({ kind: "model", model: "opencode-go/kimi-k2.7-code", fallbackModel: "opencode/deepseek-v4.1-flash-free" });
    await tick();
    expect(effort.classList.contains("muted")).toBe(true);
    expect(effort.title).toContain("no tiene niveles");
  });

  it("notice aparece como línea discreta y coste 0 free es gratis", async () => {
    const { app, root } = setup();
    await tick();
    app.handleEvent({ kind: "model", model: "opencode/deepseek-v4.1-flash-free", fallbackModel: "opencode-go/deepseek-v4.1-flash" });
    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    app.handleEvent({ kind: "notice", turnId: "t1", text: "Flash Free no disponible; reintentando con Flash Go" });
    app.handleEvent({
      kind: "usage", turnId: "t1", cost: 0, tokensIn: 10,
      tokensOut: 5, tokensReasoning: 0, tokensCache: 0,
    });
    app.handleEvent({ kind: "done", turnId: "t1", outcome: "succeeded" });
    await tick();
    expect(root.querySelector(".notice")?.textContent).toContain("reintentando con Flash Go");
    expect(root.querySelector('[data-turn="t1"] .foot')?.textContent).toContain("gratis");
    expect(root.querySelector(".center")?.textContent).toContain("gratis");
  });

  it("la actividad va antes de la respuesta aunque haya texto antes de la 1ª herramienta, y se pliega al terminar", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(root, backend);
    apps.push({ destroy: app.destroy, root });

    app.handleEvent({ kind: "turnStarted", turnId: "t1", text: "busca", screen: false });
    app.handleEvent({ kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "Voy a buscar." });
    await tick();
    app.handleEvent({ kind: "tool", turnId: "t1", callId: "c1", name: "shell", status: "ok", title: "ls" });
    app.handleEvent({ kind: "text", turnId: "t1", messageId: "m2", ordinal: 0, delta: "RESPUESTA FINAL" });
    app.handleEvent({ kind: "done", turnId: "t1", outcome: "succeeded" });
    await tick();
    const turn = root.querySelector('[data-turn="t1"]') as HTMLElement;
    const activity = turn.querySelector(".activity") as HTMLDetailsElement;
    const answers = turn.querySelector(".answers") as HTMLElement;
    expect(activity.compareDocumentPosition(answers) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(answers.textContent).toContain("RESPUESTA FINAL");
    expect(activity.open).toBe(false);
    expect(activity.querySelector(".activity-sum")?.textContent).toContain("1 acción");
  });
});
