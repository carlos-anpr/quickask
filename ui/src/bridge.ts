// bridge.ts — invoke/listen reales + MOCK para desarrollo en navegador.
// Real si '__TAURI_INTERNALS__' in window; si no, mock que simula el backend.
import { QA_EVENT } from "./contract";
import type {
  AskResult,
  BackendState,
  BackendStatus,
  ModelInfo,
  PermissionDecision,
  ScreenMode,
  UiEvent,
  Variant,
} from "./contract";
import { DEFAULT_FALLBACK_MODEL, DEFAULT_MODEL, KNOWN_MODELS } from "./contract";

export type EventCallback = (ev: UiEvent) => void;
export type Unlisten = () => void;

export interface Backend {
  ask(text: string, screen: ScreenMode): Promise<AskResult>;
  stop(): Promise<void>;
  replyPermission(requestId: string, decision: PermissionDecision): Promise<void>;
  newConversation(): Promise<void>;
  hide(): Promise<void>;
  resize(height: number): Promise<void>;
  needsScreen(text: string): Promise<boolean>;
  getState(): Promise<BackendState>;
  openInTerminal(): Promise<void>;
  openUrl(url: string): Promise<void>;
  setVariant(variant: Variant): Promise<void>;
  setModel(model: string): Promise<void>;
  onEvent(cb: EventCallback): Unlisten;
}

export function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in (window as unknown as Record<string, unknown>)
  );
}

// ---- needsScreen local (misma heurística que BACK PLAN §4.1) ----
const NEEDLE_SUBSTRINGS = [
  "pantalla",
  "en pantalla",
  "esta ventana",
  "esta pagina",
  "esta web",
  "lo que pone",
  "lo que sale",
  "lo que veo",
  "lo que ves",
  "lo que aparece",
  "que pone aqui",
  "esto que",
  "este error",
  "este mensaje",
  "screen",
  "captura",
  "lo que tengo delante",
  "que estoy viendo",
];

export function normalizeForScreen(s: string): string {
  return s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "");
}

export function needsScreenLocal(text: string): boolean {
  const n = normalizeForScreen(text);
  for (const needle of NEEDLE_SUBSTRINGS) {
    if (n.includes(needle)) return true;
  }
  return false;
}

// ---- backend Tauri real ----
function createTauriBackend(): Backend {
  // Importes diferidos para no cargar el runtime Tauri en navegador/mock.
  const getCore = () => import("@tauri-apps/api/core");
  const getEvent = () => import("@tauri-apps/api/event");
  return {
    async ask(text: string, screen: ScreenMode): Promise<AskResult> {
      const { invoke } = await getCore();
      return invoke<AskResult>("ask", { text, screen });
    },
    async stop(): Promise<void> {
      const { invoke } = await getCore();
      await invoke("stop");
    },
    async replyPermission(requestId: string, decision: PermissionDecision): Promise<void> {
      const { invoke } = await getCore();
      await invoke("replyPermission", { requestId, decision });
    },
    async newConversation(): Promise<void> {
      const { invoke } = await getCore();
      await invoke("newConversation");
    },
    async hide(): Promise<void> {
      const { invoke } = await getCore();
      await invoke("hide");
    },
    async resize(height: number): Promise<void> {
      const { invoke } = await getCore();
      await invoke("resize", { height });
    },
    async needsScreen(text: string): Promise<boolean> {
      const { invoke } = await getCore();
      return invoke<boolean>("needsScreen", { text });
    },
    async getState(): Promise<BackendState> {
      const { invoke } = await getCore();
      return invoke<BackendState>("getState");
    },
    async openInTerminal(): Promise<void> {
      const { invoke } = await getCore();
      await invoke("openInTerminal");
    },
    async openUrl(url: string): Promise<void> {
      const { invoke } = await getCore();
      await invoke("openUrl", { url });
    },
    async setVariant(variant: Variant): Promise<void> {
      const { invoke } = await getCore();
      await invoke("setVariant", { variant });
    },
    async setModel(model: string): Promise<void> {
      const { invoke } = await getCore();
      await invoke("setModel", { model });
    },
    onEvent(cb: EventCallback): Unlisten {
      let unlisten: Unlisten | undefined;
      let cancelled = false;
      void getEvent().then(({ listen }) =>
        listen<UiEvent>(QA_EVENT, (e) => cb(e.payload)),
      ).then((u) => {
        if (cancelled) u();
        else unlisten = u;
      });
      return () => {
        cancelled = true;
        unlisten?.();
      };
    },
  };
}

// ---- mock ----
export const MOCK_MARKDOWN = [
  "# Respuesta de ejemplo",
  "",
  "Esto es una respuesta simulada del mock con Markdown variado:",
  "",
  "- punto uno: resumen directo",
  "- punto dos: con `código inline`",
  "- punto tres: enlace a [Tauri](https://tauri.app)",
  "",
  "## Tabla",
  "",
  "| Campo | Valor |",
  "| --- | --- |",
  "| modelo | deepseek-v4.1-flash |",
  "| coste | 0.0008 |",
  "",
  "```rust",
  'fn saludar() -> &\'static str {',
  '    "Buenas noches, ¿qué quieres preguntar hoy?"',
  "}",
  "```",
  "",
  "Fin del ejemplo.",
].join("\n");

function makeGrayThumb(): string {
  try {
    if (typeof document !== "undefined" && typeof HTMLCanvasElement !== "undefined") {
      // jsdom no implementa canvas: ir directo al fallback sin ruido en stderr
      const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";
      if (!ua.includes("jsdom")) {
        const c = document.createElement("canvas");
        c.width = 320;
        c.height = 180;
        const ctx = c.getContext("2d");
        if (ctx) {
          ctx.fillStyle = "#888888";
          ctx.fillRect(0, 0, c.width, c.height);
          ctx.fillStyle = "#666666";
          for (let y = 0; y < c.height; y += 18) {
            ctx.fillRect(0, y, c.width, 2);
          }
          const url = c.toDataURL("image/jpeg", 0.7);
          if (url.startsWith("data:image")) return url;
        }
      }
    }
  } catch {
    // jsdom sin canvas: fallback abajo
  }
  // PNG 1x1 gris como fallback (válido como data URL)
  return (
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ" +
    "AAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
  );
}

export interface MockOptions {
  deltaDelayMs?: number;
  deltas?: number;
}

let mockTurnCounter = 0;

export function createMockBackend(opts: MockOptions = {}): Backend & { reset(): void } {
  const deltaDelayMs = opts.deltaDelayMs ?? 40;
  const nDeltas = opts.deltas ?? 30;
  const listeners = new Set<EventCallback>();
  const pendingPermissions = new Map<string, () => void>();
  let reqCounter = 0;
  let stopped = false;
  let mockVariant: Variant = "";
  let mockModel = DEFAULT_MODEL;
  let mockFallback = DEFAULT_FALLBACK_MODEL;
  // Catálogo variado como el real de OpenCode Go (con/sin variantes, gratis, sin visión).
  const mockModels: ModelInfo[] = [
    ...KNOWN_MODELS.map((m) => ({ ...m, variants: [...m.variants] })),
    { id: "opencode-go/deepseek-v4.1-flash", label: "DeepSeek V4.1 Flash", free: false, vision: true, variants: ["low", "high", "max"] },
    { id: "opencode-go/kimi-k3", label: "Kimi K3", free: false, vision: true, variants: ["max"] },
    { id: "opencode-go/claude-haiku-5-5", label: "Claude Haiku 5.5", free: false, vision: true, variants: ["none", "low", "medium", "high", "xhigh", "max"] },
    { id: "opencode-go/minimax-m3", label: "MiniMax-M3", free: false, vision: true, variants: ["none", "thinking"] },
    { id: "opencode-go/kimi-k2.7-code", label: "Kimi K2.7 Code", free: false, vision: true, variants: [] },
    { id: "opencode-go/cat-jam-free", label: "Cat Jam Free", free: true, vision: true, variants: ["low", "medium", "high", "xhigh", "max"] },
    { id: "opencode-go/texto-solo", label: "Modelo Sólo Texto", free: false, vision: false, variants: ["low", "high"] },
  ].sort((a, b) => a.label.localeCompare(b.label));

  function emit(ev: UiEvent): void {
    for (const cb of [...listeners]) {
      try {
        cb(ev);
      } catch {
        // un listener no debe romper a los demás
      }
    }
  }

  function sleep(ms: number): Promise<void> {
    return new Promise((r) => setTimeout(r, ms));
  }

  function splitDeltas(full: string, n: number): string[] {
    if (n <= 1) return [full];
    const out: string[] = [];
    const size = Math.max(1, Math.ceil(full.length / n));
    for (let i = 0; i < full.length; i += size) out.push(full.slice(i, i + size));
    while (out.length < n) out.push("");
    return out.slice(0, n);
  }

  async function runTurn(turnId: string, text: string, screenMode: ScreenMode): Promise<void> {
    stopped = false;
    const lower = text.toLowerCase();
    const wantScreen =
      screenMode === "on" ? true : screenMode === "off" ? false : needsScreenLocal(text);
    const messageId = `msg_${turnId}`;
    const fail = lower.includes("error");
    const wantPermission = lower.includes("permiso");

    emit({ kind: "turnStarted", turnId, text, screen: wantScreen });
    if (wantScreen) {
      emit({ kind: "screen", turnId, status: "capturing" });
      await sleep(Math.min(deltaDelayMs, 50));
      if (stopped) return;
      emit({ kind: "screen", turnId, status: "attached", thumb: makeGrayThumb() });
    }
    emit({ kind: "reasoning", turnId, messageId, delta: "Pensando cómo responder… " });
    await sleep(deltaDelayMs);

    if (wantPermission) {
      reqCounter += 1;
      const requestId = `per_mock${reqCounter}`;
      // Puerta creada ANTES de emitir: el listener puede responder de forma
      // síncrona dentro del emit y si no, el resume aún no existiría (deadlock).
      const gate = new Promise<void>((resolve) => pendingPermissions.set(requestId, resolve));
      // Emitir en microtarea para que el gate esté siempre registrado.
      await sleep(0);
      emit({ kind: "permission", turnId, requestId, action: "shell", resources: ["date"] });
      await gate;
      if (stopped) return;
    }

    const shellCall = `call_${turnId}_shell`;
    const subCall = `call_${turnId}_sub`;
    emit({ kind: "tool", turnId, callId: shellCall, name: "shell", status: "running", title: "date" });
    await sleep(deltaDelayMs);
    if (stopped) return;
    emit({ kind: "tool", turnId, callId: shellCall, name: "shell", status: "ok", title: "date" });
    emit({ kind: "tool", turnId, callId: subCall, name: "subagent", status: "running", title: "fusion-visual: describir captura" });
    await sleep(deltaDelayMs);
    if (stopped) return;
    emit({ kind: "tool", turnId, callId: subCall, name: "subagent", status: "ok", title: "fusion-visual: describir captura" });

    const deltas = splitDeltas(MOCK_MARKDOWN, nDeltas);
    for (const d of deltas) {
      if (stopped) return;
      emit({ kind: "text", turnId, messageId, ordinal: 0, delta: d });
      await sleep(deltaDelayMs);
    }
    emit({ kind: "textFinal", turnId, messageId, ordinal: 0, text: MOCK_MARKDOWN });
    emit({
      kind: "usage",
      turnId,
      cost: 0.0008,
      tokensIn: 2681,
      tokensOut: 370,
      tokensReasoning: 124,
      tokensCache: 20352,
    });
    if (fail) {
      emit({ kind: "done", turnId, outcome: "failed", error: "Error simulado por el mock" });
    } else {
      emit({ kind: "done", turnId, outcome: "succeeded" });
    }
  }

  return {
    async ask(text: string, screen: ScreenMode): Promise<AskResult> {
      mockTurnCounter += 1;
      const turnId = `turn-mock-${mockTurnCounter}`;
      void runTurn(turnId, text, screen);
      return { turnId };
    },
    async stop(): Promise<void> {
      stopped = true;
    },
    async replyPermission(requestId: string, _decision: PermissionDecision): Promise<void> {
      emit({ kind: "permissionClosed", requestId });
      const resume = pendingPermissions.get(requestId);
      if (resume) {
        pendingPermissions.delete(requestId);
        resume();
      }
    },
    async newConversation(): Promise<void> {
      emit({ kind: "conversationReset" });
    },
    async hide(): Promise<void> {
      emit({ kind: "hidden" });
    },
    async resize(_height: number): Promise<void> {
      // no-op en mock
    },
    async needsScreen(text: string): Promise<boolean> {
      return needsScreenLocal(text);
    },
    async getState(): Promise<BackendState> {
      const status: BackendStatus = "ready";
      return {
        model: mockModel,
        variant: mockVariant,
        sessionId: "ses_mock",
        backend: status,
        shortcut: "<Control><Alt>space",
        theme: "system",
        busy: false,
        models: mockModels.map((m) => ({ ...m })),
        fallbackModel: mockFallback,
      };
    },
    async setVariant(variant: Variant): Promise<void> {
      const info = mockModels.find((m) => m.id === mockModel);
      if (variant && info && !info.variants.includes(variant)) {
        throw new Error("Este modelo no admite ese nivel de esfuerzo");
      }
      mockVariant = variant;
      emit({ kind: "variant", variant });
    },
    async setModel(model: string): Promise<void> {
      if (!mockModels.some((m) => m.id === model)) {
        throw new Error(`Modelo desconocido: ${model}`);
      }
      const info = mockModels.find((m) => m.id === model);
      if (mockVariant && info && !info.variants.includes(mockVariant)) {
        mockVariant = "";
        emit({ kind: "variant", variant: "" });
      }
      mockModel = model;
      emit({ kind: "model", model, fallbackModel: mockFallback });
    },
    async openInTerminal(): Promise<void> {
      // no-op en mock
    },
    async openUrl(url: string): Promise<void> {
      window.open(url, "_blank", "noopener");
    },
    onEvent(cb: EventCallback): Unlisten {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    reset(): void {
      listeners.clear();
      pendingPermissions.clear();
      stopped = false;
    },
  };
}

// Singleton por defecto: real en Tauri, mock en navegador.
export const backend: Backend = isTauri() ? createTauriBackend() : createMockBackend();
