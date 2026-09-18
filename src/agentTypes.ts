export type AgentId = "hermes" | "opencode" | "agy" | "cursor";

export interface AgentOverride {
  enabled: boolean;
  binary_path: string;
  model: string;
}

export interface AgentConfig {
  enabled: boolean;
  destination: "notes" | "agent";
  default_agent: AgentId;
  workspace_dir: string;
  review_before_send: boolean;
  continue_last_session: boolean;
  apply_edits: boolean;
  opencode_auto_approve: boolean;
  hermes: AgentOverride;
  opencode: AgentOverride;
  agy: AgentOverride;
  cursor: AgentOverride;
}

export interface AgentScanItem {
  id: AgentId;
  name: string;
  found: boolean;
  path: string | null;
  docs_url: string;
  enabled: boolean;
}

export interface AgentRunResult {
  agent_id: string;
  status: "done" | "failed" | "cancelled" | string;
  exit_code: number | null;
  output: string;
}

export interface AgentUiState {
  config: AgentConfig;
  scan: AgentScanItem[];
  scanning: boolean;
  output: string;
  status: "idle" | "running" | "done" | "failed" | "cancelled";
  selected: AgentId;
  models: Record<AgentId, string[]>;
  fetchingModels: AgentId | null;
}

export const AGENT_LABELS: Record<AgentId, string> = {
  hermes: "Hermes",
  opencode: "OpenCode",
  agy: "Agy (Antigravity)",
  cursor: "Cursor",
};

export const AGENT_DOCS: Record<AgentId, string> = {
  hermes: "https://hermes-agent.nousresearch.com/docs/getting-started/installation",
  opencode: "https://opencode.ai/docs",
  agy: "https://antigravity.google/docs/cli/install/",
  cursor: "https://cursor.com/docs/cli/installation",
};

export const BEST_ALTERNATIVE_IDS: AgentId[] = ["opencode", "hermes"];

export function agentInstallCommand(id: AgentId, windows: boolean): string | null {
  if (id === "hermes") {
    return windows
      ? "iex (irm https://hermes-agent.nousresearch.com/install.ps1)"
      : "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash";
  }
  if (id === "opencode") {
    return windows
      ? "npm install -g opencode-ai"
      : "curl -fsSL https://opencode.ai/install | bash";
  }
  return null;
}

export function agentInstallWhy(id: AgentId): string {
  if (id === "opencode") {
    return "Headless `opencode run` is a good match for Lipi: send the transcript, keep output in this window.";
  }
  if (id === "hermes") {
    return "Native Windows installer. `hermes chat -q` takes the spoken prompt without a second API key in Lipi.";
  }
  return "";
}

export function defaultAgentOverride(): AgentOverride {
  return { enabled: true, binary_path: "", model: "" };
}

export function defaultAgentConfig(): AgentConfig {
  return {
    enabled: false,
    destination: "notes",
    default_agent: "hermes",
    workspace_dir: "",
    review_before_send: true,
    continue_last_session: false,
    apply_edits: true,
    opencode_auto_approve: false,
    hermes: defaultAgentOverride(),
    opencode: defaultAgentOverride(),
    agy: defaultAgentOverride(),
    cursor: defaultAgentOverride(),
  };
}

export function emptyAgentModels(): Record<AgentId, string[]> {
  return { hermes: [], opencode: [], agy: [], cursor: [] };
}

export function mergeAgentConfig(cfg?: Partial<AgentConfig> | null): AgentConfig {
  const defaults = defaultAgentConfig();
  const incoming = cfg || {};
  return {
    ...defaults,
    ...incoming,
    hermes: { ...defaults.hermes, ...incoming.hermes },
    opencode: { ...defaults.opencode, ...incoming.opencode },
    agy: { ...defaults.agy, ...incoming.agy },
    cursor: { ...defaults.cursor, ...incoming.cursor },
  };
}

export function defaultAgentUi(): AgentUiState {
  return {
    config: defaultAgentConfig(),
    scan: [],
    scanning: false,
    output: "",
    status: "idle",
    selected: "hermes",
    models: emptyAgentModels(),
    fetchingModels: null,
  };
}
