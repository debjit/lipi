import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { enable as enableAutostart, disable as disableAutostart, isEnabled as isAutostartEnabled } from "@tauri-apps/plugin-autostart";
import "./App.css";
import { SetupWizard } from "./SetupWizard";

interface Note {
  id: number;
  title: string;
  content: string;
  created_at: string;
  updated_at: string;
}

interface AppSettings {
  api_base_url: string;
  api_key: string;
  model: string;
  language?: string;
  auto_copy: boolean;
  always_on_top: boolean;
  engine_mode: "cloud" | "local";
  local_engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan";
  local_model_size: "tiny" | "base" | "small";
  models_folder?: string;
  model_idle_timeout_mins?: number;
  request_timeout_secs?: number;
  mini_record_mode?: "new_note" | "append";
  provider_id?: string;
  wizard_completed?: boolean;
}

interface LlmProvider {
  id: string;
  name: string;
  provider_type: "cloudflare" | "groq" | "openai" | "ollama" | "custom";
  api_key: string;
  account_id: string;
  base_url: string;
  model?: string;
  recent_models?: string[];
}

interface PromptPreset {
  id: string;
  label: string;
  prompt: string;
}

interface LlmSettings {
  enabled: boolean;
  active_provider_id: string;
  model: string;
  voice_preset: string;
  custom_prompt: string;
  auto_mode: boolean;
  providers: LlmProvider[];
  presets: PromptPreset[];
  request_timeout_secs?: number;
}

interface ModelStatus {
  engine: string;
  model_size: string;
  installed: boolean;
  file_path: string;
  file_size_bytes: number;
  binary_available: boolean;
  binary_path: string;
  models_dir?: string;
}

interface ModelMemoryStatus {
  is_loaded: boolean;
  engine?: string;
  model_size?: string;
  loaded_at_timestamp?: number;
  last_active_timestamp?: number;
  idle_seconds: number;
  idle_timeout_mins: number;
  estimated_ram_mb: number;
  port?: number;
}

interface CloudflareUsagePeriod {
  label: string;
  total_neurons: number;
  total_cost_usd: number;
  asr_audio_secs: number;
  asr_count: number;
  llm_tokens: number;
  llm_count: number;
}

interface OperationResult {
  text: string;
  cf_neurons?: number | null;
  cf_cost?: number | null;
}

interface CloudflareUsageSummary {
  today: CloudflareUsagePeriod;
  current_month: CloudflareUsagePeriod;
  all_time: CloudflareUsagePeriod;
  monthly_history: CloudflareUsagePeriod[];
}

function formatNeurons(n: number): string {
  if (!n || n <= 0) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  if (n % 1 !== 0) return n.toFixed(1);
  return Math.round(n).toString();
}

function formatCostUsd(c: number): string {
  if (!c || c <= 0) return "$0.00";
  if (c < 0.0001) return "<$0.0001";
  if (c < 0.01) return `$${c.toFixed(4)}`;
  return `$${c.toFixed(2)}`;
}

function formatDurationSecs(secs: number): string {
  const s = Math.round(secs);
  const mins = Math.floor(s / 60);
  const remSecs = s % 60;
  if (mins === 0) return `${remSecs}s`;
  return `${mins}m ${remSecs}s`;
}

function maskAccountId(id: string): string {
  const trimmed = (id || "").trim();
  if (!trimmed || trimmed === "<account_id>" || trimmed === "{account_id}") return "";
  if (trimmed.length <= 6) return trimmed;
  return "*".repeat(trimmed.length - 6) + trimmed.slice(-6);
}

function getDisplayModelName(settings: AppSettings): string {
  if (settings.engine_mode === "local") {
    // Strictly zero-cost / local offline
    return `Whisper (${settings.local_model_size})`;
  }
  const raw = (settings.model || "").trim();
  let baseName = raw;
  if (!baseName) baseName = "whisper-1";
  else if (baseName.startsWith("@cf/")) {
    const parts = baseName.split("/");
    baseName = parts[parts.length - 1] || baseName;
    if (baseName === "whisper-large-v3-turbo") {
      baseName = "turbo";
    }
  } else if (baseName.includes("/") && !baseName.startsWith("http")) {
    const parts = baseName.split("/");
    baseName = parts[parts.length - 1] || baseName;
  }

  return baseName;
}

function deriveTitle(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return "Untitled Note";
  const firstLine = trimmed.split("\n")[0];
  return firstLine.slice(0, 45) + (firstLine.length > 45 ? "..." : "");
}

function formatDate(dateStr: string): string {
  try {
    const d = new Date(dateStr.endsWith("Z") ? dateStr : dateStr + "Z");
    return (
      d.toLocaleDateString([], { month: "short", day: "numeric" }) +
      " " +
      d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    );
  } catch {
    return dateStr;
  }
}

function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m === 0) return `${s}s`;
  return `${m}m ${s}s`;
}

interface LogEntry {
  id: string;
  timestamp: string;
  level: "error" | "success" | "info";
  title: string;
  engine: string;
  model: string;
  endpoint?: string;
  message: string;
  details?: string;
}

function isFullScreenMode(): boolean {
  if (typeof window === "undefined" || !window.screen) return false;
  if (window.screen.availWidth < 1200) return false;
  return (
    window.innerWidth >= window.screen.availWidth - 40 &&
    window.innerHeight >= window.screen.availHeight - 80
  );
}

export default function App() {
  const [notes, setNotes] = useState<Note[]>([]);
  const [activeNoteId, setActiveNoteId] = useState<number | null>(null);
  const [content, setContent] = useState("");
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [recordSeconds, setRecordSeconds] = useState(0);
  const [copiedNotification, setCopiedNotification] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(() => isFullScreenMode());
  const [activeView, setActiveView] = useState<"notes" | "settings">("notes");
  const [miniMode, setMiniMode] = useState(false);
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [memoryStatus, setMemoryStatus] = useState<ModelMemoryStatus | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<number | null>(null);
  const [downloadSpeedBps, setDownloadSpeedBps] = useState<number>(0);
  const [downloadEtaSecs, setDownloadEtaSecs] = useState<number>(0);
  const [isDownloading, setIsDownloading] = useState(false);
  const [isInstallingDeps, setIsInstallingDeps] = useState(false);
  const [isPreloading, setIsPreloading] = useState(false);
  const [isFreeingRam, setIsFreeingRam] = useState(false);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [autostartEnabled, setAutostartEnabled] = useState(false);

  const [settings, setSettings] = useState<AppSettings>({
    api_base_url: "https://api.openai.com/v1",
    api_key: "",
    model: "whisper-1",
    language: "",
    auto_copy: true,
    always_on_top: true,
    engine_mode: "cloud",
    local_engine: "whisper_cpu",
    local_model_size: "base",
    models_folder: "",
    model_idle_timeout_mins: 10,
    request_timeout_secs: 180,
    mini_record_mode: "new_note",
  });

  const DEFAULT_PRESETS: PromptPreset[] = [
    {
      id: "grammar_fix",
      label: "✍️ Fix Grammar & Typos",
      prompt:
        "You are an expert copyeditor. Fix all grammatical mistakes, spelling errors, punctuation mistakes, and typos in the transcribed speech text. Strictly preserve the original tone, vocabulary, and meaning. Do not add any conversational remarks, explanations, or quotes. Output ONLY the corrected text.",
    },
    {
      id: "professional",
      label: "💼 Professional Tone",
      prompt:
        "You are an executive communications editor. Rewrite the transcribed speech text into clear, polished, professional business language while preserving all original facts and substance. Do not add commentary, pleasantries, or introductory remarks. Output ONLY the rewritten text.",
    },
    {
      id: "casual",
      label: "☕ Casual & Friendly",
      prompt:
        "Rewrite the transcribed speech text into a friendly, relaxed, conversational tone. Smooth out awkward spoken hesitations while keeping the speaker's personality. Do not add commentary or pleasantries. Output ONLY the rewritten text.",
    },
    {
      id: "concise",
      label: "⚡ Concise Summary",
      prompt:
        "Condense the transcribed speech text into a punchy, high-impact summary. Eliminate filler words, redundancies, and rambling. Do not add commentary. Output ONLY the concise text.",
    },
    {
      id: "bullets",
      label: "📌 Bullet Points",
      prompt:
        "Extract the core points, key details, and action items from the transcribed speech text into a structured Markdown bullet list. Do not add commentary. Output ONLY the bullet points.",
    },
  ];

  const [llmSettings, setLlmSettings] = useState<LlmSettings>({
    enabled: false,
    active_provider_id: "CLOUDFLARE_DEFAULT",
    model: "@cf/meta/llama-3.1-8b-instruct",
    voice_preset: "grammar_fix",
    custom_prompt: "",
    auto_mode: false,
    providers: [
      {
        id: "CLOUDFLARE_DEFAULT",
        name: "Cloudflare Workers AI",
        provider_type: "cloudflare",
        api_key: "",
        account_id: "",
        base_url: "https://api.cloudflare.com/client/v4/accounts/<account_id>/ai/v1",
      },
      {
        id: "GROQ_DEFAULT",
        name: "Groq Cloud",
        provider_type: "groq",
        api_key: "",
        account_id: "",
        base_url: "https://api.groq.com/openai/v1",
      },
      {
        id: "OLLAMA_DEFAULT",
        name: "Ollama (Local)",
        provider_type: "ollama",
        api_key: "",
        account_id: "",
        base_url: "http://localhost:11434/v1",
      },
    ],
    presets: DEFAULT_PRESETS,
    request_timeout_secs: 180,
  });
  const llmSettingsRef = useRef(llmSettings);
  llmSettingsRef.current = llmSettings;

  const [editingPresetId, setEditingPresetId] = useState<string | null>(null);
  const [isAddingPreset, setIsAddingPreset] = useState<boolean>(false);
  const [presetForm, setPresetForm] = useState<{ id: string; label: string; prompt: string }>({
    id: "",
    label: "",
    prompt: "",
  });

  const [rawTranscript, setRawTranscript] = useState("");
  const rawTranscriptRef = useRef(rawTranscript);
  rawTranscriptRef.current = rawTranscript;

  const [llmResult, setLlmResult] = useState("");
  const [isTransforming, setIsTransforming] = useState(false);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [isFetchingModels, setIsFetchingModels] = useState(false);
  const [editingProviderId, setEditingProviderId] = useState<string>("CLOUDFLARE_DEFAULT");
  const [navOverflowOpen, setNavOverflowOpen] = useState(false);
  const navOverflowRef = useRef<HTMLDivElement>(null);
  const [isFullScreen, setIsFullScreen] = useState(false);
  const [windowWidth, setWindowWidth] = useState(
    typeof window !== "undefined" ? window.innerWidth : 1200
  );

  const [logs, setLogs] = useState<LogEntry[]>(() => {
    try {
      const saved = localStorage.getItem("lipi_diagnostic_logs");
      return saved ? JSON.parse(saved) : [];
    } catch {
      return [];
    }
  });
  const [expandedLogId, setExpandedLogId] = useState<string | null>(null);

  const addLog = (entry: Omit<LogEntry, "id" | "timestamp">) => {
    const newEntry: LogEntry = {
      ...entry,
      id: Math.random().toString(36).substring(2, 9),
      timestamp: new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }),
    };
    setLogs((prev) => {
      const updated = [newEntry, ...prev.slice(0, 49)];
      try {
        localStorage.setItem("lipi_diagnostic_logs", JSON.stringify(updated));
      } catch {}
      return updated;
    });
    return newEntry;
  };

  const clearLogs = () => {
    setLogs([]);
    try {
      localStorage.removeItem("lipi_diagnostic_logs");
    } catch {}
  };

  const copyFullLogs = async () => {
    if (logs.length === 0) return;
    const formatted = logs
      .map(
        (l) =>
          `[${l.timestamp}] [${l.level.toUpperCase()}] ${l.title}\nEngine: ${l.engine} | Model: ${l.model}${
            l.endpoint ? ` | Endpoint: ${l.endpoint}` : ""
          }\nMessage: ${l.message}${l.details ? `\nDetails:\n${l.details}` : ""}\n---`
      )
      .join("\n\n");
    await copyText(formatted);
    showToast("✓ Diagnostic logs copied!");
  };

  const [settingsNavTab, setSettingsNavTab] = useState<"providers" | "asr" | "llm" | "preferences" | "logs">("providers");

  const [cfUsage, setCfUsage] = useState<CloudflareUsageSummary | null>(null);
  const [loadingCfUsage, setLoadingCfUsage] = useState<boolean>(false);
  const [revealAccountId, setRevealAccountId] = useState<boolean>(false);
  const [focusedAccountId, setFocusedAccountId] = useState<string | null>(null);

  const currentAsrProvider = (llmSettings.providers || []).find((p) => p.id === (settings.provider_id || (llmSettings.providers.find((pr) => pr.base_url === settings.api_base_url)?.id)));
  const isCloudflareAsr = settings.engine_mode === "cloud" && (settings.api_base_url.includes("api.cloudflare.com") || currentAsrProvider?.provider_type === "cloudflare");
  const activeLlmProvider = (llmSettings.providers || []).find((p) => p.id === llmSettings.active_provider_id);
  const isCloudflareLlm = llmSettings.enabled && (
    activeLlmProvider?.provider_type === "cloudflare" ||
    (activeLlmProvider?.base_url || "").includes("cloudflare.com")
  );
  const isCloudflareActive = isCloudflareAsr || isCloudflareLlm;

  async function loadCfUsage(): Promise<CloudflareUsageSummary | null> {
    try {
      setLoadingCfUsage(true);
      const summary: CloudflareUsageSummary = await invoke("get_cf_usage_summary");
      setCfUsage(summary);
      return summary;
    } catch (err) {
      console.error("Failed to load Cloudflare usage summary:", err);
      return null;
    } finally {
      setLoadingCfUsage(false);
    }
  }

  async function handleClearCfUsage() {
    if (!window.confirm("Are you sure you want to clear your local Cloudflare usage records? This cannot be undone.")) {
      return;
    }
    try {
      await invoke("clear_cf_usage_logs");
      await loadCfUsage();
      showToast("✓ Cloudflare usage history cleared");
    } catch (err) {
      showToast(`Failed to clear usage: ${err}`);
    }
  }

  const activeViewRef = useRef(activeView);
  activeViewRef.current = activeView;

  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  const timerRef = useRef<number | null>(null);
  const activeContentRef = useRef(content);
  activeContentRef.current = content;

  const activeIdRef = useRef(activeNoteId);
  activeIdRef.current = activeNoteId;

  const isRecordingRef = useRef(isRecording);
  isRecordingRef.current = isRecording;

  const isTranscribingRef = useRef(isTranscribing);
  isTranscribingRef.current = isTranscribing;

  const miniModeRef = useRef(miniMode);
  miniModeRef.current = miniMode;

  useEffect(() => {
    loadNotes();
    loadSettings();
    loadLlmSettings();
    loadCfUsage();
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{ percentage: number; speed_bps?: number; eta_secs?: number }>("model-download-progress", (event) => {
      setDownloadProgress(event.payload.percentage);
      if (event.payload.speed_bps !== undefined) {
        setDownloadSpeedBps(event.payload.speed_bps);
      }
      if (event.payload.eta_secs !== undefined) {
        setDownloadEtaSecs(event.payload.eta_secs);
      }
      if (event.payload.percentage >= 100) {
        setTimeout(() => {
          setDownloadProgress(null);
          setDownloadSpeedBps(0);
          setDownloadEtaSecs(0);
          setIsDownloading(false);
          refreshModelStatus();
        }, 600);
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  async function refreshModelStatus(
    engine = settingsRef.current.local_engine,
    size = settingsRef.current.local_model_size,
    customDir = settingsRef.current.models_folder
  ) {
    try {
      const s: ModelStatus = await invoke("get_model_status", {
        engine,
        modelSize: size,
        customModelsDir: customDir || undefined,
      });
      setModelStatus(s);
    } catch (e) {
      console.error("Failed to check model status:", e);
    }
  }

  async function handleDownloadModel() {
    setIsDownloading(true);
    setDownloadProgress(0);
    setErrorMsg(null);
    try {
      await invoke("download_model", {
        engine: settingsRef.current.local_engine,
        modelSize: settingsRef.current.local_model_size,
        customModelsDir: settingsRef.current.models_folder || undefined,
      });
      await updateAndSaveSettings({
        engine_mode: "local",
        local_engine: settingsRef.current.local_engine,
        local_model_size: settingsRef.current.local_model_size,
        models_folder: settingsRef.current.models_folder,
      });
      await refreshModelStatus();
      showToast("✓ Model weights ready!");
    } catch (err: any) {
      setErrorMsg(String(err));
    } finally {
      setIsDownloading(false);
    }
  }

  async function handleDeleteModel() {
    try {
      await invoke("delete_model", {
        engine: settings.local_engine,
        modelSize: settings.local_model_size,
        customModelsDir: settings.models_folder || undefined,
      });
      await refreshModelStatus();
      showToast("Model deleted");
    } catch (err: any) {
      setErrorMsg(String(err));
    }
  }

  async function handlePrepareRunner() {
    setIsInstallingDeps(true);
    setErrorMsg(null);
    try {
      const isFW = settingsRef.current.local_engine === "faster_whisper";
      showToast(
        isFW
          ? "Setting up isolated environment & installing faster-whisper (this may take ~1m)..."
          : "Preparing official whisper.cpp runner binary..."
      );
      const res: string = await invoke("prepare_engine", {
        engine: settingsRef.current.local_engine,
      });
      await refreshModelStatus();
      showToast(res || "✓ Engine runner ready!");
    } catch (err: any) {
      setErrorMsg(String(err));
    } finally {
      setIsInstallingDeps(false);
    }
  }

  async function handlePickDirectory() {
    try {
      const selected: string | null = await invoke("pick_directory");
      if (selected) {
        await updateAndSaveSettings({ models_folder: selected });
        await refreshModelStatus(undefined, undefined, selected);
        showToast("✓ Storage location updated");
      }
    } catch (err: any) {
      setErrorMsg(String(err));
    }
  }

  // Recording timer
  useEffect(() => {
    if (isRecording) {
      setRecordSeconds(0);
      timerRef.current = window.setInterval(() => {
        setRecordSeconds((s) => s + 1);
      }, 1000);
    } else {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
      setRecordSeconds(0);
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [isRecording]);

  // Keyboard shortcuts (Alt+R, Alt+M, Escape for settings, Space in mini mode)
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Escape: return to notes from settings
      if (e.key === "Escape" && activeViewRef.current === "settings") {
        e.preventDefault();
        closeSettings();
        return;
      }

      // Alt+R: toggle recording
      if (e.altKey && (e.key === "r" || e.key === "R")) {
        e.preventDefault();
        toggleRecording();
        return;
      }

      // Alt+M: toggle mini mode
      if (e.altKey && (e.key === "m" || e.key === "M")) {
        e.preventDefault();
        toggleMiniMode(!miniModeRef.current);
        return;
      }

      // In mini mode: Space bar toggles recording when not in an input
      if (miniModeRef.current && e.code === "Space") {
        const target = e.target as HTMLElement;
        if (target.tagName !== "INPUT" && target.tagName !== "TEXTAREA") {
          e.preventDefault();
          toggleRecording();
        }
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // Poll memory status while on Settings page
  useEffect(() => {
    if (activeView !== "settings") return;
    refreshMemoryStatus();
    const interval = setInterval(refreshMemoryStatus, 2500);
    return () => clearInterval(interval);
  }, [activeView]);

  // Click outside to close nav overflow dropdown
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (
        navOverflowRef.current &&
        !navOverflowRef.current.contains(e.target as Node)
      ) {
        setNavOverflowOpen(false);
      }
    }
    if (navOverflowOpen) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [navOverflowOpen]);

  // Track fullscreen / maximized state and window dimensions for stacked navbar
  useEffect(() => {
    let unlistenResize: (() => void) | undefined;

    const updateWindowMetrics = async () => {
      setWindowWidth(window.innerWidth);
      try {
        const win = getCurrentWindow();
        const [max, fs] = await Promise.all([win.isMaximized(), win.isFullscreen()]);
        setIsFullScreen(Boolean(max || fs));
      } catch {
        setIsFullScreen(window.innerWidth >= 1200);
      }
    };

    updateWindowMetrics();
    window.addEventListener("resize", updateWindowMetrics);

    try {
      const win = getCurrentWindow();
      win.onResized(() => {
        updateWindowMetrics();
      }).then((unlisten) => {
        unlistenResize = unlisten;
      }).catch(() => {});
    } catch {}

    return () => {
      window.removeEventListener("resize", updateWindowMetrics);
      if (unlistenResize) unlistenResize();
    };
  }, []);

  const isStackedNav = !isFullScreen || windowWidth < 1150;

  async function loadNotes() {
    try {
      const list: Note[] = await invoke("get_notes");
      setNotes(list);
    } catch (e) {
      console.error("Failed to load notes:", e);
    }
  }

  async function refreshMemoryStatus() {
    try {
      const s: ModelMemoryStatus = await invoke("get_model_memory_status");
      setMemoryStatus(s);
    } catch (e) {
      console.error("Failed to check memory status:", e);
    }
  }

  async function handlePreload() {
    setIsPreloading(true);
    setErrorMsg(null);
    try {
      await invoke("preload_model_to_memory", {
        engine: settingsRef.current.local_engine,
        modelSize: settingsRef.current.local_model_size,
        customModelsDir: settingsRef.current.models_folder || undefined,
      });
      await refreshMemoryStatus();
      showToast("✓ Model loaded into RAM");
    } catch (err: any) {
      setErrorMsg(String(err));
    } finally {
      setIsPreloading(false);
    }
  }

  async function handleFreeRam() {
    setIsFreeingRam(true);
    setErrorMsg(null);
    try {
      await invoke("unload_model_from_memory");
      await refreshMemoryStatus();
      showToast("✓ Model unloaded from RAM");
    } catch (err: any) {
      setErrorMsg(String(err));
    } finally {
      setIsFreeingRam(false);
    }
  }

  async function loadLlmSettings() {
    try {
      const cfg: LlmSettings = await invoke("get_llm_config");
      const activeP = (cfg.providers || []).find((p) => p.id === cfg.active_provider_id);
      const activeModel = cfg.model || activeP?.model || "";
      const normalized: LlmSettings = {
        ...cfg,
        model: activeModel,
        presets: cfg.presets && cfg.presets.length > 0 ? cfg.presets : DEFAULT_PRESETS,
      };
      setLlmSettings(normalized);
      llmSettingsRef.current = normalized;
      if (cfg.active_provider_id) {
        setEditingProviderId(cfg.active_provider_id);
      }
    } catch (e) {
      console.error("Failed loading LLM config:", e);
    }
  }

  async function updateAndSaveLlmSettings(patch: Partial<LlmSettings>) {
    const next: LlmSettings = { ...llmSettingsRef.current, ...patch };
    setLlmSettings(next);
    llmSettingsRef.current = next;
    try {
      await invoke("save_llm_config", { config: next });
    } catch (e) {
      console.error("Failed saving LLM config:", e);
      setErrorMsg(String(e));
    }
  }

  function handleSelectOrUpdateModel(newModel: string) {
    const activeId = llmSettingsRef.current.active_provider_id;
    const trimmed = newModel.trim();
    const updatedProviders = (llmSettingsRef.current.providers || []).map((p) => {
      if (p.id === activeId) {
        const recents = p.recent_models || [];
        const newRecents = trimmed && !recents.includes(trimmed)
          ? [trimmed, ...recents.filter((m) => m !== trimmed)].slice(0, 6)
          : recents;
        return { ...p, model: trimmed, recent_models: newRecents };
      }
      return p;
    });
    updateAndSaveLlmSettings({
      model: newModel,
      providers: updatedProviders,
    });
  }

  function handleStartAddPreset() {
    setEditingPresetId(null);
    setIsAddingPreset(true);
    setPresetForm({ id: "", label: "", prompt: "" });
  }

  function handleStartEditPreset(p: PromptPreset) {
    setIsAddingPreset(false);
    setEditingPresetId(p.id);
    setPresetForm({ id: p.id, label: p.label, prompt: p.prompt });
  }

  function handleCancelPresetForm() {
    setIsAddingPreset(false);
    setEditingPresetId(null);
    setPresetForm({ id: "", label: "", prompt: "" });
  }

  async function handleSavePresetForm() {
    const label = presetForm.label.trim();
    const prompt = presetForm.prompt.trim();
    if (!label) {
      setErrorMsg("Preset name cannot be empty.");
      return;
    }
    if (!prompt) {
      setErrorMsg("Preset instruction prompt cannot be empty.");
      return;
    }

    const currentPresets =
      llmSettingsRef.current.presets && llmSettingsRef.current.presets.length > 0
        ? llmSettingsRef.current.presets
        : DEFAULT_PRESETS;

    if (isAddingPreset) {
      let id = presetForm.id.trim() || label.toLowerCase().replace(/[^a-z0-9]/g, "_");
      id = id.replace(/^_+|_+$/g, "") || `preset_${Date.now()}`;
      if (currentPresets.some((p) => p.id === id)) {
        id = `${id}_${Date.now().toString().slice(-4)}`;
      }
      const newPreset: PromptPreset = { id, label, prompt };
      const updated = [...currentPresets, newPreset];
      await updateAndSaveLlmSettings({ presets: updated, voice_preset: newPreset.id });
    } else if (editingPresetId) {
      const updated = currentPresets.map((p) =>
        p.id === editingPresetId ? { ...p, label, prompt } : p
      );
      await updateAndSaveLlmSettings({ presets: updated });
    }
    handleCancelPresetForm();
  }

  async function handleDeletePreset(idToDelete: string) {
    const currentPresets =
      llmSettingsRef.current.presets && llmSettingsRef.current.presets.length > 0
        ? llmSettingsRef.current.presets
        : DEFAULT_PRESETS;

    if (currentPresets.length <= 1) {
      setErrorMsg("You must keep at least one preset.");
      return;
    }
    const updated = currentPresets.filter((p) => p.id !== idToDelete);
    let newVoicePreset = llmSettingsRef.current.voice_preset;
    if (newVoicePreset === idToDelete) {
      newVoicePreset = updated[0]?.id || "custom";
    }
    await updateAndSaveLlmSettings({ presets: updated, voice_preset: newVoicePreset });
    try {
      await invoke("delete_preset_markdown", { id: idToDelete });
    } catch (e) {
      console.error("Failed deleting preset markdown file:", e);
    }
    if (editingPresetId === idToDelete) {
      handleCancelPresetForm();
    }
  }

  async function handleRestoreDefaultPresets() {
    try {
      const defaults: PromptPreset[] = await invoke("restore_presets_defaults");
      await updateAndSaveLlmSettings({
        presets: defaults,
        voice_preset: "grammar_fix",
      });
      showToast("✓ Presets restored to defaults");
    } catch (e) {
      console.error(e);
      await updateAndSaveLlmSettings({
        presets: DEFAULT_PRESETS,
        voice_preset: "grammar_fix",
      });
    }
    handleCancelPresetForm();
  }

  async function handleFetchModels(providerId?: string) {
    const targetId = providerId || llmSettingsRef.current.active_provider_id;
    setIsFetchingModels(true);
    setErrorMsg(null);
    try {
      const models: string[] = await invoke("fetch_llm_models", { providerId: targetId });
      setAvailableModels(models);
      showToast(`✓ Loaded ${models.length} available models`);
    } catch (err: any) {
      setErrorMsg(String(err));
    } finally {
      setIsFetchingModels(false);
    }
  }

  function handleUpdateProvider(id: string, patch: Partial<LlmProvider>) {
    const updated = llmSettingsRef.current.providers.map((p) =>
      p.id === id ? { ...p, ...patch } : p
    );
    updateAndSaveLlmSettings({ providers: updated });

    const currentAsrPid = settingsRef.current.provider_id || (llmSettingsRef.current.providers.find(p => p.base_url === settingsRef.current.api_base_url)?.id);
    if (currentAsrPid === id) {
      const p = updated.find((x) => x.id === id);
      if (p) {
        const asrPatch: Partial<AppSettings> = {};
        if (patch.base_url !== undefined) asrPatch.api_base_url = p.base_url;
        if (patch.api_key !== undefined) asrPatch.api_key = p.api_key;
        if (Object.keys(asrPatch).length > 0) {
          updateAndSaveSettings(asrPatch);
        }
      }
    }
  }

  function handleAddProvider(type: "cloudflare" | "groq" | "openai" | "ollama" | "custom") {
    const rand = Math.floor(1000 + Math.random() * 9000);
    const newId =
      type === "cloudflare"
        ? `CLOUDFLARE_${rand}`
        : type === "groq"
        ? `GROQ_${rand}`
        : type === "openai"
        ? `OPENAI_${rand}`
        : type === "ollama"
        ? `OLLAMA_${rand}`
        : `CUSTOM_${rand}`;

    const newName =
      type === "cloudflare"
        ? `Cloudflare ${rand}`
        : type === "groq"
        ? `Groq Cloud ${rand}`
        : type === "openai"
        ? `OpenAI ${rand}`
        : type === "ollama"
        ? `Ollama Local ${rand}`
        : `Custom Provider ${rand}`;

    const newProvider: LlmProvider = {
      id: newId,
      name: newName,
      provider_type: type,
      api_key: "",
      account_id: "",
      base_url:
        type === "cloudflare"
          ? "https://api.cloudflare.com/client/v4/accounts/<account_id>/ai/v1"
          : type === "groq"
          ? "https://api.groq.com/openai/v1"
          : type === "openai"
          ? "https://api.openai.com/v1"
          : type === "ollama"
          ? "http://localhost:11434/v1"
          : type === "custom"
          ? "http://localhost:8000/v1"
          : "",
    };

    const nextProviders = [...llmSettingsRef.current.providers, newProvider];
    updateAndSaveLlmSettings({
      providers: nextProviders,
      active_provider_id: newId,
    });
    setEditingProviderId(newId);
  }

  function hasCustomProviderInfo(p: LlmProvider): boolean {
    if (p.api_key?.trim() || p.account_id?.trim()) return true;
    const url = p.base_url?.trim() || "";
    if (!url) return false;
    if (p.provider_type === "groq" && url === "https://api.groq.com/openai/v1") return false;
    if (p.provider_type === "openai" && url === "https://api.openai.com/v1") return false;
    if (p.provider_type === "ollama" && url === "http://localhost:11434/v1") return false;
    if (p.provider_type === "cloudflare" && (url.includes("<account_id>") || url === "https://api.cloudflare.com/client/v4/accounts/<account_id>/ai/v1")) return false;
    return true;
  }

  function handleDeleteProvider(id: string) {
    if (llmSettingsRef.current.providers.length <= 1) {
      showToast("Cannot delete the only configured provider");
      return;
    }
    const target = llmSettingsRef.current.providers.find((p) => p.id === id);
    if (target && hasCustomProviderInfo(target)) {
      const ok = window.confirm(
        `Delete provider "${target.name}"?\n\nThis provider has saved credentials or custom configuration.`
      );
      if (!ok) return;
    }
    const filtered = llmSettingsRef.current.providers.filter((p) => p.id !== id);
    const nextActive =
      llmSettingsRef.current.active_provider_id === id
        ? filtered[0].id
        : llmSettingsRef.current.active_provider_id;

    updateAndSaveLlmSettings({
      providers: filtered,
      active_provider_id: nextActive,
    });
    setEditingProviderId(nextActive);
  }

  async function handleTransform(textOverride?: string, _isAuto = false) {
    const text = (textOverride !== undefined ? textOverride : rawTranscriptRef.current).trim();
    if (!text) {
      showToast("No transcription to transform. Speak or type first.");
      return;
    }

    setIsTransforming(true);
    setErrorMsg(null);
    try {
      const res: OperationResult = await invoke("transform_with_llm", {
        text,
        providerId: llmSettingsRef.current.active_provider_id,
        model: llmSettingsRef.current.model,
        preset: llmSettingsRef.current.voice_preset,
        customPrompt: llmSettingsRef.current.custom_prompt || undefined,
      });

      const transformed = (res && typeof res === "object" ? res.text : String(res || "")).trim();
      const itemNeurons = res?.cf_neurons ?? null;
      const itemCost = res?.cf_cost ?? null;

      setLlmResult(transformed);

      const activeId = activeIdRef.current;
      await persistNote(transformed, activeId);

      await loadCfUsage();
      const activeProvider = (llmSettingsRef.current.providers || []).find((p) => p.id === llmSettingsRef.current.active_provider_id);
      const isCfLlm = llmSettingsRef.current.enabled && (
        activeProvider?.provider_type === "cloudflare" ||
        (activeProvider?.base_url || "").includes("cloudflare.com")
      );
      const isLocalLlm = activeProvider?.provider_type === "ollama" || (activeProvider?.base_url || "").includes("localhost") || (activeProvider?.base_url || "").includes("127.0.0.1");

      let logMsg = `Transformed ${text.split(/\s+/).filter(Boolean).length} words -> ${transformed.split(/\s+/).filter(Boolean).length} words.`;
      if (isCfLlm) {
        if (itemNeurons !== null && itemNeurons !== undefined) {
          logMsg += ` Consumed: ${formatNeurons(itemNeurons)} Ⓝ (${formatCostUsd(itemCost || 0)})`;
        }
      } else if (isLocalLlm) {
        logMsg += ` (Local LLM - zero cost)`;
      }

      let logDetails = `Raw Input:\n${text}\n\nTransformed Output:\n${transformed}`;
      if (isCfLlm) {
        if (itemNeurons !== null && itemNeurons !== undefined) {
          logDetails += `\n\nCloudflare LLM Consumed: ${formatNeurons(itemNeurons)} Neurons (~${formatCostUsd(itemCost || 0)})`;
        }
      } else if (isLocalLlm) {
        logDetails += `\n\nLocal model inference - 100% offline, zero cloud cost.`;
      }

      addLog({
        level: "success",
        title: "LLM Transformation Succeeded",
        engine: `LLM (${llmSettingsRef.current.voice_preset})`,
        model: llmSettingsRef.current.model,
        message: logMsg,
        details: logDetails,
      });

      if (settingsRef.current.auto_copy) {
        await copyText(transformed);
      } else {
        showToast("✓ Transformed with LLM!");
      }
    } catch (err: any) {
      const errStr = String(err);
      setErrorMsg(errStr);
      addLog({
        level: "error",
        title: "LLM Transformation Failed",
        engine: "LLM",
        model: llmSettingsRef.current.model,
        message: errStr,
        details: `Timestamp: ${new Date().toISOString()}\nModel: ${llmSettingsRef.current.model}\nError:\n${errStr}`,
      });
    } finally {
      setIsTransforming(false);
    }
  }

  async function loadSettings() {
    try {
      const s: AppSettings = await invoke("get_settings");
      setSettings(s);
      settingsRef.current = s;
      await invoke("set_always_on_top", { alwaysOnTop: s.always_on_top });
      await refreshModelStatus(s.local_engine, s.local_model_size);
      await refreshMemoryStatus();
      await loadLlmSettings();

      // Check first-run wizard
      if (!s.wizard_completed && !s.api_key && s.engine_mode !== "local") {
        setWizardOpen(true);
      }
    } catch (e) {
      console.error("Failed to load settings:", e);
    }
  }

  async function handleFinishWizard(
    settingsPatch: any,
    llmPatch?: any
  ) {
    await updateAndSaveSettings(settingsPatch);
    if (llmPatch) {
      if (llmPatch.provider) {
        const existing = llmSettingsRef.current.providers || [];
        const provIdx = existing.findIndex((p) => p.id === llmPatch.provider.id);
        const updatedProviders =
          provIdx >= 0
            ? existing.map((p, i) => (i === provIdx ? { ...p, ...llmPatch.provider } : p))
            : [...existing, llmPatch.provider];
        await updateAndSaveLlmSettings({
          enabled: llmPatch.enabled,
          active_provider_id: llmPatch.active_provider_id,
          model: llmPatch.model,
          providers: updatedProviders,
        });
      } else {
        await updateAndSaveLlmSettings({
          enabled: llmPatch.enabled,
          active_provider_id: llmPatch.active_provider_id,
          model: llmPatch.model,
        });
      }
    }
  }

  async function updateAndSaveSettings(patch: Partial<AppSettings>) {
    const next: AppSettings = { ...settingsRef.current, ...patch };
    setSettings(next);
    settingsRef.current = next;
    try {
      await invoke("save_settings", { settings: next });
      if (patch.always_on_top !== undefined) {
        await invoke("set_always_on_top", { alwaysOnTop: patch.always_on_top });
      }
    } catch (e) {
      console.error("Failed to persist settings:", e);
      setErrorMsg(String(e));
    }
  }

  function openSettings(tab: "providers" | "asr" | "llm" | "preferences" | "logs" = "asr") {
    setSettingsNavTab(tab);
    loadSettings();
    loadLlmSettings();
    loadCfUsage();
    refreshModelStatus();
    refreshMemoryStatus();
    setActiveView("settings");
  }

  async function closeSettings() {
    try {
      await invoke("save_llm_config", { config: llmSettingsRef.current });
    } catch (e) {
      console.warn("Failed saving LLM config on close:", e);
    }
    setActiveView("notes");
  }

  async function toggleAlwaysOnTop(val?: boolean) {
    const nextVal = val !== undefined ? val : !settingsRef.current.always_on_top;
    await updateAndSaveSettings({ always_on_top: nextVal });
  }

  async function persistNote(text: string, existingId: number | null): Promise<number> {
    const title = deriveTitle(text);
    if (existingId) {
      await invoke("update_note", { id: existingId, title, content: text });
      await loadNotes();
      return existingId;
    } else {
      const saved: Note = await invoke("save_note", { title, content: text });
      await loadNotes();
      setActiveNoteId(saved.id);
      activeIdRef.current = saved.id;
      return saved.id;
    }
  }

  function showToast(msg: string) {
    setCopiedNotification(msg);
    setTimeout(() => setCopiedNotification(null), 2500);
  }

  async function toggleRecording() {
    if (isTranscribingRef.current) return;
    setErrorMsg(null);

    if (!isRecordingRef.current) {
      try {
        await invoke("start_recording");
        setIsRecording(true);
      } catch (err: any) {
        setErrorMsg(String(err));
      }
    } else {
      setIsRecording(false);
      setIsTranscribing(true);
      try {
        const res: OperationResult = await invoke("stop_recording_and_transcribe");
        const transcript = (res && typeof res === "object" ? res.text : String(res || "")).trim();
        const itemNeurons = res?.cf_neurons ?? null;
        const itemCost = res?.cf_cost ?? null;

        if (transcript) {
          const isMini = miniModeRef.current;
          const shouldCreateNewNote = isMini && (settingsRef.current.mini_record_mode !== "append");
          await loadCfUsage();

          const isCfAsr = settingsRef.current.engine_mode === "cloud" && (settingsRef.current.api_base_url || "").includes("api.cloudflare.com");
          const isLocalAsr = settingsRef.current.engine_mode === "local";
          const isTinyEn = isCfAsr && (settingsRef.current.model || "").includes("tiny");

          let asrMsg = `Transcribed ${transcript.trim().split(/\s+/).filter(Boolean).length} words.`;
          if (isTinyEn) {
            asrMsg += ` (whisper-tiny-en · Beta)`;
          } else if (isCfAsr) {
            if (itemNeurons !== null && itemNeurons !== undefined) {
              asrMsg += ` Consumed: ${formatNeurons(itemNeurons)} Ⓝ (${formatCostUsd(itemCost || 0)})`;
            }
          } else if (isLocalAsr) {
            asrMsg += ` (Local engine - zero cost)`;
          }

          let asrDetails = `Result: "${transcript.slice(0, 200)}${transcript.length > 200 ? "..." : ""}"`;
          if (isTinyEn) {
            asrDetails += `\n\nCloudflare Workers AI (whisper-tiny-en Beta · Fast) - zero neuron billing.`;
          } else if (isCfAsr) {
            if (itemNeurons !== null && itemNeurons !== undefined) {
              asrDetails += `\n\nCloudflare ASR Consumed: ${formatNeurons(itemNeurons)} Neurons (~${formatCostUsd(itemCost || 0)})`;
            }
          } else if (isLocalAsr) {
            asrDetails += `\n\nLocal engine (${settingsRef.current.local_engine}) - 100% offline, zero cloud cost.`;
          }

          if (llmSettingsRef.current.enabled) {
            // Dual-field multi-clip accumulation or fresh item
            let nextRaw: string;
            let targetNoteId: number | null;

            if (shouldCreateNewNote) {
              nextRaw = transcript;
              targetNoteId = null;
              setLlmResult("");
            } else {
              const currentRaw = rawTranscriptRef.current;
              nextRaw = currentRaw ? `${currentRaw.trim()} ${transcript}` : transcript;
              targetNoteId = activeIdRef.current;
            }

            setRawTranscript(nextRaw);
            rawTranscriptRef.current = nextRaw;
            setContent(nextRaw);
            await persistNote(nextRaw, targetNoteId);

            addLog({
              level: "success",
              title: "Transcription Successful",
              engine: settingsRef.current.engine_mode === "cloud" ? (isCfAsr ? "Cloudflare Workers AI" : "Remote API") : `Local (${settingsRef.current.local_engine})`,
              model: settingsRef.current.engine_mode === "cloud" ? (settingsRef.current.model || "(default)") : settingsRef.current.local_model_size,
              endpoint: settingsRef.current.engine_mode === "cloud" ? settingsRef.current.api_base_url : undefined,
              message: asrMsg,
              details: asrDetails,
            });

            if (llmSettingsRef.current.auto_mode) {
              await handleTransform(nextRaw, true);
            } else {
              if (settingsRef.current.auto_copy) {
                await copyText(transcript);
              } else {
                showToast("✓ Transcribed to raw buffer!");
              }
            }
          } else {
            // Standard scratchpad behavior or fresh item
            let nextContent: string;
            let targetNoteId: number | null;

            if (shouldCreateNewNote) {
              nextContent = transcript;
              targetNoteId = null;
              setLlmResult("");
            } else {
              const current = activeContentRef.current;
              nextContent = current ? `${current.trim()} ${transcript}` : transcript;
              targetNoteId = activeIdRef.current;
            }

            setContent(nextContent);
            setRawTranscript(nextContent);
            rawTranscriptRef.current = nextContent;
            await persistNote(nextContent, targetNoteId);

            addLog({
              level: "success",
              title: "Transcription Successful",
              engine: settingsRef.current.engine_mode === "cloud" ? (isCfAsr ? "Cloudflare Workers AI" : "Remote API") : `Local (${settingsRef.current.local_engine})`,
              model: settingsRef.current.engine_mode === "cloud" ? (settingsRef.current.model || "(default)") : settingsRef.current.local_model_size,
              endpoint: settingsRef.current.engine_mode === "cloud" ? settingsRef.current.api_base_url : undefined,
              message: asrMsg,
              details: asrDetails,
            });

            if (settingsRef.current.auto_copy) {
              await copyText(transcript);
            } else {
              showToast("✓ Transcribed!");
            }
          }
        }
      } catch (err: any) {
        const errStr = String(err);
        setErrorMsg(errStr);
        addLog({
          level: "error",
          title: "Transcription Failed",
          engine: settingsRef.current.engine_mode === "cloud" ? "Remote API" : `Local (${settingsRef.current.local_engine})`,
          model: settingsRef.current.engine_mode === "cloud" ? (settingsRef.current.model || "(default)") : settingsRef.current.local_model_size,
          endpoint: settingsRef.current.engine_mode === "cloud" ? settingsRef.current.api_base_url : undefined,
          message: errStr,
          details: `Timestamp: ${new Date().toISOString()}\nEngine Mode: ${settingsRef.current.engine_mode}\nModel: ${settingsRef.current.model || "(default)"}\nEndpoint: ${settingsRef.current.api_base_url || "(local)"}\nError:\n${errStr}`,
        });
      } finally {
        setIsTranscribing(false);
        refreshMemoryStatus();
      }
    }
  }

  async function toggleMiniMode(enable: boolean) {
    try {
      await invoke("set_mini_mode", { mini: enable });
      setMiniMode(enable);
      setErrorMsg(null);
    } catch (e: any) {
      console.error("Failed to toggle mini mode:", e);
    }
  }

  // Load autostart status on mount
  useEffect(() => {
    isAutostartEnabled()
      .then(setAutostartEnabled)
      .catch((e) => console.error("Failed to check autostart status:", e));
  }, []);

  async function handleToggleAutostart(enabled: boolean) {
    try {
      if (enabled) {
        await enableAutostart();
        setAutostartEnabled(true);
      } else {
        await disableAutostart();
        setAutostartEnabled(false);
      }
    } catch (err: any) {
      console.error("Failed to update autostart setting:", err);
      alert(`Could not update startup setting: ${err}`);
    }
  }

  // Sync tray icon with recording state
  useEffect(() => {
    invoke("set_tray_recording_state", { isRecording }).catch(() => {});
  }, [isRecording]);

  // Listen to system tray events
  useEffect(() => {
    let unlistenMini: (() => void) | undefined;
    let unlistenRec: (() => void) | undefined;
    let unlistenPref: (() => void) | undefined;

    listen("tray_toggle_mini", () => {
      toggleMiniMode(!miniModeRef.current);
    }).then((fn) => { unlistenMini = fn; });

    listen("tray_toggle_recording", () => {
      toggleRecording();
    }).then((fn) => { unlistenRec = fn; });

    listen("open_preferences", () => {
      openSettings("preferences");
    }).then((fn) => { unlistenPref = fn; });

    return () => {
      if (unlistenMini) unlistenMini();
      if (unlistenRec) unlistenRec();
      if (unlistenPref) unlistenPref();
    };
  }, []);

  function handleContentChange(val: string) {
    setContent(val);
    setRawTranscript(val);
    rawTranscriptRef.current = val;
  }

  async function handleBlurSave() {
    if (content.trim()) {
      await persistNote(content, activeNoteId);
    }
  }

  async function handleNewNote() {
    if (content.trim() && !activeNoteId) {
      await persistNote(content, null);
    }
    setActiveNoteId(null);
    setContent("");
    setRawTranscript("");
    rawTranscriptRef.current = "";
    setLlmResult("");
    setErrorMsg(null);
  }

  function handleSelectNote(note: Note) {
    setActiveNoteId(note.id);
    setContent(note.content);
    setRawTranscript(note.content);
    rawTranscriptRef.current = note.content;
    setLlmResult("");
    setErrorMsg(null);
    if (typeof window !== "undefined" && window.innerWidth <= 768) {
      setSidebarOpen(false);
    }
  }

  async function handleDeleteNote(e: React.MouseEvent, id: number) {
    e.stopPropagation();
    try {
      await invoke("delete_note", { id });
      if (activeNoteId === id) {
        setActiveNoteId(null);
        setContent("");
      }
      await loadNotes();
    } catch (err: any) {
      setErrorMsg(String(err));
    }
  }

  async function copyText(text: string, noteId?: number) {
    if (!text) return;
    let copied = false;

    // 1. Try native backend clipboard (bypasses browser focus and user gesture permission issues)
    try {
      await invoke("write_to_clipboard", { text });
      copied = true;
    } catch (rustErr) {
      console.warn("Native clipboard write failed, trying fallback:", rustErr);
    }

    // 2. Fallback to browser navigator.clipboard
    if (!copied && navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
      try {
        await navigator.clipboard.writeText(text);
        copied = true;
      } catch (navErr) {
        console.warn("Browser clipboard write failed:", navErr);
      }
    }

    if (copied) {
      if (noteId) {
        setCopiedId(noteId);
        setTimeout(() => setCopiedId(null), 1500);
      } else {
        showToast("✓ Copied to clipboard!");
      }
    }
  }

  async function handleSaveSettings(e?: React.FormEvent) {
    if (e) e.preventDefault();
    try {
      await invoke("save_settings", { settings });
      settingsRef.current = settings;
      await invoke("save_llm_config", { config: llmSettingsRef.current });
      showToast("✓ Settings saved!");
      setActiveView("notes");
    } catch (err: any) {
      setErrorMsg(String(err));
    }
  }

  const formatTimer = (secs: number) => {
    const m = Math.floor(secs / 60)
      .toString()
      .padStart(2, "0");
    const s = (secs % 60).toString().padStart(2, "0");
    return `${m}:${s}`;
  };

  const wordCount = content.trim() ? content.trim().split(/\s+/).length : 0;

  function handleDrag(e: React.MouseEvent) {
    if (e.buttons === 1) {
      invoke("start_drag").catch(() => {});
    }
  }

  // Mini Floating Wizard View (Transparent Floating Cluster)
  if (miniMode) {
    return (
      <div
        className="mini-widget"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) handleDrag(e);
        }}
        title="Drag pill to move. Alt+R to record. Alt+M to expand."
      >
        <div
          className="mini-drag-pill"
          onMouseDown={handleDrag}
          title="Drag to move"
        >
          ⋮⋮
        </div>

        <button
          className={`btn-mini-record ${isRecording ? "recording" : ""} ${
            isTranscribing ? "transcribing" : ""
          } ${copiedNotification ? "copied" : ""}`}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            toggleRecording();
          }}
          disabled={isTranscribing}
          title={
            isRecording
              ? `Stop Recording (${formatTimer(recordSeconds)})`
              : isTranscribing
              ? "Transcribing..."
              : copiedNotification
              ? "Copied to clipboard!"
              : "Record (Alt+R or Space)"
          }
        >
          {isRecording ? "■" : isTranscribing ? "…" : copiedNotification ? "✓" : "●"}
        </button>

        {isRecording && (
          <span
            className="mini-elapsed-pill"
            onMouseDown={(e) => e.stopPropagation()}
            title={`Recording: ${formatTimer(recordSeconds)}`}
          >
            {formatTimer(recordSeconds)}
          </span>
        )}

        <button
          className="btn-mini-float"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            toggleMiniMode(false);
          }}
          title="Expand to Full Note Editor (Alt+M)"
        >
          ⛶
        </button>
      </div>
    );
  }

  // Full View
  return (
    <div className="app-container">
      {/* Toast notification for clipboard copy */}
      {copiedNotification && (
        <div className="toast-clipboard">
          <span>{copiedNotification}</span>
        </div>
      )}

      {/* Sidebar: History (Only visible in Notes view) */}
      {activeView === "notes" && (
        <>
          <aside className={`sidebar ${sidebarOpen ? "" : "closed"}`}>
            <div className="sidebar-header">
              <span className="sidebar-title">History ({notes.length})</span>
              <button
                className="btn-icon"
                onClick={handleNewNote}
                title="New blank note"
              >
                +
              </button>
            </div>

            <div className="history-list">
              {notes.length === 0 ? (
                <div className="history-empty">No notes yet. Record or write one!</div>
              ) : (
                notes.map((note) => (
                  <div
                    key={note.id}
                    className={`history-item ${activeNoteId === note.id ? "active" : ""}`}
                    onClick={() => handleSelectNote(note)}
                  >
                    <div className="history-item-top">
                      <span className="history-item-title">{note.title}</span>
                      <span className="history-item-time">{formatDate(note.updated_at)}</span>
                    </div>
                    <div className="history-item-preview">{note.content || "Empty note"}</div>

                    <div className="history-actions">
                      <button
                        className="btn-icon"
                        title="Copy note"
                        onClick={(e) => {
                          e.stopPropagation();
                          copyText(note.content, note.id);
                        }}
                      >
                        {copiedId === note.id ? "✓" : "📋"}
                      </button>
                      <button
                        className="btn-icon"
                        title="Delete note"
                        onClick={(e) => handleDeleteNote(e, note.id)}
                      >
                        ✕
                      </button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </aside>
          {sidebarOpen && (
            <div
              className="sidebar-backdrop"
              onClick={() => setSidebarOpen(false)}
              aria-hidden="true"
            />
          )}
        </>
      )}

      {/* Main Content Area: Switch between Notes Scratchpad and Full-Page Settings */}
      {activeView === "settings" ? (
        <div className="settings-page">
          {/* Full-Height Left Navigation Sidebar */}
          <aside className="settings-sidebar">
            <div className="settings-sidebar-header">
              <button
                type="button"
                className="btn-back"
                onClick={closeSettings}
                title="Return to notes editor (Esc)"
              >
                ← Back <kbd className="key-hint">Esc</kbd>
              </button>
              <h2 className="settings-page-title">Settings</h2>
            </div>

            <nav className="settings-sidebar-nav" aria-label="Settings Navigation">
              <button
                type="button"
                className={`settings-nav-tab ${settingsNavTab === "providers" ? "active" : ""}`}
                onClick={() => {
                  setSettingsNavTab("providers");
                  loadLlmSettings();
                  loadCfUsage();
                }}
              >
                <span className="settings-nav-icon">🔌</span>
                <span className="settings-nav-label">Providers</span>
              </button>
              <button
                type="button"
                className={`settings-nav-tab ${settingsNavTab === "asr" ? "active" : ""}`}
                onClick={() => setSettingsNavTab("asr")}
              >
                <span className="settings-nav-icon">🎙</span>
                <span className="settings-nav-label">Audio / ASR</span>
              </button>
              <button
                type="button"
                className={`settings-nav-tab ${settingsNavTab === "llm" ? "active" : ""}`}
                onClick={() => {
                  setSettingsNavTab("llm");
                  loadLlmSettings();
                }}
              >
                <span className="settings-nav-icon">🤖</span>
                <span className="settings-nav-label">LLM Transform</span>
                {llmSettings.enabled && <span className="settings-badge-on">ON</span>}
              </button>
              <button
                type="button"
                className={`settings-nav-tab ${settingsNavTab === "preferences" ? "active" : ""}`}
                onClick={() => setSettingsNavTab("preferences")}
              >
                <span className="settings-nav-icon">⚙</span>
                <span className="settings-nav-label">Preferences</span>
              </button>
              <button
                type="button"
                className={`settings-nav-tab ${settingsNavTab === "logs" ? "active" : ""}`}
                onClick={() => setSettingsNavTab("logs")}
              >
                <span className="settings-nav-icon">📋</span>
                <span className="settings-nav-label">Diagnostic Logs</span>
                {logs.filter((l) => l.level === "error").length > 0 && (
                  <span className="tab-error-pill">
                    {logs.filter((l) => l.level === "error").length}
                  </span>
                )}
              </button>
            </nav>

            {settings.engine_mode === "local" && (
              <div className="settings-sidebar-footer">
                {memoryStatus?.is_loaded ? (
                  <div className="memory-badge-loaded">
                    <span className="memory-pulse-dot"></span>
                    <span>RAM: ~{memoryStatus.estimated_ram_mb} MB</span>
                  </div>
                ) : (
                  <div className="memory-badge-cold">
                    <span>○ RAM: Inactive (0 MB)</span>
                  </div>
                )}
              </div>
            )}
          </aside>

          {/* Settings Main Content Area */}
          <main className="settings-main">
            <div className="settings-content">
            {errorMsg && (
              <div className="toast-error" style={{ position: "relative", top: 0, left: 0, transform: "none", width: "100%", maxWidth: "100%", marginBottom: "14px" }}>
                <div className="toast-error-header">
                  <div className="toast-error-title-row">
                    <span>⚠️</span>
                    <span className="toast-error-title">Transcription Error</span>
                  </div>
                  <button className="btn-icon" onClick={() => setErrorMsg(null)} title="Dismiss">✕</button>
                </div>
                <div className="toast-error-message">{errorMsg}</div>
                <div className="toast-error-actions">
                  <button
                    type="button"
                    className="btn-toast-action"
                    onClick={async () => {
                      await copyText(`Transcription Error Log:\n${errorMsg}\nTime: ${new Date().toISOString()}\nEngine Mode: ${settings.engine_mode}\nModel: ${settings.model || "default"}\nEndpoint: ${settings.api_base_url || "(local)"}`);
                      showToast("✓ Error log copied!");
                    }}
                  >
                    📋 Copy Error Details
                  </button>
                </div>
              </div>
            )}

            {settingsNavTab === "providers" && (
              <>
                <div className="settings-section-card">
                  <div className="settings-section-heading" style={{ justifyContent: "space-between" }}>
                    <span>Configured AI &amp; Speech Providers</span>
                    <div style={{ display: "flex", gap: "6px" }}>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => handleAddProvider("cloudflare")}
                        title="Add Cloudflare Workers AI"
                      >
                        + Cloudflare
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => handleAddProvider("groq")}
                        title="Add Groq Cloud"
                      >
                        + Groq
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => handleAddProvider("openai")}
                        title="Add OpenAI"
                      >
                        + OpenAI
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => handleAddProvider("ollama")}
                        title="Add local Ollama"
                      >
                        + Ollama
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => handleAddProvider("custom")}
                        title="Add Custom endpoint"
                      >
                        + Custom
                      </button>
                    </div>
                  </div>

                  <div className="form-hint" style={{ marginBottom: "12px" }}>
                    Endpoints and API keys saved here are shared across both 🎙 ASR (Audio) and 🤖 LLM tabs.
                  </div>

                  <div className="llm-provider-list" style={{ marginBottom: "16px" }}>
                    {llmSettings.providers.map((p) => {
                      const isSelected = editingProviderId === p.id;
                      const isLlmActive = llmSettings.active_provider_id === p.id;
                      const currentAsrPid = settings.provider_id || (llmSettings.providers.find((pr) => pr.base_url === settings.api_base_url)?.id);
                      const isAsrActive = currentAsrPid === p.id;
                      return (
                        <div
                          key={p.id}
                          className={`llm-provider-card ${isSelected ? "active" : ""}`}
                          onClick={() => setEditingProviderId(p.id)}
                        >
                          <div className="llm-provider-info">
                            <span className="llm-provider-icon">
                              {p.provider_type === "cloudflare"
                                ? "☁"
                                : p.provider_type === "groq"
                                ? "⚡"
                                : p.provider_type === "openai"
                                ? "🤖"
                                : p.provider_type === "ollama"
                                ? "🦙"
                                : "⚙"}
                            </span>
                            <div>
                              <div className="llm-provider-name">{p.name}</div>
                              <div className="llm-provider-type">
                                {p.provider_type} • {p.api_key?.trim() ? "Key set" : p.provider_type === "ollama" ? "Local" : "No key"}
                              </div>
                            </div>
                          </div>
                          <div style={{ display: "flex", gap: "6px", alignItems: "center" }}>
                            {isAsrActive && (
                              <span className="badge-pill installed" title="Selected for Audio transcription" style={{ fontSize: "10px", padding: "2px 6px" }}>
                                🎙 Audio
                              </span>
                            )}
                            {isLlmActive && (
                              <span className="badge-pill installed" title="Selected for LLM assistant" style={{ fontSize: "10px", padding: "2px 6px" }}>
                                🤖 LLM
                              </span>
                            )}
                            {llmSettings.providers.length > 1 && (
                              <button
                                type="button"
                                className="btn-icon"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  handleDeleteProvider(p.id);
                                }}
                                title="Delete provider"
                              >
                                ✕
                              </button>
                            )}
                          </div>
                        </div>
                      );
                    })}
                  </div>

                  {(() => {
                    const cur = llmSettings.providers.find((p) => p.id === editingProviderId) || llmSettings.providers[0];
                    if (!cur) return null;
                    return (
                      <div style={{ background: "var(--bg-primary)", padding: "14px", borderRadius: "8px", border: "1px solid var(--border)", display: "flex", flexDirection: "column", gap: "12px" }}>
                        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                          <span style={{ fontSize: "13px", fontWeight: 600 }}>
                            Configure {cur.name} Credentials
                          </span>
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={() => handleFetchModels(cur.id)}
                            disabled={isFetchingModels}
                            title="Query provider endpoint to verify credentials"
                          >
                            {isFetchingModels ? "Testing..." : "🔍 Test Connection"}
                          </button>
                        </div>

                        <div className="form-group">
                          <label className="form-label">Provider Label</label>
                          <input
                            type="text"
                            className="form-input"
                            value={cur.name}
                            onChange={(e) => handleUpdateProvider(cur.id, { name: e.target.value })}
                          />
                        </div>

                        {cur.provider_type === "cloudflare" && (
                          <>
                            <div className="form-group">
                              <label className="form-label">Cloudflare Account ID</label>
                              <div className="input-with-button">
                                <input
                                  type="text"
                                  className="form-input"
                                  placeholder="e.g. c3a0b12984ef... (found in Cloudflare Dashboard)"
                                  value={(() => {
                                    const raw = cur.account_id || (() => {
                                      const match = cur.base_url?.match(/accounts\/([a-zA-Z0-9_-]+)/);
                                      return match && match[1] !== "<account_id>" && match[1] !== "{account_id}" ? match[1] : "";
                                    })();
                                    return revealAccountId || focusedAccountId === cur.id
                                      ? raw
                                      : maskAccountId(raw);
                                  })()}
                                  onFocus={() => setFocusedAccountId(cur.id)}
                                  onBlur={() => setFocusedAccountId(null)}
                                  onChange={(e) => {
                                    const acc = e.target.value.trim();
                                    const targetUrl = acc ? `https://api.cloudflare.com/client/v4/accounts/${acc}/ai/v1` : "https://api.cloudflare.com/client/v4/accounts/<account_id>/ai/v1";
                                    handleUpdateProvider(cur.id, {
                                      account_id: acc,
                                      base_url: targetUrl,
                                    });
                                  }}
                                />
                                <button
                                  type="button"
                                  className="btn btn-secondary btn-sm"
                                  style={{ minWidth: "56px" }}
                                  onClick={() => setRevealAccountId(!revealAccountId)}
                                  title={revealAccountId ? "Mask Account ID" : "Reveal full Account ID"}
                                >
                                  {revealAccountId ? "Mask" : "Show"}
                                </button>
                              </div>
                              <span className="form-hint">
                                Found in Cloudflare Dashboard &rarr; Workers &amp; Pages overview (right sidebar). URL is added automatically.
                              </span>
                            </div>

                            <div className="form-group">
                              <label className="form-label">API Key / Token</label>
                              <input
                                type="password"
                                className="form-input"
                                placeholder="Cloudflare API Token with Workers AI Read permissions"
                                value={cur.api_key}
                                onChange={(e) => handleUpdateProvider(cur.id, { api_key: e.target.value })}
                              />
                              <span className="form-hint">
                                Needs <em>Workers AI: Read</em> permissions. Stored securely in local configuration.
                              </span>
                            </div>

                            <div className="cf-usage-panel" style={{ marginTop: "12px" }}>
                              <div className="cf-usage-panel-header">
                                <div className="cf-usage-panel-title">
                                  <span>⚡</span>
                                  <span>Cloudflare Consumed Workers AI Usage</span>
                                </div>
                                <div className="cf-usage-reset-badge">
                                  Resets 00:00 UTC ({cfUsage?.today?.label || "Today"})
                                </div>
                              </div>

                              <div className="cf-usage-panel-stats">
                                <div className="cf-panel-stat-box">
                                  <span className="cf-panel-stat-label">Today's Consumed</span>
                                  <span className="cf-panel-stat-val">
                                    {formatNeurons(cfUsage?.today?.total_neurons ?? 0)}
                                    <span className="cf-panel-stat-sub"> Ⓝ</span>
                                  </span>
                                </div>

                                <div className="cf-panel-stat-box">
                                  <span className="cf-panel-stat-label">Consumed Cost</span>
                                  <span className="cf-panel-stat-val">
                                    {formatCostUsd(cfUsage?.today?.total_cost_usd ?? 0)}
                                    <span className="cf-panel-stat-sub"> @ $0.011/1k</span>
                                  </span>
                                </div>

                                <div className="cf-panel-stat-box">
                                  <span className="cf-panel-stat-label">Today's Speech</span>
                                  <span className="cf-panel-stat-val">
                                    {formatDurationSecs(cfUsage?.today?.asr_audio_secs ?? 0)}
                                    <span className="cf-panel-stat-sub"> ({cfUsage?.today?.asr_count ?? 0} {cfUsage?.today?.asr_count === 1 ? "rec" : "recs"})</span>
                                  </span>
                                </div>

                                <div className="cf-panel-stat-box">
                                  <span className="cf-panel-stat-label">Today's LLM Tokens</span>
                                  <span className="cf-panel-stat-val">
                                    {(cfUsage?.today?.llm_tokens ?? 0).toLocaleString()}
                                    <span className="cf-panel-stat-sub"> ({cfUsage?.today?.llm_count ?? 0} {cfUsage?.today?.llm_count === 1 ? "tx" : "txs"})</span>
                                  </span>
                                </div>

                                <div className="cf-panel-stat-box">
                                  <span className="cf-panel-stat-label">This Month Consumed</span>
                                  <span className="cf-panel-stat-val">
                                    {formatNeurons(cfUsage?.current_month?.total_neurons ?? 0)} Ⓝ
                                    <span className="cf-panel-stat-sub"> ({formatCostUsd(cfUsage?.current_month?.total_cost_usd ?? 0)})</span>
                                  </span>
                                </div>
                              </div>

                              <div className="cf-usage-panel-footer">
                                <span className="cf-panel-hint">
                                  Tracks Neurons consumed directly within Lipi (ASR + LLM). Cloudflare free tier resets at 00:00 UTC.
                                </span>
                                <div style={{ display: "flex", gap: "6px" }}>
                                  <button
                                    type="button"
                                    className="btn btn-secondary"
                                    style={{ fontSize: "11px", padding: "2px 8px" }}
                                    onClick={() => loadCfUsage()}
                                    disabled={loadingCfUsage}
                                    title="Refresh Cloudflare usage statistics"
                                  >
                                    🔄 Refresh
                                  </button>
                                  <button
                                    type="button"
                                    className="btn btn-secondary"
                                    style={{ fontSize: "11px", padding: "2px 8px", color: "var(--text-danger, #ef4444)" }}
                                    onClick={handleClearCfUsage}
                                    title="Reset local Cloudflare usage tracking data"
                                  >
                                    🗑 Reset
                                  </button>
                                </div>
                              </div>
                            </div>
                          </>
                        )}

                        {cur.provider_type === "groq" && (
                          <div className="form-group">
                            <label className="form-label">Groq API Key</label>
                            <input
                              type="password"
                              className="form-input"
                              placeholder="gsk_..."
                              value={cur.api_key}
                              onChange={(e) => handleUpdateProvider(cur.id, { api_key: e.target.value })}
                            />
                            <span className="form-hint">From console.groq.com/keys.</span>
                          </div>
                        )}

                        {cur.provider_type === "openai" && (
                          <div className="form-group">
                            <label className="form-label">OpenAI API Key</label>
                            <input
                              type="password"
                              className="form-input"
                              placeholder="sk-..."
                              value={cur.api_key}
                              onChange={(e) => handleUpdateProvider(cur.id, { api_key: e.target.value })}
                            />
                          </div>
                        )}

                        {cur.provider_type === "ollama" && (
                          <>
                            <div className="form-group">
                              <label className="form-label">Ollama API Base URL</label>
                              <input
                                type="text"
                                className="form-input"
                                placeholder="http://localhost:11434/v1"
                                value={cur.base_url || "http://localhost:11434/v1"}
                                onChange={(e) => handleUpdateProvider(cur.id, { base_url: e.target.value })}
                              />
                              <span className="form-hint">
                                Runs locally via Ollama API. Make sure Ollama is running (<code>ollama serve</code>). No API key needed.
                              </span>
                            </div>
                            <div className="form-group">
                              <label className="form-label">API Key / Token (Optional)</label>
                              <input
                                type="password"
                                className="form-input"
                                placeholder="Optional (not required for local Ollama)"
                                value={cur.api_key}
                                onChange={(e) => handleUpdateProvider(cur.id, { api_key: e.target.value })}
                              />
                            </div>
                          </>
                        )}

                        {cur.provider_type === "custom" && (
                          <>
                            <div className="form-group">
                              <label className="form-label">Base URL</label>
                              <input
                                type="text"
                                className="form-input"
                                placeholder="http://localhost:11434/v1 or http://localhost:8000/v1"
                                value={cur.base_url}
                                onChange={(e) => handleUpdateProvider(cur.id, { base_url: e.target.value })}
                              />
                            </div>
                            <div className="form-group">
                              <label className="form-label">API Key / Token (Optional)</label>
                              <input
                                type="password"
                                className="form-input"
                                placeholder="Bearer token if required by server"
                                value={cur.api_key}
                                onChange={(e) => handleUpdateProvider(cur.id, { api_key: e.target.value })}
                              />
                            </div>
                          </>
                        )}
                      </div>
                    );
                  })()}
                </div>
              </>
            )}

            {settingsNavTab === "asr" && (
              <>
                {/* Section 1: ASR Engine Mode */}
                <div className="settings-section-card">
                  <div className="settings-section-heading">
                    <span>ASR Engine Mode (Speech-to-Text)</span>
                  </div>
                  <div className="settings-tabs" style={{ marginBottom: 0 }}>
                <button
                  type="button"
                  className={`settings-tab ${settings.engine_mode === "cloud" ? "active" : ""}`}
                  onClick={() => updateAndSaveSettings({ engine_mode: "cloud" })}
                >
                  🌐 Custom / Remote API (OpenAI-Compatible)
                </button>
                <button
                  type="button"
                  className={`settings-tab ${settings.engine_mode === "local" ? "active" : ""}`}
                  onClick={() => {
                    updateAndSaveSettings({ engine_mode: "local" });
                    refreshModelStatus(settingsRef.current.local_engine, settingsRef.current.local_model_size);
                    refreshMemoryStatus();
                  }}
                >
                  ⚡ Local Offline Engine (Private & Zero-Cost)
                </button>
              </div>

              {settings.engine_mode === "cloud" ? (
                <div style={{ display: "flex", flexDirection: "column", gap: "12px", marginTop: "10px" }}>
                  {/* Unified Provider Selection */}
                  <div className="form-group">
                    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "6px" }}>
                      <label className="form-label" style={{ margin: 0 }}>Transcription Provider</label>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => {
                          const curPid = settings.provider_id || (llmSettings.providers.find((p) => p.base_url === settings.api_base_url)?.id) || llmSettings.providers[0]?.id;
                          if (curPid) setEditingProviderId(curPid);
                          setSettingsNavTab("providers");
                        }}
                      >
                        ⚙ Manage Providers
                      </button>
                    </div>
                    <select
                      className="form-input form-select"
                      value={settings.provider_id || (llmSettings.providers.find((p) => p.base_url === settings.api_base_url)?.id) || llmSettings.providers[0]?.id || ""}
                      onChange={(e) => {
                        const pid = e.target.value;
                        const prov = llmSettings.providers.find((p) => p.id === pid);
                        if (prov) {
                          let defaultModel = settings.model;
                          if (prov.provider_type === "groq") defaultModel = "whisper-large-v3-turbo";
                          else if (prov.provider_type === "cloudflare") defaultModel = "@cf/openai/whisper";
                          else if (prov.provider_type === "openai" || prov.provider_type === "ollama") defaultModel = "whisper-1";
                          const updated: Partial<AppSettings> = {
                            provider_id: prov.id,
                            api_base_url: prov.base_url,
                            api_key: prov.api_key,
                            model: defaultModel,
                          };
                          setSettings((prev) => ({ ...prev, ...updated }));
                          updateAndSaveSettings(updated);
                        }
                      }}
                    >
                      {llmSettings.providers.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name} ({p.provider_type})
                        </option>
                      ))}
                    </select>
                    {(() => {
                      const curPid = settings.provider_id || (llmSettings.providers.find((pr) => pr.base_url === settings.api_base_url)?.id) || llmSettings.providers[0]?.id;
                      const prov = llmSettings.providers.find((p) => p.id === curPid);
                      if (!prov) return null;
                      return (
                        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: "4px", fontSize: "12px", color: "var(--text-secondary)" }}>
                          <span>Endpoint: {prov.base_url || "Default"}</span>
                          <span>{prov.api_key?.trim() ? "✓ Key Configured" : prov.provider_type === "ollama" ? "✓ Local Server" : "⚠ Key Missing in Providers tab"}</span>
                        </div>
                      );
                    })()}
                  </div>

                  <div className="form-group">
                    <label className="form-label">Audio Model Name</label>
                    <input
                      type="text"
                      className="form-input"
                      placeholder="whisper-1, whisper-large-v3-turbo, or @cf/openai/whisper"
                      value={settings.model}
                      onChange={(e) => setSettings({ ...settings, model: e.target.value })}
                      onBlur={() => updateAndSaveSettings({ model: settings.model })}
                    />
                    {(() => {
                      const curPid = settings.provider_id || (llmSettings.providers.find((pr) => pr.base_url === settings.api_base_url)?.id) || llmSettings.providers[0]?.id;
                      const prov = llmSettings.providers.find((p) => p.id === curPid);
                      const pType = prov?.provider_type;
                      if (pType === "cloudflare") {
                        return (
                          <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "6px" }}>
                            {[
                              { id: "@cf/openai/whisper", label: "whisper (Standard · ~41.1 Ⓝ/min)" },
                              { id: "@cf/openai/whisper-large-v3-turbo", label: "large-v3-turbo (Turbo · ~46.6 Ⓝ/min)" },
                              { id: "@cf/openai/whisper-tiny-en", label: "whisper-tiny-en (Beta · Fast)" },
                            ].map((item) => (
                              <button
                                key={item.id}
                                type="button"
                                className={`provider-chip ${settings.model === item.id ? "active" : ""}`}
                                style={{ fontSize: "11px", padding: "3px 8px" }}
                                onClick={() => {
                                  setSettings({ ...settings, model: item.id });
                                  updateAndSaveSettings({ model: item.id });
                                }}
                              >
                                {item.label}
                              </button>
                            ))}
                          </div>
                        );
                      }
                      if (pType === "groq") {
                        return (
                          <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "6px" }}>
                            {[
                              { id: "whisper-large-v3-turbo", label: "whisper-large-v3-turbo (Ultra-Fast)" },
                              { id: "whisper-large-v3", label: "whisper-large-v3" },
                            ].map((item) => (
                              <button
                                key={item.id}
                                type="button"
                                className={`provider-chip ${settings.model === item.id ? "active" : ""}`}
                                style={{ fontSize: "11px", padding: "3px 8px" }}
                                onClick={() => {
                                  setSettings({ ...settings, model: item.id });
                                  updateAndSaveSettings({ model: item.id });
                                }}
                              >
                                {item.label}
                              </button>
                            ))}
                          </div>
                        );
                      }
                      return (
                        <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "6px" }}>
                          {[
                            { id: "whisper-1", label: "whisper-1 (OpenAI standard)" },
                          ].map((item) => (
                            <button
                              key={item.id}
                              type="button"
                              className={`provider-chip ${settings.model === item.id ? "active" : ""}`}
                              style={{ fontSize: "11px", padding: "3px 8px" }}
                              onClick={() => {
                                setSettings({ ...settings, model: item.id });
                                updateAndSaveSettings({ model: item.id });
                              }}
                            >
                              {item.label}
                            </button>
                          ))}
                        </div>
                      );
                    })()}
                    <span className="form-hint">
                      Model used for speech-to-text with the selected provider.
                    </span>
                  </div>

                  <div className="form-group" style={{ marginTop: "12px", paddingTop: "12px", borderTop: "1px solid var(--border)" }}>
                    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "8px" }}>
                      <label className="form-label" style={{ margin: 0 }}>Request Timeout (applies to both ASR &amp; LLM)</label>
                      <span style={{ fontSize: "12px", color: "var(--text-secondary)", fontWeight: 500 }}>
                        {Math.floor((settings.request_timeout_secs || 180) / 60)}m {(settings.request_timeout_secs || 180) % 60}s ({settings.request_timeout_secs || 180}s)
                      </span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: "10px", marginBottom: "8px" }}>
                      <input
                        type="number"
                        min={180}
                        step={30}
                        className="form-input"
                        style={{ width: "110px" }}
                        value={settings.request_timeout_secs || 180}
                        onChange={(e) => {
                          const val = Math.max(180, parseInt(e.target.value, 10) || 180);
                          updateAndSaveSettings({ request_timeout_secs: val });
                          updateAndSaveLlmSettings({ request_timeout_secs: val });
                        }}
                      />
                      <span style={{ fontSize: "12px", color: "var(--text-secondary)" }}>
                        seconds (min 180s / 3 min)
                      </span>
                    </div>
                    <div className="timeout-pills">
                      {[
                        { label: "3 min (180s)", val: 180 },
                        { label: "5 min (300s)", val: 300 },
                        { label: "10 min (600s)", val: 600 },
                      ].map((opt) => (
                        <button
                          key={opt.val}
                          type="button"
                          className={`timeout-pill ${(settings.request_timeout_secs || 180) === opt.val ? "active" : ""}`}
                          onClick={() => {
                            updateAndSaveSettings({ request_timeout_secs: opt.val });
                            updateAndSaveLlmSettings({ request_timeout_secs: opt.val });
                          }}
                        >
                          {opt.label}
                        </button>
                      ))}
                    </div>
                    <span className="form-hint" style={{ marginTop: "6px", display: "block" }}>
                      Custom timeout for network requests. Configured timeout applies to both speech transcription (ASR) and LLM rectification requests.
                    </span>
                  </div>
                </div>
              ) : (
                <div style={{ display: "flex", flexDirection: "column", gap: "12px", marginTop: "10px" }}>
                  <div className="form-group">
                    <label className="form-label">Select Local Engine</label>
                    <select
                      className="form-input form-select"
                      value={settings.local_engine}
                      onChange={(e) => {
                        const eng = e.target.value as any;
                        updateAndSaveSettings({ local_engine: eng });
                        refreshModelStatus(eng, settingsRef.current.local_model_size);
                        refreshMemoryStatus();
                      }}
                    >
                      <option value="whisper_cpu">
                        Whisper (Standard CPU) — Reference whisper.cpp, universal compatibility
                      </option>
                      <option value="faster_whisper">
                        Faster-Whisper (CPU) — CTranslate2 INT8 quantization, high accuracy
                      </option>
                      <option value="whisper_vulkan">
                        Vulkan-Accelerated Whisper (GPU) — Hardware GPU compute (cross-vendor)
                      </option>
                    </select>
                  </div>
                </div>
              )}
            </div>

            {/* If Local Engine: Runner, Storage, Memory */}
            {settings.engine_mode === "local" && (
              <>
                {/* Stage 1: Engine Environment & Runner Binary */}
                <div className="settings-section-card">
                  <div className="settings-section-heading">
                    <span>1. Engine Runner & Environment</span>
                  </div>
                  <div className="runner-card">
                    <div className="runner-info">
                      <div className="runner-status">
                        {modelStatus?.binary_available ? (
                          <span className="text-success">✓ Runner environment ready</span>
                        ) : (
                          <span className="text-warning">⚠ Runner binary/runtime missing</span>
                        )}
                      </div>
                      <div className="runner-path" title={modelStatus?.binary_path || ""}>
                        {modelStatus?.binary_available
                          ? modelStatus.binary_path
                          : settings.local_engine === "faster_whisper"
                          ? "Python faster-whisper virtualenv not configured"
                          : "whisper-cli / whisper-server runner binary not downloaded yet"}
                      </div>
                    </div>
                    {!modelStatus?.binary_available && (
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={handlePrepareRunner}
                        disabled={isInstallingDeps}
                      >
                        {isInstallingDeps
                          ? "Setting up..."
                          : settings.local_engine === "faster_whisper"
                          ? "⚡ Setup Python Environment"
                          : "⬇ Download Runner Binary"}
                      </button>
                    )}
                  </div>
                </div>

                {/* Stage 2: Storage Location & Model Weights */}
                <div className="settings-section-card">
                  <div className="settings-section-heading">
                    <span>2. Storage Location & Model Weights</span>
                  </div>

                  <div className="form-group">
                    <label className="form-label">Model Storage Directory</label>
                    <div className="input-with-button">
                      <input
                        type="text"
                        className="form-input"
                        placeholder={modelStatus?.models_dir || "Default: ~/.local/share/com.lipi.app/models"}
                        value={settings.models_folder || ""}
                        onChange={(e) =>
                          setSettings({ ...settings, models_folder: e.target.value })
                        }
                        onBlur={() => {
                          updateAndSaveSettings({ models_folder: settings.models_folder });
                          refreshModelStatus(undefined, undefined, settings.models_folder);
                        }}
                      />
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={handlePickDirectory}
                        title="Browse directory (e.g. external drive or USB)"
                      >
                        📁 Browse
                      </button>
                      {settings.models_folder && (
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm"
                          onClick={() => {
                            updateAndSaveSettings({ models_folder: "" });
                            refreshModelStatus(undefined, undefined, "");
                          }}
                          title="Reset to default local app folder"
                        >
                          ↺ Reset
                        </button>
                      )}
                    </div>
                    <span className="form-hint" style={{ marginTop: "4px" }}>
                      Tip: You can select an external SSD or USB drive to store models. Weights load into RAM during transcription.
                    </span>
                  </div>

                  <div className="form-group">
                    <label className="form-label">Model Size</label>
                    <select
                      className="form-input form-select"
                      value={settings.local_model_size}
                      onChange={(e) => {
                        const sz = e.target.value as any;
                        updateAndSaveSettings({ local_model_size: sz });
                        refreshModelStatus(settingsRef.current.local_engine, sz);
                      }}
                    >
                      <option value="tiny">Tiny (~75 MB) — Ultra fast, lowest memory usage</option>
                      <option value="base">Base (~142 MB) — Recommended standard balance</option>
                      <option value="small">Small (~466 MB) — High precision, multilingual</option>
                    </select>
                  </div>

                  <div className="model-status-card">
                    <div className="model-status-header">
                      <div className="model-status-title">
                        <span className="model-name">
                          {settings.local_engine === "faster_whisper"
                            ? `faster-whisper-${settings.local_model_size}`
                            : `ggml-${settings.local_model_size}.bin`}
                        </span>
                        <span className="model-engine-badge">
                          {settings.local_engine === "whisper_cpu"
                            ? "CPU"
                            : settings.local_engine === "faster_whisper"
                            ? "CTranslate2"
                            : "Vulkan GPU"}
                        </span>
                      </div>
                      <span className={`badge-pill ${modelStatus?.installed ? "installed" : "missing"}`}>
                        {modelStatus?.installed
                          ? `✓ Installed ${
                              modelStatus.file_size_bytes > 0
                                ? `(${(modelStatus.file_size_bytes / 1024 / 1024).toFixed(1)} MB)`
                                : ""
                            }`
                          : "Not Downloaded"}
                      </span>
                    </div>

                    {downloadProgress !== null && (
                      <div className="progress-container">
                        <div className="progress-bar">
                          <div
                            className="progress-fill"
                            style={{ width: `${Math.min(100, downloadProgress)}%` }}
                          ></div>
                        </div>
                        <span className="progress-text">
                          {downloadProgress >= 100
                            ? "Finalizing model weights..."
                            : `Downloading weights: ${downloadProgress}%`}
                        </span>
                      </div>
                    )}

                    <div className="model-actions">
                      {!modelStatus?.installed ? (
                        <button
                          type="button"
                          className="btn btn-primary btn-sm"
                          onClick={handleDownloadModel}
                          disabled={isDownloading}
                        >
                          {isDownloading ? "Downloading..." : `⬇ Download ${settings.local_model_size} Model`}
                        </button>
                      ) : (
                        <button
                          type="button"
                          className="btn btn-danger-outline btn-sm"
                          onClick={handleDeleteModel}
                          disabled={isDownloading}
                        >
                          🗑 Delete Weights
                        </button>
                      )}
                    </div>
                  </div>
                </div>

                {/* Stage 3: Memory & RAM Performance */}
                <div className="settings-section-card">
                  <div className="settings-section-heading">
                    <span>3. In-Memory Daemon & RAM Management</span>
                  </div>

                  <div className="memory-ram-card">
                    <div className="memory-status-row">
                      {memoryStatus?.is_loaded ? (
                        <div className="memory-badge-loaded">
                          <span className="memory-pulse-dot"></span>
                          <span>Active in RAM (~{memoryStatus.estimated_ram_mb} MB)</span>
                        </div>
                      ) : (
                        <div className="memory-badge-cold">
                          <span>○ Cold / Inactive (0 MB RAM)</span>
                        </div>
                      )}

                      <div style={{ display: "flex", gap: "8px" }}>
                        {memoryStatus?.is_loaded ? (
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={handleFreeRam}
                            disabled={isFreeingRam}
                            title="Immediately unload the model from system RAM"
                          >
                            {isFreeingRam ? "Freeing..." : "🧹 Free RAM Now"}
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="btn btn-primary btn-sm"
                            onClick={handlePreload}
                            disabled={isPreloading || !modelStatus?.installed}
                            title="Preload model weights into RAM for instant transcription"
                          >
                            {isPreloading ? "Loading..." : "⚡ Pre-load to RAM"}
                          </button>
                        )}
                      </div>
                    </div>

                    {memoryStatus?.is_loaded ? (
                      <div className="memory-details-grid">
                        <div className="memory-stat-item">
                          <span className="memory-stat-label">Running Daemon</span>
                          <span className="memory-stat-value">
                            {memoryStatus.engine} ({memoryStatus.model_size})
                          </span>
                        </div>
                        <div className="memory-stat-item">
                          <span className="memory-stat-label">Daemon Port</span>
                          <span className="memory-stat-value">
                            127.0.0.1:{memoryStatus.port}
                          </span>
                        </div>
                        <div className="memory-stat-item">
                          <span className="memory-stat-label">Idle Duration</span>
                          <span className="memory-stat-value">
                            {formatDuration(memoryStatus.idle_seconds)}
                          </span>
                        </div>
                        <div className="memory-stat-item">
                          <span className="memory-stat-label">Auto-Unload</span>
                          <span className="memory-stat-value">
                            {(settings.model_idle_timeout_mins ?? 10) === 0
                              ? "Disabled"
                              : `in ${formatDuration(
                                  Math.max(
                                    0,
                                    (settings.model_idle_timeout_mins ?? 10) * 60 -
                                      memoryStatus.idle_seconds
                                  )
                                )}`}
                          </span>
                        </div>
                      </div>
                    ) : (
                      <span className="form-hint">
                        Model is cold on disk. It will automatically load into memory on your first recording, or click Pre-load above.
                      </span>
                    )}

                    <div className="timeout-selector-group">
                      <label className="form-label">Idle Inactivity Timeout</label>
                      <div className="timeout-pills">
                        {[
                          { label: "2 mins", val: 2 },
                          { label: "5 mins", val: 5 },
                          { label: "10 mins (Default)", val: 10 },
                          { label: "30 mins", val: 30 },
                          { label: "Never (Stay in RAM)", val: 0 },
                        ].map((opt) => (
                          <button
                            key={opt.val}
                            type="button"
                            className={`timeout-pill ${(settings.model_idle_timeout_mins ?? 10) === opt.val ? "active" : ""}`}
                            onClick={() => updateAndSaveSettings({ model_idle_timeout_mins: opt.val })}
                          >
                            {opt.label}
                          </button>
                        ))}
                      </div>
                      <span className="form-hint">
                        Automatically frees RAM when you aren't actively dictating. Set to 'Never' if you dictate frequently and want zero warm-up lag.
                      </span>
                    </div>

                    <div className="hardware-tip-box">
                      <span className="hardware-tip-icon">💡</span>
                      <div>
                        <strong>Hardware Health & Performance Protection:</strong> AI model weights (75 MB - 500 MB+) put wear on consumer SSDs and USB drives if loaded repeatedly on every single voice clip. Lipi preserves storage health and delivers instant zero-latency speech-to-text by keeping the model warm in RAM while you work.
                      </div>
                    </div>
                  </div>
                </div>
              </>
            )}

            {/* Speech Recognition Language Options Card */}
            <div className="settings-section-card">
              <div className="settings-section-heading">
                <span>Speech Recognition Language</span>
              </div>

              <div className="form-group">
                <label className="form-label">Spoken Language</label>
                <div style={{ display: "flex", gap: "10px", alignItems: "center", flexWrap: "wrap" }}>
                  <select
                    className="form-input form-select"
                    style={{ maxWidth: "260px" }}
                    value={
                      [
                        "",
                        "en",
                        "bn",
                        "hi",
                        "es",
                        "fr",
                        "de",
                        "zh",
                        "ja",
                        "ar",
                        "pt",
                        "ru",
                        "it",
                        "ko",
                      ].includes(settings.language || "")
                        ? settings.language || ""
                        : "custom"
                    }
                    onChange={(e) => {
                      const val = e.target.value;
                      if (val !== "custom") {
                        setSettings({ ...settings, language: val });
                        updateAndSaveSettings({ language: val });
                      }
                    }}
                  >
                    <option value="">🌐 Auto-Detect (Multilingual)</option>
                    <option value="en">🇺🇸 English (en)</option>
                    <option value="bn">🇧🇩 Bengali / বাংলা (bn)</option>
                    <option value="hi">🇮🇳 Hindi / हिन्दी (hi)</option>
                    <option value="es">🇪🇸 Spanish / Español (es)</option>
                    <option value="fr">🇫🇷 French / Français (fr)</option>
                    <option value="de">🇩🇪 German / Deutsch (de)</option>
                    <option value="zh">🇨🇳 Chinese / 中文 (zh)</option>
                    <option value="ja">🇯🇵 Japanese / 日本語 (ja)</option>
                    <option value="ar">🇸🇦 Arabic / العربية (ar)</option>
                    <option value="pt">🇵🇹 Portuguese / Português (pt)</option>
                    <option value="ru">🇷🇺 Russian / Русский (ru)</option>
                    <option value="it">🇮🇹 Italian / Italiano (it)</option>
                    <option value="ko">🇰🇷 Korean / 한국어 (ko)</option>
                    <option value="custom">🛠 Custom ISO Code...</option>
                  </select>

                  <input
                    type="text"
                    className="form-input"
                    style={{ maxWidth: "160px" }}
                    placeholder="ISO code (e.g. en, bn)"
                    value={settings.language || ""}
                    onChange={(e) =>
                      setSettings({ ...settings, language: e.target.value })
                    }
                    onBlur={() => updateAndSaveSettings({ language: settings.language })}
                  />
                </div>
                <span className="form-hint">
                  Explicitly choosing your spoken language avoids Whisper's automatic language detection step, reducing latency and avoiding mistaken language detection on short utterances.
                </span>
              </div>

              <div className="language-quick-chips">
                {[
                  { code: "", label: "Auto-detect" },
                  { code: "en", label: "English" },
                  { code: "bn", label: "Bengali (বাংলা)" },
                  { code: "hi", label: "Hindi (हिन्दी)" },
                  { code: "es", label: "Spanish" },
                  { code: "fr", label: "French" },
                  { code: "de", label: "German" },
                  { code: "zh", label: "Chinese" },
                  { code: "ja", label: "Japanese" },
                ].map((lang) => {
                  const isSel = (settings.language || "") === lang.code;
                  return (
                    <button
                      key={lang.code || "auto"}
                      type="button"
                      className={`provider-chip ${isSel ? "active" : ""}`}
                      style={{ fontSize: "11px", padding: "3px 8px" }}
                      onClick={() => {
                        setSettings({ ...settings, language: lang.code });
                        updateAndSaveSettings({ language: lang.code });
                      }}
                    >
                      {lang.label}
                    </button>
                  );
                })}
              </div>
            </div>
          </>
        )}

            {/* Section: LLM Post-Processing */}
            {settingsNavTab === "llm" && (
              <>
                <div className="settings-section-card">
                <div className="settings-section-heading" style={{ justifyContent: "space-between" }}>
                  <span>LLM Post-Processing &amp; Rectification</span>
                  <span className={`badge-pill ${llmSettings.enabled ? "installed" : "missing"}`}>
                    {llmSettings.enabled ? "Active" : "Disabled"}
                  </span>
                </div>

                <div className="form-group" style={{ marginBottom: "16px" }}>
                  <label className="checkbox-label" style={{ fontWeight: 600, fontSize: "14px" }}>
                    <input
                      type="checkbox"
                      className="checkbox-input"
                      checked={llmSettings.enabled}
                      onChange={(e) => updateAndSaveLlmSettings({ enabled: e.target.checked })}
                    />
                    <span>Enable LLM Speech Transformation &amp; Grammar Rectification</span>
                  </label>
                  <span className="form-hint" style={{ marginLeft: "26px" }}>
                    Transforms your speech using an OpenAI-compatible endpoint. Supports Cloudflare Workers AI, Groq, OpenAI, and local models.
                  </span>
                </div>

                {llmSettings.enabled && (
                  <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
                    {/* Active LLM Provider Selection */}
                    <div className="form-group">
                      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "6px" }}>
                        <label className="form-label" style={{ margin: 0 }}>Active LLM Provider</label>
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm"
                          onClick={() => {
                            setEditingProviderId(llmSettings.active_provider_id);
                            setSettingsNavTab("providers");
                          }}
                        >
                          ⚙ Manage Providers
                        </button>
                      </div>
                      <select
                        className="form-input form-select"
                        value={llmSettings.active_provider_id}
                        onChange={(e) => {
                          const newPid = e.target.value;
                          const targetProv = llmSettings.providers.find((p) => p.id === newPid);
                          const newModel = targetProv?.model || "";
                          updateAndSaveLlmSettings({
                            active_provider_id: newPid,
                            model: newModel,
                          });
                          setAvailableModels([]);
                        }}
                      >
                        {llmSettings.providers.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.name} ({p.provider_type})
                          </option>
                        ))}
                      </select>
                      {(() => {
                        const cur = llmSettings.providers.find((p) => p.id === llmSettings.active_provider_id);
                        if (!cur) return null;
                        return (
                          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: "4px", fontSize: "12px", color: "var(--text-secondary)" }}>
                            <span>Endpoint: {cur.base_url || "Default endpoint"}</span>
                            <span>{cur.api_key?.trim() ? "✓ Key Configured" : cur.provider_type === "ollama" ? "✓ Local Server" : "⚠ Key Missing in Providers tab"}</span>
                          </div>
                        );
                      })()}
                    </div>

                    {/* Model Selection with Fetch Models */}
                    <div className="form-group">
                      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "6px" }}>
                        <label className="form-label" style={{ margin: 0 }}>Model Name</label>
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm"
                          onClick={() => handleFetchModels()}
                          disabled={isFetchingModels}
                          title="Query provider /models endpoint for available models"
                        >
                          {isFetchingModels ? "🔄 Fetching..." : "🔄 Fetch Available Models"}
                        </button>
                      </div>

                      <div className="input-with-button">
                        <input
                          type="text"
                          className="form-input"
                          placeholder={(() => {
                            const cur = llmSettings.providers.find((p) => p.id === llmSettings.active_provider_id);
                            if (cur?.provider_type === "cloudflare") return "e.g. @cf/meta/llama-3.3-70b-instruct-fp8-fast";
                            if (cur?.name?.toLowerCase().includes("nvidia")) return "e.g. nvidia/nemotron-3.5-lightning-30b-a3b";
                            if (cur?.provider_type === "groq") return "e.g. llama-3.3-70b-versatile";
                            if (cur?.provider_type === "ollama") return "e.g. llama3.2";
                            return "e.g. model-name";
                          })()}
                          value={llmSettings.model}
                          onChange={(e) => handleSelectOrUpdateModel(e.target.value)}
                          list="available-models-list"
                        />
                        {availableModels.length > 0 && (
                          <datalist id="available-models-list">
                            {availableModels.map((m) => (
                              <option key={m} value={m} />
                            ))}
                          </datalist>
                        )}
                      </div>

                      {availableModels.length > 0 && (
                        <div style={{ marginTop: "6px" }}>
                          <select
                            className="form-input form-select"
                            value={availableModels.includes(llmSettings.model) ? llmSettings.model : ""}
                            onChange={(e) => {
                              if (e.target.value) {
                                handleSelectOrUpdateModel(e.target.value);
                              }
                            }}
                          >
                            <option value="" disabled>
                              -- Select from {availableModels.length} fetched models --
                            </option>
                            {availableModels.map((m) => (
                              <option key={m} value={m}>
                                {m}
                              </option>
                            ))}
                          </select>
                        </div>
                      )}

                      {(() => {
                        const cur = llmSettings.providers.find((p) => p.id === llmSettings.active_provider_id) || llmSettings.providers[0];
                        const isCf = cur?.provider_type === "cloudflare";

                        if (isCf) {
                          // The Cloudflare LLMs are fine as predefined quick chips
                          return (
                            <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "6px" }}>
                              {[
                                { id: "@cf/meta/llama-3.3-70b-instruct-fp8-fast", label: "Llama 3.3 70B (Quality)" },
                                { id: "@cf/meta/llama-3.1-8b-instruct-fp8", label: "Llama 3.1 8B (Fast)" },
                                { id: "@cf/deepseek-ai/deepseek-r1-distill-qwen-32b", label: "DeepSeek R1 32B" },
                                { id: "@cf/qwen/qwen2.5-coder-32b-instruct", label: "Qwen 2.5 Coder 32B" },
                                { id: "@cf/openai/gpt-oss-120b", label: "GPT-OSS 120B" },
                              ].map((item) => (
                                <button
                                  key={item.id}
                                  type="button"
                                  className={`provider-chip ${llmSettings.model === item.id ? "active" : ""}`}
                                  style={{ fontSize: "11px", padding: "3px 8px" }}
                                  onClick={() => handleSelectOrUpdateModel(item.id)}
                                >
                                  {item.label}
                                </button>
                              ))}
                            </div>
                          );
                        }

                        // Not predefined for other LLM providers: show previously used models for this provider
                        const previousModels = [
                          ...(cur?.recent_models || []),
                          ...(cur?.model && !(cur?.recent_models || []).includes(cur.model) ? [cur.model] : []),
                        ].filter((m) => m && m.trim().length > 0);

                        if (previousModels.length === 0) {
                          return null;
                        }

                        return (
                          <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "6px", alignItems: "center" }}>
                            <span style={{ fontSize: "11px", color: "var(--text-secondary)" }}>Previously used:</span>
                            {previousModels.map((m) => (
                              <button
                                key={m}
                                type="button"
                                className={`provider-chip ${llmSettings.model === m ? "active" : ""}`}
                                style={{ fontSize: "11px", padding: "3px 8px" }}
                                onClick={() => handleSelectOrUpdateModel(m)}
                                title={`Switch to model: ${m}`}
                              >
                                {m}
                              </button>
                            ))}
                          </div>
                        );
                      })()}
                    </div>

                    {/* Default Voice / Rectify Preset */}
                    <div className="form-group">
                      <label className="form-label">Default Voice / Rectification Preset</label>
                      <select
                        className="form-input form-select"
                        value={llmSettings.voice_preset}
                        onChange={(e) => updateAndSaveLlmSettings({ voice_preset: e.target.value })}
                      >
                        {(llmSettings.presets || DEFAULT_PRESETS).map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.label}
                          </option>
                        ))}
                        <option value="custom">🛠 Custom Instructions (Ad-hoc)</option>
                      </select>
                    </div>

                    {llmSettings.voice_preset === "custom" && (
                      <div className="form-group">
                        <label className="form-label">Ad-Hoc Custom Prompt Instructions</label>
                        <textarea
                          className="form-input"
                          rows={3}
                          placeholder="e.g. You are an editor. Rewrite the text into bullet points..."
                          value={llmSettings.custom_prompt}
                          onChange={(e) =>
                            setLlmSettings({ ...llmSettings, custom_prompt: e.target.value })
                          }
                          onBlur={() =>
                            updateAndSaveLlmSettings({ custom_prompt: llmSettings.custom_prompt })
                          }
                        />
                        <span className="form-hint">
                          Used whenever 'Custom Instructions' is selected in the workspace.
                        </span>
                      </div>
                    )}

                    {/* Auto Mode Setting */}
                    <div className="form-group">
                      <label className="checkbox-label">
                        <input
                          type="checkbox"
                          className="checkbox-input"
                          checked={llmSettings.auto_mode}
                          onChange={(e) => updateAndSaveLlmSettings({ auto_mode: e.target.checked })}
                        />
                        <span>Auto-transform immediately when recording stops</span>
                      </label>
                      <span className="form-hint" style={{ marginLeft: "26px" }}>
                        When enabled, audio is transcribed then immediately corrected by LLM and copied to clipboard. When disabled, you can manually review and edit raw text before clicking Transform.
                      </span>
                    </div>

                    {/* Request Timeout Setting */}
                    <div className="form-group" style={{ marginTop: "8px", paddingTop: "12px", borderTop: "1px solid var(--border)" }}>
                      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "8px" }}>
                        <label className="form-label" style={{ margin: 0 }}>Request Timeout (applies to both ASR &amp; LLM)</label>
                        <span style={{ fontSize: "12px", color: "var(--text-secondary)", fontWeight: 500 }}>
                          {Math.floor((llmSettings.request_timeout_secs || 180) / 60)}m {(llmSettings.request_timeout_secs || 180) % 60}s ({llmSettings.request_timeout_secs || 180}s)
                        </span>
                      </div>
                      <div style={{ display: "flex", alignItems: "center", gap: "10px", marginBottom: "8px" }}>
                        <input
                          type="number"
                          min={180}
                          step={30}
                          className="form-input"
                          style={{ width: "110px" }}
                          value={llmSettings.request_timeout_secs || 180}
                          onChange={(e) => {
                            const val = Math.max(180, parseInt(e.target.value, 10) || 180);
                            updateAndSaveLlmSettings({ request_timeout_secs: val });
                            updateAndSaveSettings({ request_timeout_secs: val });
                          }}
                        />
                        <span style={{ fontSize: "12px", color: "var(--text-secondary)" }}>
                          seconds (min 180s / 3 min)
                        </span>
                      </div>
                      <div className="timeout-pills">
                        {[
                          { label: "3 min (180s)", val: 180 },
                          { label: "5 min (300s)", val: 300 },
                          { label: "10 min (600s)", val: 600 },
                        ].map((opt) => (
                          <button
                            key={opt.val}
                            type="button"
                            className={`timeout-pill ${(llmSettings.request_timeout_secs || 180) === opt.val ? "active" : ""}`}
                            onClick={() => {
                              updateAndSaveLlmSettings({ request_timeout_secs: opt.val });
                              updateAndSaveSettings({ request_timeout_secs: opt.val });
                            }}
                          >
                            {opt.label}
                          </button>
                        ))}
                      </div>
                      <span className="form-hint" style={{ marginTop: "6px", display: "block" }}>
                        Custom timeout for network requests. Configured timeout applies to both speech transcription (ASR) and LLM rectification requests.
                      </span>
                    </div>
                  </div>
                )}
              </div>

              {/* Presets & Conversions Manager - Dedicated Card */}
              <div className="settings-section-card">
                <div className="settings-section-heading" style={{ justifyContent: "space-between" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                    <span>Voice Presets &amp; Conversion Instructions</span>
                    <span className="badge-pill installed" style={{ fontSize: "10px" }}>
                      {(llmSettings.presets || DEFAULT_PRESETS).length} Presets
                    </span>
                  </div>
                  <div style={{ display: "flex", gap: "6px" }}>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      style={{ fontSize: "11px", padding: "4px 8px" }}
                      onClick={async () => {
                        try {
                          await invoke("open_presets_folder");
                        } catch (err: any) {
                          setErrorMsg("Failed opening presets folder: " + String(err));
                        }
                      }}
                      title="Open presets directory in your file explorer to view or edit Markdown preset files directly"
                    >
                      📂 Open Folder
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      style={{ fontSize: "11px", padding: "4px 8px" }}
                      onClick={handleRestoreDefaultPresets}
                      title="Reset all presets back to built-in defaults"
                    >
                      🔄 Restore Defaults
                    </button>
                    <button
                      type="button"
                      className="btn btn-primary"
                      style={{ fontSize: "11px", padding: "4px 10px" }}
                      onClick={handleStartAddPreset}
                    >
                      ➕ Add Preset
                    </button>
                  </div>
                </div>

                <span className="form-hint" style={{ marginBottom: "12px", display: "block" }}>
                  Stored as open Markdown (<code>.md</code>) files with YAML frontmatter in <code>presets/</code>. A hidden <code>.guide.md</code> provides formatting instructions.
                </span>

                {/* Inline Formatted Add Preset Form */}
                {isAddingPreset && (
                  <div className="preset-form-card">
                    <div className="preset-form-header">
                      <div className="preset-form-title">
                        <span className="preset-form-icon">➕</span>
                        <div>
                          <h4>Create Custom Conversion Preset</h4>
                          <span className="form-hint">Saves directly into your local Markdown presets directory</span>
                        </div>
                      </div>
                      <button
                        type="button"
                        className="btn-icon"
                        onClick={handleCancelPresetForm}
                        title="Cancel"
                      >
                        ✕
                      </button>
                    </div>

                    <div className="preset-form-body">
                      <div className="preset-form-row">
                        <div className="form-group flex-1">
                          <label className="form-label">
                            Preset Name / Label <span className="required-mark">*</span>
                          </label>
                          <input
                            type="text"
                            className="form-input"
                            placeholder="e.g. 🇧🇩 Bengali Formal, 📝 Meeting Minutes, 📧 Executive Email"
                            value={presetForm.label}
                            onChange={(e) => setPresetForm({ ...presetForm, label: e.target.value })}
                            autoFocus
                          />
                        </div>

                        <div className="form-group preset-id-col">
                          <label className="form-label">File Slug</label>
                          <input
                            type="text"
                            className="form-input font-mono"
                            placeholder="auto-slug"
                            value={
                              presetForm.label
                                .toLowerCase()
                                .replace(/[^a-z0-9_-]/g, "_")
                                .replace(/_+/g, "_")
                                .replace(/^_|_$/g, "") || "custom_preset"
                            }
                            disabled
                            title="Generated filename: <slug>.md"
                          />
                        </div>
                      </div>

                      <div className="form-group">
                        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                          <label className="form-label">
                            System Instruction / Transformation Prompt <span className="required-mark">*</span>
                          </label>
                          <span className="form-hint" style={{ margin: 0 }}>
                            {presetForm.prompt.length} chars
                          </span>
                        </div>
                        <textarea
                          className="form-input preset-prompt-textarea"
                          rows={5}
                          placeholder="You are an expert copyeditor. Rewrite the transcribed speech text into clear, polished language while preserving all original facts and substance. Output ONLY the rewritten text without commentary, pleasantries, or introductory remarks."
                          value={presetForm.prompt}
                          onChange={(e) => setPresetForm({ ...presetForm, prompt: e.target.value })}
                        />
                        <div className="preset-prompt-hints">
                          <span>💡 Lipi passes the raw transcription to this prompt. The LLM response fills the Result pane.</span>
                        </div>
                      </div>
                    </div>

                    <div className="preset-form-footer">
                      <button type="button" className="btn btn-secondary" onClick={handleCancelPresetForm}>
                        Cancel
                      </button>
                      <button
                        type="button"
                        className="btn btn-primary"
                        onClick={handleSavePresetForm}
                        disabled={!presetForm.label.trim() || !presetForm.prompt.trim()}
                      >
                        💾 Save Preset
                      </button>
                    </div>
                  </div>
                )}

                {/* Presets List */}
                <div className="llm-presets-container">
                  {(llmSettings.presets || DEFAULT_PRESETS).map((p) => {
                    const isEditing = editingPresetId === p.id;
                    return (
                      <div key={p.id} className="llm-preset-card">
                        {isEditing ? (
                          <div className="preset-form-card" style={{ margin: 0 }}>
                            <div className="preset-form-header">
                              <div className="preset-form-title">
                                <span className="preset-form-icon">✏️</span>
                                <div>
                                  <h4>Edit Preset: {p.label}</h4>
                                  <span className="form-hint">File: presets/{p.id}.md</span>
                                </div>
                              </div>
                              <button
                                type="button"
                                className="btn-icon"
                                onClick={handleCancelPresetForm}
                                title="Cancel"
                              >
                                ✕
                              </button>
                            </div>

                            <div className="preset-form-body">
                              <div className="form-group">
                                <label className="form-label">
                                  Preset Name / Label <span className="required-mark">*</span>
                                </label>
                                <input
                                  type="text"
                                  className="form-input"
                                  value={presetForm.label}
                                  onChange={(e) => setPresetForm({ ...presetForm, label: e.target.value })}
                                />
                              </div>

                              <div className="form-group">
                                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                                  <label className="form-label">
                                    System Instruction / Transformation Prompt <span className="required-mark">*</span>
                                  </label>
                                  <span className="form-hint" style={{ margin: 0 }}>
                                    {presetForm.prompt.length} chars
                                  </span>
                                </div>
                                <textarea
                                  className="form-input preset-prompt-textarea"
                                  rows={5}
                                  value={presetForm.prompt}
                                  onChange={(e) => setPresetForm({ ...presetForm, prompt: e.target.value })}
                                />
                              </div>
                            </div>

                            <div className="preset-form-footer">
                              <button type="button" className="btn btn-secondary" onClick={handleCancelPresetForm}>
                                Cancel
                              </button>
                              <button
                                type="button"
                                className="btn btn-primary"
                                onClick={handleSavePresetForm}
                                disabled={!presetForm.label.trim() || !presetForm.prompt.trim()}
                              >
                                ✓ Update Preset
                              </button>
                            </div>
                          </div>
                        ) : (
                          <>
                            <div className="llm-preset-header">
                              <div className="llm-preset-title">
                                <span>{p.label}</span>
                                <span className="llm-preset-id-badge">{p.id}.md</span>
                                {llmSettings.voice_preset === p.id && (
                                  <span style={{ fontSize: "10px", color: "var(--accent)", fontWeight: 600 }}>
                                    ✓ Active in Workspace
                                  </span>
                                )}
                              </div>
                              <div className="llm-preset-actions">
                                {llmSettings.voice_preset !== p.id && (
                                  <button
                                    type="button"
                                    className="btn btn-secondary"
                                    style={{ fontSize: "11px", padding: "3px 8px" }}
                                    onClick={() => updateAndSaveLlmSettings({ voice_preset: p.id })}
                                    title="Set as active preset in workspace"
                                  >
                                    Use
                                  </button>
                                )}
                                <button
                                  type="button"
                                  className="btn btn-secondary"
                                  style={{ fontSize: "11px", padding: "3px 8px" }}
                                  onClick={() => handleStartEditPreset(p)}
                                  title="Edit this preset"
                                >
                                  ✏️ Edit
                                </button>
                                <button
                                  type="button"
                                  className="btn btn-danger"
                                  style={{ fontSize: "11px", padding: "3px 8px" }}
                                  onClick={() => handleDeletePreset(p.id)}
                                  title="Delete this preset"
                                >
                                  🗑️
                                </button>
                              </div>
                            </div>
                            <div className="llm-preset-prompt-preview">
                              {p.prompt}
                            </div>
                          </>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            </>
          )}

            {/* Section 3: General Audio & Transcription Preferences */}
            {settingsNavTab === "preferences" && (
              <div className="settings-section-card">
                <div className="settings-section-heading">
                  <span>Preferences</span>
                </div>

                <div className="form-group" style={{ marginBottom: "20px", padding: "14px 16px", background: "var(--bg-secondary)", borderRadius: "8px", border: "1px solid var(--border)" }}>
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "12px" }}>
                    <div>
                      <div style={{ fontWeight: 600, fontSize: "14px", color: "var(--text-primary)" }}>🚀 First-Time Setup Wizard</div>
                      <div className="form-hint" style={{ marginTop: "2px" }}>Inspect machine specs, check RAM recommendations, or reconfigure local/cloud modes.</div>
                    </div>
                    <button type="button" className="btn btn-secondary" onClick={() => setWizardOpen(true)}>
                      Launch Wizard
                    </button>
                  </div>
                </div>

                <div className="form-group">
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      className="checkbox-input"
                      checked={settings.auto_copy}
                      onChange={(e) =>
                        updateAndSaveSettings({ auto_copy: e.target.checked })
                      }
                    />
                    <span>Automatically copy transcription to clipboard</span>
                  </label>
                  <span className="form-hint" style={{ marginLeft: "26px" }}>
                    When unchecked, transcript only writes to Scratchpad.
                  </span>
                </div>

                <div className="form-group">
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      className="checkbox-input"
                      checked={settings.always_on_top}
                      onChange={(e) =>
                        updateAndSaveSettings({ always_on_top: e.target.checked })
                      }
                    />
                    <span>Keep window and wizard always on top</span>
                  </label>
                  <span className="form-hint" style={{ marginLeft: "26px" }}>
                    Prevents other windows from covering this app.
                  </span>
                </div>

                <div className="form-group">
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      className="checkbox-input"
                      checked={autostartEnabled}
                      onChange={(e) => handleToggleAutostart(e.target.checked)}
                    />
                    <span>Start Lipi on system startup</span>
                  </label>
                  <span className="form-hint" style={{ marginLeft: "26px" }}>
                    Automatically launch Lipi into a visible window when you log in.
                  </span>
                </div>

                <div className="form-group" style={{ marginTop: "18px", paddingTop: "14px", borderTop: "1px solid var(--border)" }}>
                  <label className="form-label" style={{ fontWeight: 600, display: "block", marginBottom: "8px" }}>
                    Mini Wizard Recording Behavior
                  </label>
                  <div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
                    <label className="radio-label" style={{ display: "flex", alignItems: "flex-start", gap: "10px", cursor: "pointer" }}>
                      <input
                        type="radio"
                        name="mini_record_mode"
                        value="new_note"
                        checked={settings.mini_record_mode !== "append"}
                        onChange={() => updateAndSaveSettings({ mini_record_mode: "new_note" })}
                        style={{ marginTop: "3px" }}
                      />
                      <div>
                        <span style={{ fontWeight: 500, color: "var(--text-primary)" }}>
                          Create a new note for each recording (Default)
                        </span>
                        <div className="form-hint" style={{ marginTop: "2px" }}>
                          Each recording finished in the Mini Wizard saves as an independent new note.
                        </div>
                      </div>
                    </label>

                    <label className="radio-label" style={{ display: "flex", alignItems: "flex-start", gap: "10px", cursor: "pointer" }}>
                      <input
                        type="radio"
                        name="mini_record_mode"
                        value="append"
                        checked={settings.mini_record_mode === "append"}
                        onChange={() => updateAndSaveSettings({ mini_record_mode: "append" })}
                        style={{ marginTop: "3px" }}
                      />
                      <div>
                        <span style={{ fontWeight: 500, color: "var(--text-primary)" }}>
                          Append to current active note
                        </span>
                        <div className="form-hint" style={{ marginTop: "2px" }}>
                          Appends transcribed speech onto the end of the currently selected note.
                        </div>
                      </div>
                    </label>
                  </div>
                  <span className="form-hint" style={{ display: "block", marginTop: "10px", fontStyle: "italic" }}>
                    Note: In full screen mode, recordings append to the active note by default, and you can create a fresh note anytime using the "New Note" button.
                  </span>
                </div>
              </div>
            )}



            {/* Section 4: Diagnostic & Activity Logs */}
            {settingsNavTab === "logs" && (
              <div className="settings-section-card">
                <div className="settings-section-heading" style={{ justifyContent: "space-between" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                    <span>Diagnostic & Activity Logs</span>
                    <span className="log-count-pill">
                      {logs.length} {logs.length === 1 ? "entry" : "entries"}
                    </span>
                  </div>
                  {logs.length > 0 && (
                    <div style={{ display: "flex", gap: "8px" }}>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={copyFullLogs}
                        title="Copy full diagnostic logs to clipboard"
                      >
                        📋 Copy All Logs
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={clearLogs}
                        title="Clear log history"
                      >
                        🗑 Clear
                      </button>
                    </div>
                  )}
                </div>

                {logs.length === 0 ? (
                  <div className="empty-logs-msg">
                    <span>No transcription errors or activity logs recorded yet. Recent events and server diagnostics will appear here.</span>
                  </div>
                ) : (
                  <div className="log-card">
                    <div className="log-list">
                      {logs.map((log) => {
                        const isExpanded = expandedLogId === log.id;
                        return (
                          <div key={log.id} className="log-item">
                            <div className="log-item-header">
                              <div className="log-item-left">
                                <span className={`log-badge ${log.level}`}>
                                  {log.level}
                                </span>
                                <span className="log-time">{log.timestamp}</span>
                                <span className="log-meta">
                                  {log.engine} • {log.model || "default"}
                                </span>
                              </div>
                              <button
                                type="button"
                                className="btn btn-secondary btn-sm"
                                style={{ padding: "2px 8px", fontSize: "11px" }}
                                onClick={() => setExpandedLogId(isExpanded ? null : log.id)}
                              >
                                {isExpanded ? "Hide ▲" : "Details ▼"}
                              </button>
                            </div>
                            <div className="log-message">{log.message}</div>
                            {isExpanded && log.details && (
                              <div className="log-details-box">
                                {log.details}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}
              </div>
            )}

            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: "8px" }}>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => handleSaveSettings()}
                style={{ padding: "8px 24px" }}
              >
                Done
              </button>
            </div>
          </div>
        </main>
      </div>
    ) : (
        <main className="main-view">
          {/* Navbar */}
          <header className={`navbar ${isStackedNav ? "compact-stacked" : ""}`}>
            <div className="navbar-left">
              <button
                type="button"
                className="btn-icon"
                onClick={() => setSidebarOpen(!sidebarOpen)}
                title="Toggle sidebar"
              >
                ☰
              </button>
              <span className="brand-name">Lipi</span>

              {isRecording && (
                <div className="badge-status badge-recording">
                  <span className="dot pulse"></span>
                  <span>Rec {formatTimer(recordSeconds)}</span>
                </div>
              )}
              {isTranscribing && (
                <div className="badge-status badge-transcribing">
                  <span className="dot pulse"></span>
                  <span>Transcribing...</span>
                </div>
              )}
              {!isRecording && !isTranscribing && (
                <div className="badge-status badge-idle">
                  <span className="dot"></span>
                  <span>Ready</span>
                </div>
              )}

              {/* Speech & LLM engine badges */}
              <div className="nav-engine-cluster">
                <div
                  className="badge-engine"
                  onClick={() => openSettings("asr")}
                  title={`ASR Engine: ${settings.engine_mode === "local" ? settings.local_engine : "API"} (${settings.model || settings.local_model_size}). Click to configure`}
                >
                  <span>
                    {settings.engine_mode === "local" ? "⚡" : "🌐"} {getDisplayModelName(settings)}
                  </span>
                </div>

                {llmSettings.enabled && (
                  <div
                    className="badge-engine badge-llm"
                    onClick={() => openSettings("llm")}
                    title={`LLM Preset: ${llmSettings.voice_preset.replace("_", " ")} (${llmSettings.auto_mode ? "Auto Transform ON" : "Manual"}). Click to configure`}
                    style={{ background: "rgba(99, 102, 241, 0.15)", borderColor: "rgba(99, 102, 241, 0.35)" }}
                  >
                    <span>
                      🤖 LLM{llmSettings.auto_mode ? " Auto" : ""}
                    </span>
                  </div>
                )}
              </div>
            </div>

            <div className="navbar-right">
              <button
                type="button"
                className="btn btn-secondary nav-btn-mini"
                onClick={() => toggleMiniMode(true)}
                title="Switch to compact floating wizard (Alt+M)"
              >
                <span>⊡</span>
                <span className="nav-btn-mini-label"> Mini Wizard</span>
              </button>

              {/* Stacked Quick Controls & Settings Dropdown */}
              <div className="nav-overflow-container" ref={navOverflowRef}>
                <button
                  type="button"
                  className={`btn-icon nav-overflow-trigger ${navOverflowOpen ? "active" : ""}`}
                  onClick={() => setNavOverflowOpen(!navOverflowOpen)}
                  title="Settings & Quick Controls"
                  aria-label="Settings & Quick Controls"
                >
                  ⚙
                  {(llmSettings.auto_mode || settings.auto_copy || settings.always_on_top) && (
                    <span className="nav-overflow-dot" />
                  )}
                </button>

                {navOverflowOpen && (
                  <div className="nav-overflow-menu">
                    <div className="nav-overflow-header">
                      <span>Quick Controls</span>
                      <button
                        type="button"
                        className="btn-icon"
                        style={{ padding: "2px 6px", fontSize: "11px" }}
                        onClick={() => setNavOverflowOpen(false)}
                        title="Close"
                      >
                        ✕
                      </button>
                    </div>

                    {llmSettings.enabled && (
                      <label className="nav-overflow-item">
                        <span className="nav-overflow-item-left">
                          <span>⚡</span>
                          <span>Auto Transform</span>
                        </span>
                        <input
                          type="checkbox"
                          checked={llmSettings.auto_mode}
                          onChange={(e) =>
                            updateAndSaveLlmSettings({ auto_mode: e.target.checked })
                          }
                        />
                        <span className={`mini-status-pill ${llmSettings.auto_mode ? "active" : ""}`}>
                          {llmSettings.auto_mode ? "ON" : "OFF"}
                        </span>
                      </label>
                    )}

                    <label className="nav-overflow-item">
                      <span className="nav-overflow-item-left">
                        <span>📋</span>
                        <span>Auto Copy</span>
                      </span>
                      <input
                        type="checkbox"
                        checked={settings.auto_copy}
                        onChange={(e) =>
                          updateAndSaveSettings({ auto_copy: e.target.checked })
                        }
                      />
                      <span className={`mini-status-pill ${settings.auto_copy ? "active" : ""}`}>
                        {settings.auto_copy ? "ON" : "OFF"}
                      </span>
                    </label>

                    <button
                      type="button"
                      className="nav-overflow-btn"
                      onClick={() => {
                        toggleAlwaysOnTop();
                      }}
                    >
                      <span className="nav-overflow-item-left">
                        <span>📌</span>
                        <span>Always on Top</span>
                      </span>
                      <span className={`mini-status-pill ${settings.always_on_top ? "active" : ""}`}>
                        {settings.always_on_top ? "ON" : "OFF"}
                      </span>
                    </button>



                    <div className="nav-overflow-divider" />

                    <button
                      type="button"
                      className="nav-overflow-btn nav-overflow-settings-btn"
                      onClick={() => {
                        setNavOverflowOpen(false);
                        openSettings("asr");
                      }}
                    >
                      <span className="nav-overflow-item-left">
                        <span>⚙</span>
                        <span>Settings</span>
                      </span>
                      <span className="nav-overflow-hint">Configure ↗</span>
                    </button>
                  </div>
                )}
              </div>
            </div>
          </header>

          {/* Error Notification */}
          {errorMsg && (
            <div className="toast-error">
              <div className="toast-error-header">
                <div className="toast-error-title-row">
                  <span>⚠️</span>
                  <span className="toast-error-title">Transcription Error</span>
                </div>
                <button className="btn-icon" onClick={() => setErrorMsg(null)} title="Dismiss">✕</button>
              </div>
              <div className="toast-error-message">{errorMsg}</div>
              <div className="toast-error-actions">
                <button
                  type="button"
                  className="btn-toast-action"
                  onClick={async () => {
                    await copyText(`Transcription Error Log:\n${errorMsg}\nTime: ${new Date().toISOString()}\nEngine: ${settings.engine_mode}\nModel: ${settings.model || "default"}\nEndpoint: ${settings.api_base_url || "(local)"}`);
                    showToast("✓ Error log copied!");
                  }}
                  title="Copy error details to clipboard"
                >
                  📋 Copy Error Details
                </button>
                <button
                  type="button"
                  className="btn-toast-action secondary"
                  onClick={() => {
                    setErrorMsg(null);
                    openSettings("logs");
                  }}
                  title="Open Settings to view diagnostic logs"
                >
                  🔍 View Logs in Settings
                </button>
              </div>
            </div>
          )}

          {/* Main Editor: Dual-Field when LLM enabled, Classic Scratchpad otherwise */}
          {llmSettings.enabled ? (
            <div className="dual-editor-container">
              {/* Middle Action Strip */}
              <div className="transform-strip">
                <div className="transform-controls-left">
                  <div className="voice-selector">
                    <label className="strip-label">Voice / Style:</label>
                    <select
                      className="form-select strip-select"
                      value={llmSettings.voice_preset}
                      onChange={(e) => {
                        updateAndSaveLlmSettings({ voice_preset: e.target.value });
                      }}
                      title={
                        (llmSettings.presets || DEFAULT_PRESETS).find(
                          (p) => p.id === llmSettings.voice_preset
                        )?.prompt || "Select conversion style"
                      }
                    >
                      {(llmSettings.presets || DEFAULT_PRESETS).map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                      <option value="custom">🛠 Custom Instructions</option>
                    </select>
                    <button
                      type="button"
                      className="preset-strip-btn"
                      onClick={async () => {
                        try {
                          await invoke("open_presets_folder");
                        } catch (err: any) {
                          setErrorMsg("Failed opening presets folder: " + String(err));
                        }
                      }}
                      title="Open presets directory in file manager to view or edit Markdown preset files"
                    >
                      📂
                    </button>
                    <button
                      type="button"
                      className="preset-strip-btn"
                      onClick={() => openSettings("llm")}
                      title="Manage, add, and customize presets in Settings"
                    >
                      <span>⚙️</span> <span className="btn-label-text">Presets</span>
                    </button>
                  </div>

                  {llmSettings.voice_preset === "custom" && (
                    <input
                      type="text"
                      className="form-input strip-custom-input"
                      placeholder="e.g. Rewrite as an executive memo, or translate to Bengali..."
                      value={llmSettings.custom_prompt}
                      onChange={(e) =>
                        setLlmSettings({ ...llmSettings, custom_prompt: e.target.value })
                      }
                      onBlur={() =>
                        updateAndSaveLlmSettings({ custom_prompt: llmSettings.custom_prompt })
                      }
                    />
                  )}
                </div>

                <div className="transform-controls-right">
                  <button
                    type="button"
                    className={`btn btn-transform ${isTransforming ? "loading" : ""}`}
                    onClick={() => handleTransform()}
                    disabled={isTransforming || !rawTranscript.trim()}
                    title="Transform raw transcript using selected LLM voice"
                  >
                    {isTransforming ? (
                      <>⏳ <span className="btn-transform-text">Transforming...</span></>
                    ) : (
                      <>✨ <span className="btn-transform-text">Transform with LLM</span></>
                    )}
                  </button>
                </div>
              </div>

              {/* Panel 1: Raw Transcription Buffer */}
              <div className="dual-editor-panel raw-panel">
                <div className="panel-header">
                  <div className="panel-title-group">
                    <span className="panel-badge raw">🎙 Raw Transcription</span>
                    <span className="panel-sub">Multi-clip buffer • Editable</span>
                  </div>
                  <div className="panel-actions">
                    {rawTranscript && (
                      <>
                        <button
                          type="button"
                          className="btn-panel-action"
                          onClick={() => copyText(rawTranscript)}
                          title="Copy raw text to clipboard"
                        >
                          📋 <span className="btn-panel-label">Copy</span>
                        </button>
                        <button
                          type="button"
                          className="btn-panel-action danger"
                          onClick={() => {
                            setRawTranscript("");
                            rawTranscriptRef.current = "";
                          }}
                          title="Clear raw transcript buffer"
                        >
                          🗑️ <span className="btn-panel-label">Clear</span>
                        </button>
                      </>
                    )}
                  </div>
                </div>
                <textarea
                  className="dual-textarea raw-textarea"
                  placeholder="Record speech [Alt+R] or type here. Multiple clips accumulate in this buffer for you to review and edit before transforming..."
                  value={rawTranscript}
                  onChange={(e) => handleContentChange(e.target.value)}
                  onBlur={handleBlurSave}
                />
                <div className="panel-footer">
                  <span>{rawTranscript.trim() ? rawTranscript.trim().split(/\s+/).length : 0} words</span>
                  <span>{rawTranscript.length} chars</span>
                </div>
              </div>

              {/* Panel 2: LLM Transformed Result */}
              <div className="dual-editor-panel result-panel">
                <div className="panel-header">
                  <div className="panel-title-group">
                    <span className="panel-badge result">✨ LLM Result</span>
                    <span className="panel-sub">
                      {llmSettings.voice_preset === "grammar_fix"
                        ? "Grammar Rectified"
                        : llmSettings.voice_preset === "professional"
                        ? "Professional Voice"
                        : llmSettings.voice_preset === "casual"
                        ? "Casual Voice"
                        : llmSettings.voice_preset === "concise"
                        ? "Concise Summary"
                        : llmSettings.voice_preset === "bullets"
                        ? "Action Bullets"
                        : "Custom Transformation"}
                    </span>
                  </div>
                  <div className="panel-actions">
                    {llmResult && (
                      <>
                        <button
                          type="button"
                          className="btn-panel-action primary"
                          onClick={() => copyText(llmResult)}
                          title="Copy result to clipboard"
                        >
                          📋 <span className="btn-panel-label">Copy Result</span>
                        </button>
                        <button
                          type="button"
                          className="btn-panel-action"
                          onClick={() => {
                            setRawTranscript(llmResult);
                            rawTranscriptRef.current = llmResult;
                            showToast("✓ Result applied to raw buffer");
                          }}
                          title="Send result back to raw buffer for multi-pass editing"
                        >
                          ⬆ <span className="btn-panel-label">Apply to Raw</span>
                        </button>
                        <button
                          type="button"
                          className="btn-panel-action"
                          onClick={() => persistNote(llmResult, activeNoteId)}
                          title="Save result as current note"
                        >
                          💾 <span className="btn-panel-label">Save Note</span>
                        </button>
                      </>
                    )}
                  </div>
                </div>
                <textarea
                  className="dual-textarea result-textarea"
                  placeholder={
                    isTransforming
                      ? "LLM is thinking and refining your speech..."
                      : "LLM-enhanced result will appear here. Click '✨ Transform' above or enable 'Auto Transform' to process automatically on recording stop..."
                  }
                  value={llmResult}
                  onChange={(e) => setLlmResult(e.target.value)}
                />
                <div className="panel-footer">
                  <span>{llmResult.trim() ? llmResult.trim().split(/\s+/).length : 0} words</span>
                  <span>{llmResult.length} chars</span>
                </div>
              </div>
            </div>
          ) : (
            <div className="editor-container">
              <textarea
                className="scratchpad-textarea"
                placeholder="Type here, or press 'Record' [Alt+R] to speak. Transcribed speech automatically appends and copies to clipboard..."
                value={content}
                onChange={(e) => handleContentChange(e.target.value)}
                onBlur={handleBlurSave}
              />
            </div>
          )}

          {/* Bottom Floating Control Dock */}
          <footer className="bottom-bar">
            <div className="bottom-left">
              <button
                className="btn btn-secondary"
                onClick={handleNewNote}
                title="Start a fresh note"
              >
                <span>+</span> <span className="btn-label-text">New Note</span>
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => copyText(content)}
                disabled={!content.trim()}
                title="Copy current note to clipboard"
              >
                <span>📋</span> <span className="btn-label-text">Copy Note</span>
              </button>
            </div>

            <div className="bottom-center">
              <button
                className={`btn btn-record ${isRecording ? "recording" : ""} ${
                  isTranscribing ? "transcribing" : ""
                }`}
                onClick={toggleRecording}
                disabled={isTranscribing}
                title="Shortcut: Alt+R"
              >
                {isRecording ? (
                  <>■ <span className="btn-record-text">Stop</span></>
                ) : isTranscribing ? (
                  <>⌛ <span className="btn-record-text">Processing...</span></>
                ) : (
                  <>● <span className="btn-record-text">Record</span> <span className="btn-record-hint">[Alt+R]</span></>
                )}
              </button>

              {isRecording && (
                <div className="record-elapsed-pill" title="Recording elapsed time">
                  <span className="record-elapsed-dot pulse" />
                  <span className="record-elapsed-time">{formatTimer(recordSeconds)}</span>
                </div>
              )}
            </div>

            <div className="bottom-right">
              <span className="stat-counter">
                {wordCount} {wordCount === 1 ? "word" : "words"} · {content.length} chars
                {isCloudflareActive && !settings.model?.includes("tiny") && (
                  <span
                    className="stat-counter-cf"
                    onClick={() => openSettings("providers")}
                    title={`Cloudflare Workers AI: Today consumed ${formatNeurons(cfUsage?.today?.total_neurons ?? 0)} Ⓝ (${formatCostUsd(cfUsage?.today?.total_cost_usd ?? 0)}) in Lipi. Click to view providers.`}
                    style={{ cursor: "pointer" }}
                  >
                    {" "}· {formatCostUsd(cfUsage?.today?.total_cost_usd ?? 0)}
                  </span>
                )}
              </span>
            </div>
          </footer>
        </main>
      )}

      {/* First-Time Setup Wizard Modal */}
      <SetupWizard
        isOpen={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onFinish={handleFinishWizard}
        currentSettings={settings}
        modelStatus={modelStatus}
        downloadProgress={downloadProgress}
        downloadSpeedBps={downloadSpeedBps}
        downloadEtaSecs={downloadEtaSecs}
        isDownloading={isDownloading}
        onDownloadModel={async (engine, size) => {
          await updateAndSaveSettings({ local_engine: engine, local_model_size: size });
          await handleDownloadModel();
        }}
        showToast={showToast}
      />
    </div>
  );
}
