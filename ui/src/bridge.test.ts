import { describe, expect, it } from "vitest";
import { createMockBackend, needsScreenLocal } from "./bridge";
import type { UiEvent } from "./contract";

function collect(turnId?: string): { events: UiEvent[]; waitDone: Promise<UiEvent> } {
  const events: UiEvent[] = [];
  let resolveDone!: (ev: UiEvent) => void;
  const waitDone = new Promise<UiEvent>((r) => {
    resolveDone = r;
  });
  return {
    events,
    waitDone,
    // el caller suscribe y empuja; helper simple:
    push(ev: UiEvent) {
      events.push(ev);
      if (turnId !== undefined && ev.kind === "done" && ev.turnId === turnId) resolveDone(ev);
      else if (turnId === undefined && ev.kind === "done") resolveDone(ev);
    },
  } as unknown as { events: UiEvent[]; waitDone: Promise<UiEvent> };
}

describe("bridge mock", () => {
  it("pregunta normal: turnStarted, reasoning, tools, text, usage, done", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 6 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
    });
    const { turnId } = await backend.ask("hola, ¿qué hora es?", "off");
    const done = await new Promise<UiEvent>((resolve) => {
      const check = () => {
        const d = events.find((e) => e.kind === "done" && e.turnId === turnId);
        if (d) resolve(d);
        else setTimeout(check, 5);
      };
      check();
    });
    un();
    expect(events[0]).toMatchObject({ kind: "turnStarted", turnId });
    expect(events.some((e) => e.kind === "reasoning")).toBe(true);
    const tools = events.filter((e) => e.kind === "tool");
    expect(tools.length).toBeGreaterThanOrEqual(4);
    expect(tools.some((e) => e.kind === "tool" && e.name === "shell" && e.status === "ok")).toBe(true);
    expect(tools.some((e) => e.kind === "tool" && e.name === "subagent" && e.status === "ok")).toBe(true);
    expect(events.some((e) => e.kind === "text")).toBe(true);
    expect(events.some((e) => e.kind === "textFinal")).toBe(true);
    expect(events.some((e) => e.kind === "usage")).toBe(true);
    expect(done).toMatchObject({ kind: "done", outcome: "succeeded" });
  });

  it("pregunta con pantalla: capturing -> attached con miniatura", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 4 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
    });
    const { turnId } = await backend.ask("¿qué pone en esta pantalla?", "auto");
    await new Promise<UiEvent>((resolve) => {
      const check = () => {
        const d = events.find((e) => e.kind === "done" && e.turnId === turnId);
        if (d) resolve(d);
        else setTimeout(check, 5);
      };
      check();
    });
    un();
    expect(needsScreenLocal("¿qué pone en esta pantalla?")).toBe(true);
    const screens = events.filter((e) => e.kind === "screen");
    expect(screens.map((e) => (e.kind === "screen" ? e.status : ""))).toEqual([
      "capturing",
      "attached",
    ]);
    const attached = screens[1];
    expect(attached.kind === "screen" && attached.thumb?.startsWith("data:image")).toBe(true);
  });

  it("pregunta con permiso: emite permission y espera replyPermission", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 4 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
      if (ev.kind === "permission") {
        void backend.replyPermission(ev.requestId, "once");
      }
    });
    const { turnId } = await backend.ask("dame permiso para usar shell", "off");
    await new Promise<UiEvent>((resolve) => {
      const check = () => {
        const d = events.find((e) => e.kind === "done" && e.turnId === turnId);
        if (d) resolve(d);
        else setTimeout(check, 5);
      };
      check();
    });
    un();
    expect(events.some((e) => e.kind === "permission")).toBe(true);
    expect(events.some((e) => e.kind === "permissionClosed")).toBe(true);
    expect(events.some((e) => e.kind === "done" && e.turnId === turnId)).toBe(true);
  });

  it("pregunta con error: done failed", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 4 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
    });
    const { turnId } = await backend.ask("provoca un error, por favor", "off");
    const done = await new Promise<UiEvent>((resolve) => {
      const check = () => {
        const d = events.find((e) => e.kind === "done" && e.turnId === turnId);
        if (d) resolve(d);
        else setTimeout(check, 5);
      };
      check();
    });
    un();
    expect(done).toMatchObject({ kind: "done", outcome: "failed" });
  });

  it("setVariant emite variant y getState lo refleja", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
    });
    await backend.setVariant("high");
    expect(events).toContainEqual({ kind: "variant", variant: "high" });
    expect((await backend.getState()).variant).toBe("high");
    un();
  });

  it("setModel emite model; modelo desconocido rechaza", async () => {
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 2 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
    });
    await backend.setModel("opencode-go/kimi-k3");
    expect(events).toContainEqual({
      kind: "model",
      model: "opencode-go/kimi-k3",
      fallbackModel: "",
    });
    await expect(backend.setModel("no-existe")).rejects.toThrow();
    un();
  });

  it("needsScreenLocal: heurística PLAN §4.1", () => {
    expect(needsScreenLocal("¿qué se ve en la pantalla?")).toBe(true);
    expect(needsScreenLocal("haz una captura de esto")).toBe(true);
    expect(needsScreenLocal("mira lo que tengo delante")).toBe(true);
    expect(needsScreenLocal("¿cuánto es 2+2?")).toBe(false);
  });

  it("construye secuencia UiEvent desde el caso sse con subagente y visión", async () => {
    // Replica la traducción BACK (PLAN §2.3 -> §4) del fixture
    // fixtures/sse-turno-con-subagente-y-vision.jsonl: el mock ya emite
    // la secuencia equivalente (reasoning + tool subagent + text + usage + done).
    const backend = createMockBackend({ deltaDelayMs: 1, deltas: 4 });
    const events: UiEvent[] = [];
    const un = backend.onEvent((ev) => {
      events.push(ev);
    });
    const { turnId } = await backend.ask("describe lo que sale en pantalla", "on");
    await new Promise<UiEvent>((resolve) => {
      const check = () => {
        const d = events.find((e) => e.kind === "done" && e.turnId === turnId);
        if (d) resolve(d);
        else setTimeout(check, 5);
      };
      check();
    });
    un();
    const kinds = events.map((e) => e.kind);
    expect(kinds[0]).toBe("turnStarted");
    expect(kinds).toContain("screen");
    expect(kinds).toContain("reasoning");
    expect(kinds).toContain("tool");
    expect(kinds).toContain("text");
    expect(kinds).toContain("textFinal");
    expect(kinds).toContain("usage");
    expect(kinds[kinds.length - 1]).toBe("done");
  });
});

export { collect };
