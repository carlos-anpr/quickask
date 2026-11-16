import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { UiEvent } from "./contract";

describe("state reducer", () => {
  it("secuencia completa de un turno", () => {
    let s = initialState();
    s = reduce(s, { kind: "summoned", greeting: "Buenas noches, ¿qué quieres preguntar hoy?" });
    expect(s.visible).toBe(true);
    expect(s.greeting).toContain("Buenas noches");

    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "hola", screen: false });
    expect(s.busy).toBe(true);
    expect(s.turns).toHaveLength(1);

    s = reduce(s, { kind: "reasoning", turnId: "t1", messageId: "m1", delta: "pensando " });
    s = reduce(s, { kind: "reasoning", turnId: "t1", messageId: "m1", delta: "más" });
    expect(s.turns[0].reasoning).toBe("pensando más");

    s = reduce(s, { kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "Hola " });
    s = reduce(s, { kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "mundo" });
    expect(s.turns[0].segments[0].text).toBe("Hola mundo");

    s = reduce(s, {
      kind: "usage",
      turnId: "t1",
      cost: 0.1,
      tokensIn: 10,
      tokensOut: 5,
      tokensReasoning: 2,
      tokensCache: 4,
    });
    expect(s.turns[0].usage?.cost).toBe(0.1);
    expect(s.turns[0].usage?.tokensReasoning).toBe(2);

    s = reduce(s, { kind: "done", turnId: "t1", outcome: "succeeded" });
    expect(s.turns[0].outcome).toBe("succeeded");
    expect(s.busy).toBe(false);
  });

  it("textFinal reemplaza el segmento acumulado", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q", screen: false });
    s = reduce(s, { kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "parcial1 " });
    s = reduce(s, { kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "parcial2 " });
    s = reduce(s, {
      kind: "textFinal",
      turnId: "t1",
      messageId: "m1",
      ordinal: 0,
      text: "FINAL reconciliado",
    });
    expect(s.turns[0].segments[0].text).toBe("FINAL reconciliado");
    expect(s.turns[0].segments[0].final).toBe(true);
    s = reduce(s, { kind: "text", turnId: "t1", messageId: "m1", ordinal: 0, delta: "tardío" });
    expect(s.turns[0].segments[0].text).toBe("FINAL reconciliado");
  });

  it("tools por callId cambian de estado", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q", screen: false });
    s = reduce(s, { kind: "tool", turnId: "t1", callId: "c1", name: "shell", status: "running", title: "date" });
    s = reduce(s, { kind: "tool", turnId: "t1", callId: "c1", name: "shell", status: "ok", title: "date" });
    expect(s.turns[0].tools).toHaveLength(1);
    expect(s.turns[0].tools[0].status).toBe("ok");
  });

  it("permission y permissionClosed", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q", screen: false });
    s = reduce(s, { kind: "permission", turnId: "t1", requestId: "per_1", action: "shell", resources: ["date"] });
    expect(s.turns[0].permissions).toHaveLength(1);
    s = reduce(s, { kind: "permissionClosed", requestId: "per_1" });
    expect(s.turns[0].permissions).toHaveLength(0);
  });

  it("conversationReset vacía", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q", screen: false });
    s = reduce(s, { kind: "conversationReset" });
    expect(s.turns).toHaveLength(0);
    expect(s.busy).toBe(false);
  });

  it("eventos de otro turnId no mezclan", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q1", screen: false });
    const before = JSON.stringify(s);
    s = reduce(s, { kind: "text", turnId: "otro", messageId: "m9", ordinal: 0, delta: "x" });
    expect(JSON.stringify(s)).toBe(before);
    s = reduce(s, { kind: "done", turnId: "otro", outcome: "succeeded" });
    expect(s.turns[0].outcome).toBeUndefined();
    expect(s.busy).toBe(true);
  });

  it("secuencia equivalente a fixtures/sse-turno-con-subagente-y-vision.jsonl", () => {
    const turnId = "t-vis";
    const mid = "msg_vis";
    const callId = "call_subagent_1";
    const finalText =
      "Captura de pantalla de un escritorio Linux con una ventana multiplexada de terminal en tres columnas.\n\nHerramientas/skills: `herdr`, `playwright-cli`.";
    const events: UiEvent[] = [
      { kind: "turnStarted", turnId, text: "Describe en una frase qué se ve en la imagen", screen: true },
      { kind: "screen", turnId, status: "capturing" },
      { kind: "screen", turnId, status: "attached", thumb: "data:image/jpeg;base64,MOCK" },
      { kind: "reasoning", turnId, messageId: mid, delta: "The user wants a one-sentence description " },
      { kind: "reasoning", turnId, messageId: mid, delta: "of the image and 2 tool/skill names. " },
      { kind: "reasoning", turnId, messageId: mid, delta: "Delegate to fusion-visual." },
      { kind: "tool", turnId, callId, name: "subagent", status: "pending", title: "fusion-visual: Describir captura" },
      { kind: "tool", turnId, callId, name: "subagent", status: "running", title: "fusion-visual: Describir captura" },
      { kind: "tool", turnId, callId, name: "subagent", status: "ok", title: "fusion-visual: Describir captura" },
      { kind: "text", turnId, messageId: mid, ordinal: 0, delta: "Captura de pantalla de un escritorio Linux " },
      { kind: "text", turnId, messageId: mid, ordinal: 0, delta: "con una ventana multiplexada de terminal en tres columnas.\n\n" },
      { kind: "text", turnId, messageId: mid, ordinal: 0, delta: "Herramientas/skills: `herdr`, " },
      { kind: "text", turnId, messageId: mid, ordinal: 0, delta: "`playwright-cli`." },
      { kind: "textFinal", turnId, messageId: mid, ordinal: 0, text: finalText },
      { kind: "usage", turnId, cost: 0.000820662, tokensIn: 2681, tokensOut: 370, tokensReasoning: 124, tokensCache: 40704 },
      { kind: "done", turnId, outcome: "succeeded" },
    ];
    let s = initialState();
    for (const e of events) s = reduce(s, e);
    expect(s.turns).toHaveLength(1);
    const t = s.turns[0];
    expect(t.screenStatus).toBe("attached");
    expect(t.reasoning).toContain("fusion-visual");
    expect(t.tools).toHaveLength(1);
    expect(t.tools[0].name).toBe("subagent");
    expect(t.tools[0].status).toBe("ok");
    expect(t.segments[0].text).toBe(finalText);
    expect(t.outcome).toBe("succeeded");
    expect(t.usage?.tokensIn).toBe(2681);
    expect(t.usage?.tokensReasoning).toBe(124);
  });

  it("usage trae el acumulado del turno: el segundo reemplaza", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q", screen: false });
    s = reduce(s, {
      kind: "usage", turnId: "t1", cost: 0.000287556, tokensIn: 58,
      tokensOut: 239, tokensReasoning: 124, tokensCache: 20352,
    });
    s = reduce(s, {
      kind: "usage", turnId: "t1", cost: 0.000820662, tokensIn: 2681,
      tokensOut: 370, tokensReasoning: 124, tokensCache: 40704,
    });
    expect(s.turns[0].usage).toEqual({
      cost: 0.000820662, tokensIn: 2681, tokensOut: 370,
      tokensReasoning: 124, tokensCache: 40704,
    });
  });

  it("variant actualiza el esfuerzo global", () => {
    let s = initialState();
    expect(s.variant).toBe("");
    s = reduce(s, { kind: "variant", variant: "high" });
    expect(s.variant).toBe("high");
  });

  it("model actualiza modelo y relevo", () => {
    let s = initialState();
    expect(s.model).toBe("opencode/deepseek-v4.1-flash-free");
    expect(s.fallbackModel).toBe("");
    s = reduce(s, {
      kind: "model",
      model: "opencode-go/kimi-k3",
      fallbackModel: "opencode/deepseek-v4.1-flash-free",
    });
    expect(s.model).toBe("opencode-go/kimi-k3");
    expect(s.fallbackModel).toBe("opencode/deepseek-v4.1-flash-free");
  });

  it("notice se añade al turno (ignora turnId desconocido)", () => {
    let s = initialState();
    s = reduce(s, { kind: "turnStarted", turnId: "t1", text: "q", screen: false });
    s = reduce(s, { kind: "notice", turnId: "t1", text: "Flash Free no disponible; reintentando con Flash Go" });
    expect(s.turns[0].notices).toHaveLength(1);
    const before = JSON.stringify(s);
    s = reduce(s, { kind: "notice", turnId: "otro", text: "x" });
    expect(JSON.stringify(s)).toBe(before);
  });
});
