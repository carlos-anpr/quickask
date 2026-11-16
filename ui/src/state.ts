// state.ts — reducer PURO reduce(state, UiEvent) -> state.
import type { ModelInfo, UiEvent, Variant } from "./contract";
import { DEFAULT_FALLBACK_MODEL, DEFAULT_MODEL, KNOWN_MODELS } from "./contract";

export type ToolStatus = "pending" | "running" | "ok" | "error";
export type ScreenStatus = "capturing" | "attached" | "failed";
export type TurnOutcome = "succeeded" | "failed" | "interrupted";

export interface TextSegment {
  messageId: string;
  ordinal: number;
  text: string;
  final: boolean;
}

export interface ToolState {
  callId: string;
  name: string;
  status: ToolStatus;
  title?: string;
}

export interface PermissionState {
  requestId: string;
  action: string;
  resources: string[];
}

export interface UsageState {
  cost: number;
  tokensIn: number;
  tokensOut: number;
  tokensReasoning: number;
  tokensCache: number;
}

export interface Turn {
  turnId: string;
  text: string;
  screen: boolean;
  screenStatus: ScreenStatus | null;
  thumb?: string;
  screenError?: string;
  reasoning: string;
  segments: TextSegment[];
  tools: ToolState[];
  permissions: PermissionState[];
  notices: string[];
  usage?: UsageState;
  outcome?: TurnOutcome;
  error?: string;
}

export interface AppState {
  turns: Turn[];
  greeting: string;
  backend: "connecting" | "ready" | "down";
  backendDetail?: string;
  visible: boolean;
  busy: boolean;
  variant: Variant;
  contextTokens?: number;
  model: string;
  fallbackModel: string;
  models: ModelInfo[];
}

export function initialState(): AppState {
  return {
    turns: [],
    greeting: "",
    backend: "connecting",
    backendDetail: undefined,
    visible: false,
    busy: false,
    variant: "",
    model: DEFAULT_MODEL,
    fallbackModel: DEFAULT_FALLBACK_MODEL,
    models: KNOWN_MODELS.map((m) => ({ ...m })),
  };
}

function findTurnIndex(turns: Turn[], turnId: string): number {
  return turns.findIndex((t) => t.turnId === turnId);
}

function cloneTurn(t: Turn): Turn {
  return {
    ...t,
    segments: t.segments.map((s) => ({ ...s })),
    tools: t.tools.map((x) => ({ ...x })),
    permissions: t.permissions.map((p) => ({ ...p, resources: [...p.resources] })),
    notices: [...t.notices],
    usage: t.usage ? { ...t.usage } : undefined,
  };
}

export function reduce(state: AppState, ev: UiEvent): AppState {
  switch (ev.kind) {
    case "summoned":
      return { ...state, greeting: ev.greeting, visible: true };
    case "hidden":
      return { ...state, visible: false };
    case "backend":
      return { ...state, backend: ev.status, backendDetail: ev.detail };
    case "turnStarted": {
      const turn: Turn = {
        turnId: ev.turnId,
        text: ev.text,
        screen: ev.screen,
        screenStatus: null,
        reasoning: "",
        segments: [],
        tools: [],
        permissions: [],
        notices: [],
      };
      return { ...state, turns: [...state.turns, turn], busy: true };
    }
    case "screen": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      turns[i].screenStatus = ev.status;
      if (ev.thumb !== undefined) turns[i].thumb = ev.thumb;
      if (ev.error !== undefined) turns[i].screenError = ev.error;
      return { ...state, turns };
    }
    case "reasoning": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      turns[i].reasoning += ev.delta;
      return { ...state, turns };
    }
    case "text": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      const seg = turns[i].segments.find(
        (s) => s.messageId === ev.messageId && s.ordinal === ev.ordinal,
      );
      if (seg) {
        if (!seg.final) seg.text += ev.delta;
      } else {
        turns[i].segments.push({
          messageId: ev.messageId,
          ordinal: ev.ordinal,
          text: ev.delta,
          final: false,
        });
      }
      // Orden estable por (messageId, ordinal)
      turns[i].segments.sort(
        (a, b) => a.messageId.localeCompare(b.messageId) || a.ordinal - b.ordinal,
      );
      return { ...state, turns };
    }
    case "textFinal": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      const seg = turns[i].segments.find(
        (s) => s.messageId === ev.messageId && s.ordinal === ev.ordinal,
      );
      if (seg) {
        seg.text = ev.text; // REEMPLAZA el acumulado (reconciliación)
        seg.final = true;
      } else {
        turns[i].segments.push({
          messageId: ev.messageId,
          ordinal: ev.ordinal,
          text: ev.text,
          final: true,
        });
      }
      turns[i].segments.sort(
        (a, b) => a.messageId.localeCompare(b.messageId) || a.ordinal - b.ordinal,
      );
      return { ...state, turns };
    }
    case "tool": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      const existing = turns[i].tools.find((t) => t.callId === ev.callId);
      if (existing) {
        existing.name = ev.name;
        existing.status = ev.status;
        if (ev.title !== undefined) existing.title = ev.title;
      } else {
        turns[i].tools.push({
          callId: ev.callId,
          name: ev.name,
          status: ev.status,
          title: ev.title,
        });
      }
      return { ...state, turns };
    }
    case "permission": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      if (!turns[i].permissions.some((p) => p.requestId === ev.requestId)) {
        turns[i].permissions.push({
          requestId: ev.requestId,
          action: ev.action,
          resources: [...ev.resources],
        });
      }
      return { ...state, turns };
    }
    case "permissionClosed": {
      const turns = state.turns.map(cloneTurn);
      let changed = false;
      for (const t of turns) {
        const before = t.permissions.length;
        t.permissions = t.permissions.filter((p) => p.requestId !== ev.requestId);
        if (t.permissions.length !== before) changed = true;
      }
      if (!changed) return state;
      return { ...state, turns };
    }
    case "usage": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      // El evento ya trae el ACUMULADO del turno (R8): se reemplaza.
      turns[i].usage = {
        cost: ev.cost,
        tokensIn: ev.tokensIn,
        tokensOut: ev.tokensOut,
        tokensReasoning: ev.tokensReasoning,
        tokensCache: ev.tokensCache,
      };
      return { ...state, turns, contextTokens: ev.tokensContext ?? state.contextTokens };
    }
    case "variant":
      return { ...state, variant: ev.variant };
    case "model":
      return { ...state, model: ev.model, fallbackModel: ev.fallbackModel };
    case "notice": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      turns[i].notices.push(ev.text);
      return { ...state, turns };
    }
    case "done": {
      const i = findTurnIndex(state.turns, ev.turnId);
      if (i < 0) return state;
      const turns = state.turns.map(cloneTurn);
      turns[i].outcome = ev.outcome;
      if (ev.error !== undefined) turns[i].error = ev.error;
      // busy global: queda otro turno sin outcome?
      const busy = turns.some((t) => t.outcome === undefined);
      return { ...state, turns, busy };
    }
    case "conversationReset":
      return { ...state, contextTokens: undefined, turns: [], busy: false };
    default: {
      const _exhaustive: never = ev;
      void _exhaustive;
      return state;
    }
  }
}
