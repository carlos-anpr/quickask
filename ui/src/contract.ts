// Tipos EXACTOS del PLAN §4 + REVISIÓN 8 (APROBADO por el coordinador).
// BACK los replica con serde. No modificar sin pasar por docs/CONTRATO-CAMBIOS.md.

// ---- backend -> UI: evento Tauri "qa://event" ----
export type UiEvent =
  | { kind: "summoned"; greeting: string } // la ventana se acaba de mostrar por el atajo -> UI enfoca el input y anima entrada
  | { kind: "hidden" } // la ventana se ocultó (Esc, blur, toggle)
  | { kind: "backend"; status: "connecting" | "ready" | "down"; detail?: string }
  | { kind: "turnStarted"; turnId: string; text: string; screen: boolean }
  | { kind: "screen"; turnId: string; status: "capturing" | "attached" | "failed"; thumb?: string /* data:image/jpeg;base64, <=320px de ancho */; error?: string }
  | { kind: "reasoning"; turnId: string; messageId: string; delta: string }
  | { kind: "text"; turnId: string; messageId: string; ordinal: number; delta: string }
  | { kind: "textFinal"; turnId: string; messageId: string; ordinal: number; text: string } // texto completo del segmento (reconciliar)
  | { kind: "tool"; turnId: string; callId: string; name: string; status: "pending" | "running" | "ok" | "error"; title?: string } // title: resumen <=80 chars (comando, agente, url…)
  | { kind: "permission"; turnId: string; requestId: string; action: string; resources: string[] }
  | { kind: "permissionClosed"; requestId: string }
  | { kind: "usage"; turnId: string; cost: number; tokensIn: number; tokensOut: number; tokensReasoning: number; tokensCache: number; tokensContext?: number } // ACUMULADO por turno (R8): suma de todos los pasos
  | { kind: "variant"; variant: Variant } // esfuerzo del modelo (R8)
  | { kind: "model"; model: string; fallbackModel: string } // modelo activo + relevo (R11)
  | { kind: "notice"; turnId: string; text: string } // aviso breve dentro del turno (R11)
  | { kind: "done"; turnId: string; outcome: "succeeded" | "failed" | "interrupted"; error?: string }
  | { kind: "conversationReset" };

// ---- UI -> backend: invoke(nombre, args) ----
export type ScreenMode = "auto" | "on" | "off";
export type PermissionDecision = "once" | "always" | "reject";
export type BackendStatus = "connecting" | "ready" | "down";
// Esfuerzo del modelo (R8): "" = default. setVariant({variant}) -> void.
// Cada modelo declara sus variantes (none, minimal, low, medium, high, xhigh, max, thinking…).
export type Variant = string;

export interface AskResult {
  turnId: string;
}

export interface BackendState {
  model: string;
  variant: string;
  sessionId: string | null;
  backend: BackendStatus;
  shortcut: string;
  theme: "system" | "dark" | "light";
  busy: boolean;
  models: ModelInfo[];
  fallbackModel: string;
}

export interface ModelInfo {
  id: string;
  label: string;
  free: boolean;
  vision: boolean;
  variants: string[];
}

// Por defecto: el modelo gratuito de OpenCode Zen y sin relevo. El backend manda
// (config.toml o QUICKASK_MODEL); el catálogo real viene de getState().models.
export const DEFAULT_MODEL = "opencode/deepseek-v4.1-flash-free";
export const DEFAULT_FALLBACK_MODEL = "";
export const KNOWN_MODELS: ModelInfo[] = [
  { id: DEFAULT_MODEL, label: "DeepSeek V4.1 Flash Free", free: true, vision: true, variants: ["low", "high", "max"] },
];

export const QA_EVENT = "qa://event";
