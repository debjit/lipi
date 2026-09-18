import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AGENT_DOCS,
  AGENT_LABELS,
  BEST_ALTERNATIVE_IDS,
  AgentConfig,
  AgentId,
  AgentScanItem,
  agentInstallCommand,
  agentInstallWhy,
} from "./agentTypes";

interface AgentsSettingsProps {
  config: AgentConfig;
  scan: AgentScanItem[];
  isScanning: boolean;
  models: Record<AgentId, string[]>;
  fetchingModels: AgentId | null;
  onSave: (patch: Partial<AgentConfig>) => Promise<void> | void;
  onRefreshScan: () => Promise<void> | void;
  onPickWorkspace: () => Promise<string | null>;
  onFetchModels: (id: AgentId) => Promise<void> | void;
}

export function AgentsSettings({
  config,
  scan,
  isScanning,
  models,
  fetchingModels,
  onSave,
  onRefreshScan,
  onPickWorkspace,
  onFetchModels,
}: AgentsSettingsProps) {
  const [copiedId, setCopiedId] = useState<AgentId | null>(null);
  const isWindows = navigator.userAgent.includes("Windows");
  const found = scan.filter((item) => item.found);
  const alternatives = BEST_ALTERNATIVE_IDS.filter((id) => !scan.find((item) => item.id === id)?.found)
    .map((id) => scan.find((item) => item.id === id) || {
      id,
      name: AGENT_LABELS[id],
      found: false,
      path: null,
      docs_url: AGENT_DOCS[id],
      enabled: config[id].enabled,
    });

  async function patchOverride(id: AgentId, patch: Partial<AgentConfig[AgentId]>) {
    const current = config[id];
    await onSave({ [id]: { ...current, ...patch } } as Partial<AgentConfig>);
  }

  async function copyInstall(id: AgentId) {
    const command = agentInstallCommand(id, isWindows);
    if (!command) return;
    try {
      await navigator.clipboard.writeText(command);
      setCopiedId(id);
      window.setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 2000);
    } catch {
      await openUrl(AGENT_DOCS[id]);
    }
  }

  function renderFoundCard(item: AgentScanItem) {
    const fetched = models[item.id] || [];
    return (
      <div key={item.id} className="agent-scan-card found">
        <div className="agent-scan-top">
          <strong>{item.name}</strong>
          <span className="badge-pill installed">Found</span>
        </div>
        <label className="checkbox-label">
          <input
            type="checkbox"
            className="checkbox-input"
            checked={config[item.id].enabled}
            onChange={(e) => patchOverride(item.id, { enabled: e.target.checked })}
          />
          <span>Enabled</span>
        </label>
        <input
          className="form-input font-mono"
          style={{ fontSize: 12, marginTop: 8 }}
          placeholder="Custom binary path (optional)"
          value={config[item.id].binary_path}
          onChange={(e) => patchOverride(item.id, { binary_path: e.target.value })}
          onBlur={() => onRefreshScan()}
        />
        <div className="form-hint font-mono agent-path">{item.path || "On PATH"}</div>
        <div className="agent-model-row">
          <input
            className="form-input font-mono"
            style={{ fontSize: 12 }}
            placeholder="Model (optional)"
            value={config[item.id].model}
            list={`agent-models-${item.id}`}
            onChange={(e) => patchOverride(item.id, { model: e.target.value })}
          />
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            disabled={fetchingModels === item.id}
            onClick={() => onFetchModels(item.id)}
          >
            {fetchingModels === item.id ? "Fetching…" : "Fetch models"}
          </button>
        </div>
        {fetched.length > 0 && (
          <>
            <datalist id={`agent-models-${item.id}`}>
              {fetched.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
            <select
              className="form-input form-select"
              style={{ fontSize: 12, marginTop: 6 }}
              value={fetched.includes(config[item.id].model) ? config[item.id].model : ""}
              onChange={(e) => {
                if (e.target.value) patchOverride(item.id, { model: e.target.value });
              }}
            >
              <option value="" disabled>
                -- Select from {fetched.length} fetched models --
              </option>
              {fetched.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </>
        )}
      </div>
    );
  }

  return (
    <div className="settings-section-card">
      <div className="settings-section-heading" style={{ justifyContent: "space-between" }}>
        <span>Coding Agents</span>
        <span className={`badge-pill ${config.enabled ? "installed" : "missing"}`}>
          {config.enabled ? "On" : "Off"}
        </span>
      </div>

      <p className="form-hint" style={{ marginBottom: 14 }}>
        Lipi transcribes locally, then runs an installed CLI with that text. OpenCode and Hermes are the recommended
        CLIs if you do not already have one.
      </p>

      <label className="checkbox-label" style={{ fontWeight: 600, fontSize: 14, marginBottom: 12 }}>
        <input
          type="checkbox"
          className="checkbox-input"
          checked={config.enabled}
          onChange={(e) =>
            onSave({ enabled: e.target.checked, destination: e.target.checked ? config.destination : "notes" })
          }
        />
        <span>Enable agent destination</span>
      </label>

      <div className="form-group">
        <label className="form-label">Default agent</label>
        <select
          className="form-input form-select"
          value={config.default_agent}
          onChange={(e) => onSave({ default_agent: e.target.value as AgentId })}
        >
          {(Object.keys(AGENT_LABELS) as AgentId[]).map((id) => {
            const item = scan.find((s) => s.id === id);
            return (
              <option key={id} value={id}>
                {AGENT_LABELS[id]}
                {item?.found ? "" : " (not found)"}
              </option>
            );
          })}
        </select>
      </div>

      <div className="form-group">
        <label className="form-label">Project folder (optional)</label>
        <div className="agent-workspace-row">
          <input
            className="form-input font-mono"
            value={config.workspace_dir}
            onChange={(e) => onSave({ workspace_dir: e.target.value })}
            placeholder="Leave empty for no project"
          />
          <button
            type="button"
            className="btn btn-secondary"
            onClick={async () => {
              const path = await onPickWorkspace();
              if (path) await onSave({ workspace_dir: path });
            }}
          >
            Browse
          </button>
          {config.workspace_dir.trim() && (
            <button type="button" className="btn btn-secondary" onClick={() => onSave({ workspace_dir: "" })}>
              Clear
            </button>
          )}
        </div>
        <span className="form-hint">
          Not required. If you pick a folder, Lipi runs the CLI there (OpenCode `--dir`). Otherwise the agent uses its own default.
        </span>
      </div>

      <label className="checkbox-label">
        <input
          type="checkbox"
          className="checkbox-input"
          checked={config.review_before_send}
          onChange={(e) => onSave({ review_before_send: e.target.checked })}
        />
        <span>Review transcript before sending</span>
      </label>
      <label className="checkbox-label">
        <input
          type="checkbox"
          className="checkbox-input"
          checked={config.continue_last_session}
          onChange={(e) => onSave({ continue_last_session: e.target.checked })}
        />
        <span>Continue last agent session when the CLI supports it</span>
      </label>
      <label className="checkbox-label">
        <input
          type="checkbox"
          className="checkbox-input"
          checked={config.apply_edits}
          onChange={(e) => onSave({ apply_edits: e.target.checked })}
        />
        <span>Cursor: apply file edits (`--force`)</span>
      </label>
      <label className="checkbox-label">
        <input
          type="checkbox"
          className="checkbox-input"
          checked={config.opencode_auto_approve}
          onChange={(e) => onSave({ opencode_auto_approve: e.target.checked })}
        />
        <span>OpenCode: auto-approve tools (`--auto`)</span>
      </label>

      {alternatives.length > 0 && (
        <>
          <div className="agent-section-head">
            <label className="form-label" style={{ margin: 0 }}>
              {alternatives.length === 1 ? "Best alternative" : "Best alternatives"}
            </label>
          </div>
          <p className="form-hint" style={{ marginBottom: 8 }}>
            Copy the official install command, run it in PowerShell, then Re-detect. Lipi does not run the installer.
          </p>
          <div className="agent-scan-grid">
            {alternatives.map((item) => {
              const command = agentInstallCommand(item.id, isWindows) || "";
              return (
                <div key={item.id} className="agent-scan-card agent-alt-card">
                  <div className="agent-scan-top">
                    <strong>{item.name}</strong>
                    <span className="badge-pill installed">Recommended</span>
                  </div>
                  <p className="form-hint">{agentInstallWhy(item.id)}</p>
                  <code className="agent-install-cmd">{command}</code>
                  <div className="agent-alt-actions">
                    <button type="button" className="btn btn-primary btn-sm" onClick={() => copyInstall(item.id)}>
                      {copiedId === item.id ? "Copied" : "Copy install command"}
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => openUrl(item.docs_url || AGENT_DOCS[item.id])}
                    >
                      Docs
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </>
      )}

      <div className="agent-section-head">
        <label className="form-label" style={{ margin: 0 }}>
          Detected CLIs
        </label>
        <button type="button" className="btn btn-secondary btn-sm" onClick={() => onRefreshScan()} disabled={isScanning}>
          {isScanning ? "Scanning…" : "Re-detect"}
        </button>
      </div>
      {found.length > 0 ? (
        <>
          <p className="form-hint" style={{ marginBottom: 8 }}>
            Fetch models asks the CLI for its catalog. Leave the model blank to use the CLI default.
          </p>
          <div className="agent-scan-grid">{found.map(renderFoundCard)}</div>
        </>
      ) : (
        <p className="form-hint">
          {isScanning
            ? "Looking for CLIs on PATH…"
            : "None found yet. Install a recommended CLI above, then Re-detect."}
        </p>
      )}
    </div>
  );
}
