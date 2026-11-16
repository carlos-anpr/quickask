import { describe, expect, it } from "vitest";
import {
  convoTotals,
  estOutTokens,
  fmtCost,
  fmtCostFor,
  fmtElapsed,
  fmtTokens,
  isFreeModel,
  modelLabelFor,
  needsConfirm,
  shortModel,
  showConfirm,
  toolLabel,
  VARIANT_LABELS,
  variantLabel,
  filterModels,
  fold,
} from "./tools";
import { KNOWN_MODELS } from "./contract";

describe("toolLabel", () => {
  it("shell playwright-cli con binario → Navegador sin prefijo", () => {
    expect(toolLabel("shell", "playwright-cli click e12")).toEqual({
      icon: expect.stringContaining("<svg"),
      label: "Navegador: click e12",
    });
  });
  it("shell con título sin binario (BACK B7) → Navegador", () => {
    expect(toolLabel("shell", "open --extension").label).toBe("Navegador: open --extension");
    expect(toolLabel("shell", "snapshot").label).toBe("Navegador: snapshot");
  });
  it("shell normal → Código:", () => {
    expect(toolLabel("shell", "date").label).toBe("Código: date");
  });
  it("skill → Skill:", () => {
    expect(toolLabel("skill", "correo").label).toBe("Skill: correo");
  });
  it("freshservice → Freshservice:", () => {
    expect(toolLabel("mcp-freshservice", "ver ticket FS#123").label).toContain("Freshservice:");
  });
  it("gitlab/glab en title o nombre → GitLab:", () => {
    expect(toolLabel("mcp", "glab mr list").label).toContain("GitLab:");
    expect(toolLabel("gitlab-cli", "list").label).toContain("GitLab:");
  });
  it("jira → Jira:", () => {
    expect(toolLabel("mcp", "jira get issue").label).toContain("Jira:");
  });
  it("exa/websearch/webfetch → Web:", () => {
    expect(toolLabel("exa", "web_search_exa").label).toContain("Web:");
    expect(toolLabel("webfetch", "leer url").label).toContain("Web:");
  });
  it("subagent → Subagente:", () => {
    expect(toolLabel("subagent", "fusion-visual: describir").label).toContain("Subagente:");
  });
  it("read/write/edit → etiqueta tal cual", () => {
    expect(toolLabel("read", "/tmp/a.txt").label).toBe("/tmp/a.txt");
    expect(toolLabel("write", "fichero").label).toBe("fichero");
  });
  it("resto → Código con title; sin title → Código", () => {
    expect(toolLabel("question", "algo").label).toBe("Código: algo");
    expect(toolLabel("otro").label).toBe("Código");
  });
  it("formato execute (R10): recorta el servidor", () => {
    expect(toolLabel("execute", "freshservice get_ticket ticket_id: 12345").label).toBe(
      "Freshservice: get_ticket ticket_id: 12345",
    );
    expect(
      toolLabel("execute", "freshservice get_ticket_conversations ticket_id: 12345").label,
    ).toBe("Freshservice: get_ticket_conversations ticket_id: 12345");
    expect(toolLabel("execute", "gitlab api mr/77").label).toBe("GitLab: api mr/77");
    expect(toolLabel("execute", "jira get_issue key: X").label).toBe("Jira: get_issue key: X");
    expect(toolLabel("execute", "exa web_search q: hola").label).toBe("Exa: web_search q: hola");
    expect(toolLabel("execute", "context7 query-docs react").label).toBe("Context7: query-docs react");
    expect(toolLabel("execute", "discord list_guilds").label).toBe("Discord: list_guilds");
    expect(toolLabel("execute", "drawio search_shapes aws").label).toBe("Drawio: search_shapes aws");
    expect(toolLabel("execute", "markitdown convert_to_markdown x").label).toBe(
      "Markitdown: convert_to_markdown x",
    );
  });
});

describe("formato es-ES", () => {
  it("tokens con k y coma decimal", () => {
    expect(fmtTokens(370)).toBe("370");
    expect(fmtTokens(2681)).toBe("2,7k");
    expect(fmtTokens(12300)).toBe("12,3k");
    expect(fmtTokens(2000)).toBe("2k");
  });
  it("coste en dólares con coma", () => {
    expect(fmtCost(0.0008)).toBe("$0,0008");
    expect(fmtCost(0.0042)).toBe("$0,0042");
  });
  it("tiempos: décima hasta 60 s, luego m:ss", () => {
    expect(fmtElapsed(3400)).toBe("3,4 s");
    expect(fmtElapsed(59000)).toBe("59,0 s");
    expect(fmtElapsed(65000)).toBe("1:05");
    expect(fmtElapsed(125000)).toBe("2:05");
  });
  it("estimación de salida ~tokens", () => {
    expect(estOutTokens(1280)).toBe(320);
    expect(estOutTokens(0)).toBe(0);
  });
  it("coste gratis y etiquetas de modelo", () => {
    expect(fmtCostFor(0, true)).toBe("gratis");
    expect(fmtCostFor(0.0008, true)).toBe("$0,0008");
    expect(fmtCostFor(0, false)).toBe("$0,0000");
    expect(modelLabelFor(KNOWN_MODELS, "opencode/deepseek-v4.1-flash-free")).toBe("DeepSeek V4.1 Flash Free");
    expect(isFreeModel(KNOWN_MODELS, "opencode/deepseek-v4.1-flash-free")).toBe(true);
    expect(isFreeModel(KNOWN_MODELS, "opencode-go/deepseek-v4.1-flash")).toBe(false);
  });
  it("modelo corto y etiquetas de esfuerzo", () => {
    expect(shortModel("opencode-go/deepseek-v4.1-flash")).toBe("deepseek-v4.1-flash");
    expect(VARIANT_LABELS[""]).toBe("Estándar");
    expect(VARIANT_LABELS.low).toBe("Bajo");
    expect(VARIANT_LABELS.high).toBe("Alto");
    expect(VARIANT_LABELS.max).toBe("Máx");
    expect(variantLabel("none")).toBe("Sin razonamiento");
    expect(variantLabel("xhigh")).toBe("Muy alto");
    expect(variantLabel("thinking")).toBe("Razonando");
    expect(variantLabel("turbo")).toBe("Turbo");
  });
  it("filtro de modelos sin tildes ni mayúsculas, por label o id", () => {
    const ms = [
      { id: "opencode-go/kimi-k3", label: "Kimi K3" },
      { id: "opencode-go/glm-5.3", label: "GLM-5.3" },
      { id: "x/minimax", label: "MiniMax Razón" },
    ];
    expect(filterModels(ms, "KIMI").map((m) => m.id)).toEqual(["opencode-go/kimi-k3"]);
    expect(filterModels(ms, "glm-5").map((m) => m.label)).toEqual(["GLM-5.3"]);
    expect(filterModels(ms, "razon").map((m) => m.id)).toEqual(["x/minimax"]);
    expect(filterModels(ms, "  ")).toHaveLength(3);
    expect(fold("Árbol")).toBe("arbol");
  });
  it("totales de conversación", () => {
    expect(
      convoTotals([
        { usage: { cost: 0.0008, tokensIn: 2681, tokensOut: 370 } },
        { usage: { cost: 0.0002, tokensIn: 100, tokensOut: 20 } },
        {},
      ]),
    ).toEqual({ cost: 0.001, tokensIn: 2781, tokensOut: 390 });
    expect(convoTotals([])).toEqual({ cost: 0, tokensIn: 0, tokensOut: 0 });
  });
});
describe("showConfirm", () => {
  const base = {
    outcome: "succeeded" as const,
    lastText: "Borrador listo.\n\n¿Lo publico?",
    isLast: true,
    busy: false,
  };
  it("visible en último turno terminado sin busy", () => {
    expect(showConfirm(base)).toBe(true);
  });
  it("oculto con busy", () => {
    expect(showConfirm({ ...base, busy: true })).toBe(false);
  });
  it("oculto si hay turno posterior", () => {
    expect(showConfirm({ ...base, isLast: false })).toBe(false);
  });
  it("oculto sin outcome", () => {
    expect(showConfirm({ ...base, outcome: undefined })).toBe(false);
  });
  it("oculto sin pregunta de confirmación o sin segmentos", () => {
    expect(showConfirm({ ...base, lastText: "Son las 12:00." })).toBe(false);
    expect(showConfirm({ ...base, lastText: null })).toBe(false);
  });
});

describe("needsConfirm", () => {
  it("detecta ¿Lo hago? / ¿Lo publico? / ¿Lo envío? al final", () => {
    expect(needsConfirm("He preparado el texto.\n\n¿Lo hago?")).toBe(true);
    expect(needsConfirm("Borrador listo.\n\n¿Lo publico?")).toBe(true);
    expect(needsConfirm("Correo redactado.\n\n¿Lo envío?")).toBe(true);
  });
  it("ignora preguntas que no son de confirmación", () => {
    expect(needsConfirm("Resultado:\n\nSon las 12:00.")).toBe(false);
    expect(needsConfirm("¿Qué hora es?")).toBe(false);
  });
  it("solo mira el último párrafo", () => {
    expect(needsConfirm("¿Lo hago?\n\nYa lo he hecho yo.")).toBe(false);
  });
});
