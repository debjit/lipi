import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

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

interface ProviderPreset {
  id: string;
  name: string;
  icon: string;
  url: string;
  defaultModel: string;
  keyPlaceholder: string;
  keyHint: string;
  urlHint: string;
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

const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "groq",
    name: "Groq",
    icon: "⚡",
    url: "https://api.groq.com/openai/v1",
    defaultModel: "whisper-large-v3-turbo",
    keyPlaceholder: "gsk_...",
    keyHint: "Groq Cloud API Key (from console.groq.com/keys).",
    urlHint: "Ultra-fast Whisper hosted on Groq LPU inference engine.",
  },
  {
    id: "cloudflare",
    name: "Cloudflare Workers AI",
    icon: "☁",
    url: "https://api.cloudflare.com/client/v4/accounts/<account_id>/ai/v1",
    defaultModel: "@cf/openai/whisper",
    keyPlaceholder: "Cloudflare API Token",
    keyHint: "Cloudflare API Token with Workers AI Read permissions.",
    urlHint: "Cloudflare Workers AI serverless endpoint.",
  },
  {
    id: "openai",
    name: "OpenAI",
    icon: "🤖",
    url: "https://api.openai.com/v1",
    defaultModel: "whisper-1",
    keyPlaceholder: "sk-...",
    keyHint: "OpenAI Secret API Key.",
    urlHint: "Official OpenAI audio transcription endpoint.",
  },
  {
    id: "local",
    name: "Local / Self-Hosted",
    icon: "🖥",
    url: "http://localhost:8000/v1",
    defaultModel: "whisper-1",
    keyPlaceholder: "(Optional for local servers)",
    keyHint: "Optional auth token if required by your local server.",
    urlHint: "Local vLLM, Ollama, Speaches, or whisper-asr server.",
  },
  {
    id: "custom",
    name: "Custom",
    icon: "⚙",
    url: "",
    defaultModel: "",
    keyPlaceholder: "Bearer token or API key",
    keyHint: "Authorization Bearer token.",
    urlHint: "Custom OpenAI-compatible transcription endpoint.",
  },
];

function getActivePreset(url: string): ProviderPreset {
  const trimmed = (url || "").trim();
  if (trimmed.includes("api.groq.com")) {
    return PROVIDER_PRESETS[0];
  }
  if (trimmed.includes("api.cloudflare.com")) {
    return PROVIDER_PRESETS[1];
  }
  if (trimmed.includes("api.openai.com")) {
    return PROVIDER_PRESETS[2];
  }
  if (trimmed.includes("localhost") || trimmed.includes("127.0.0.1") || trimmed.includes("0.0.0.0")) {
    return PROVIDER_PRESETS[3];
  }
  return PROVIDER_PRESETS[4];
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
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [activeView, setActiveView] = useState<"notes" | "settings">("notes");
  const [miniMode, setMiniMode] = useState(false);
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [memoryStatus, setMemoryStatus] = useState<ModelMemoryStatus | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<number | null>(null);
  const [isDownloading, setIsDownloading] = useState(false);
  const [isInstallingDeps, setIsInstallingDeps] = useState(false);
  const [isPreloading, setIsPreloading] = useState(false);
  const [isFreeingRam, setIsFreeingRam] = useState(false);

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
  });

  const activePreset = getActivePreset(settings.api_base_url);

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

  const [settingsNavTab, setSettingsNavTab] = useState<"asr" | "llm" | "preferences" | "logs">("asr");

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
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<{ percentage: number }>("model-download-progress", (event) => {
      setDownloadProgress(event.payload.percentage);
      if (event.payload.percentage >= 100) {
        setTimeout(() => {
          setDownloadProgress(null);
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

  async function loadSettings() {
    try {
      const s: AppSettings = await invoke("get_settings");
      setSettings(s);
      settingsRef.current = s;
      await invoke("set_always_on_top", { alwaysOnTop: s.always_on_top });
      await refreshModelStatus(s.local_engine, s.local_model_size);
      await refreshMemoryStatus();
    } catch (e) {
      console.error("Failed to load settings:", e);
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

  function openSettings(tab: "asr" | "llm" | "preferences" | "logs" = "asr") {
    setSettingsNavTab(tab);
    loadSettings();
    refreshModelStatus();
    refreshMemoryStatus();
    setActiveView("settings");
  }

  function closeSettings() {
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
        const transcript: string = await invoke("stop_recording_and_transcribe");
        if (transcript) {
          // 1. Always append to active note & persist to SQLite first
          const current = activeContentRef.current;
          const nextContent = current ? `${current.trim()} ${transcript}` : transcript;
          setContent(nextContent);
          await persistNote(nextContent, activeIdRef.current);

          // 2. Add success diagnostic log
          addLog({
            level: "success",
            title: "Transcription Successful",
            engine: settingsRef.current.engine_mode === "cloud" ? "Remote API" : `Local (${settingsRef.current.local_engine})`,
            model: settingsRef.current.engine_mode === "cloud" ? (settingsRef.current.model || "(default)") : settingsRef.current.local_model_size,
            endpoint: settingsRef.current.engine_mode === "cloud" ? settingsRef.current.api_base_url : undefined,
            message: `Transcribed ${transcript.trim().split(/\s+/).filter(Boolean).length} words.`,
            details: `Result: "${transcript.slice(0, 200)}${transcript.length > 200 ? "..." : ""}"`,
          });

          // 3. Copy to clipboard if option enabled
          if (settingsRef.current.auto_copy) {
            await copyText(transcript);
          } else {
            showToast("✓ Transcribed!");
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

  function handleContentChange(val: string) {
    setContent(val);
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
    setErrorMsg(null);
  }

  function handleSelectNote(note: Note) {
    setActiveNoteId(note.id);
    setContent(note.content);
    setErrorMsg(null);
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

        <button
          className={`btn-mini-float ${settings.always_on_top ? "pinned" : ""}`}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            toggleAlwaysOnTop();
          }}
          title={
            settings.always_on_top
              ? "Always on Top: ON (Click to unpin)"
              : "Always on Top: OFF (Click to pin)"
          }
        >
          📌
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
      )}

      {/* Main Content Area: Switch between Notes Scratchpad and Full-Page Settings */}
      {activeView === "settings" ? (
        <main className="settings-page">
          <header className="settings-page-header">
            <div className="settings-header-left">
              <button
                type="button"
                className="btn-back"
                onClick={closeSettings}
                title="Return to notes editor (Esc)"
              >
                ← Back to Notes <kbd className="key-hint">Esc</kbd>
              </button>
              <h2 className="settings-page-title">Settings</h2>
            </div>
            <div className="settings-header-right">
              {settings.engine_mode === "local" && (
                memoryStatus?.is_loaded ? (
                  <div className="memory-badge-loaded">
                    <span className="memory-pulse-dot"></span>
                    <span>RAM: ~{memoryStatus.estimated_ram_mb} MB</span>
                  </div>
                ) : (
                  <div className="memory-badge-cold">
                    <span>○ RAM: Inactive (0 MB)</span>
                  </div>
                )
              )}
            </div>
          </header>

          {/* Top-Level Settings Navigation Tabs */}
          <div className="settings-nav-tabs">
            <button
              type="button"
              className={`settings-nav-tab ${settingsNavTab === "asr" ? "active" : ""}`}
              onClick={() => setSettingsNavTab("asr")}
            >
              🎙 ASR
            </button>
            <button
              type="button"
              className={`settings-nav-tab ${settingsNavTab === "llm" ? "active" : ""}`}
              onClick={() => setSettingsNavTab("llm")}
            >
              🤖 LLM
              <span className="coming-soon-chip">Coming Soon</span>
            </button>
            <button
              type="button"
              className={`settings-nav-tab ${settingsNavTab === "preferences" ? "active" : ""}`}
              onClick={() => setSettingsNavTab("preferences")}
            >
              ⚙ Preferences
            </button>
            <button
              type="button"
              className={`settings-nav-tab ${settingsNavTab === "logs" ? "active" : ""}`}
              onClick={() => setSettingsNavTab("logs")}
            >
              📋 Diagnostic Logs
              {logs.filter((l) => l.level === "error").length > 0 && (
                <span className="tab-error-pill">
                  {logs.filter((l) => l.level === "error").length}
                </span>
              )}
            </button>
          </div>

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
                  <div className="form-group">
                    <label className="form-label">Provider Preset</label>
                    <div className="provider-presets">
                      {PROVIDER_PRESETS.map((preset) => {
                        const isActive = activePreset.id === preset.id;
                        return (
                          <button
                            key={preset.id}
                            type="button"
                            className={`provider-chip ${isActive ? "active" : ""}`}
                            onClick={() => {
                              if (preset.id === "custom") return;
                              let targetUrl = preset.url;
                              if (preset.id === "cloudflare") {
                                const match = settings.api_base_url.match(/accounts\/([a-zA-Z0-9_-]+)\/ai/);
                                if (match && match[1] && match[1] !== "<account_id>") {
                                  targetUrl = `https://api.cloudflare.com/client/v4/accounts/${match[1]}/ai/v1`;
                                }
                              }
                              const updated = {
                                api_base_url: targetUrl,
                                model: preset.defaultModel,
                              };
                              setSettings((prev) => ({ ...prev, ...updated }));
                              updateAndSaveSettings(updated);
                            }}
                          >
                            <span className="provider-icon">{preset.icon}</span>
                            <span className="provider-name">{preset.name}</span>
                          </button>
                        );
                      })}
                    </div>
                  </div>

                  {activePreset.id === "cloudflare" && (
                    <div className={`preset-alert ${settings.api_base_url.includes("<account_id>") || settings.api_base_url.includes("{account_id}") ? "warning" : "info"}`}>
                      {settings.api_base_url.includes("<account_id>") || settings.api_base_url.includes("{account_id}") ? (
                        <>
                          ⚠️ <strong>Action Needed:</strong> Replace <code>&lt;account_id&gt;</code> in the URL below with your actual Cloudflare Account ID (found in Cloudflare Dashboard &rarr; Workers &amp; Pages).
                        </>
                      ) : (
                        <>
                          ℹ️ <strong>Cloudflare Workers AI:</strong> Audio will be dispatched directly to <code>{settings.model || "@cf/openai/whisper"}</code> (supports standard binary and large-v3-turbo). Make sure your API Token has <em>Workers AI: Read</em> permissions.
                        </>
                      )}
                    </div>
                  )}

                  <div className="form-group">
                    <label className="form-label">API Endpoint URL</label>
                    <input
                      type="text"
                      className="form-input"
                      placeholder="https://api.openai.com/v1, https://api.groq.com/openai/v1, or http://localhost:8000/v1"
                      value={settings.api_base_url}
                      onChange={(e) =>
                        setSettings({ ...settings, api_base_url: e.target.value })
                      }
                      onBlur={() => updateAndSaveSettings({ api_base_url: settings.api_base_url })}
                      required
                    />
                    <span className="form-hint">
                      {activePreset.urlHint}
                    </span>
                  </div>

                  <div className="form-group">
                    <label className="form-label">API Key / Token {activePreset.id === "local" ? "(Optional)" : ""}</label>
                    <input
                      type="password"
                      className="form-input"
                      placeholder={activePreset.keyPlaceholder}
                      value={settings.api_key}
                      onChange={(e) =>
                        setSettings({ ...settings, api_key: e.target.value })
                      }
                      onBlur={() => updateAndSaveSettings({ api_key: settings.api_key })}
                    />
                    <span className="form-hint">
                      {activePreset.keyHint} Stored securely in local SQLite.
                    </span>
                  </div>

                  <div className="form-group">
                    <label className="form-label">Model Name</label>
                    <input
                      type="text"
                      className="form-input"
                      placeholder={activePreset.defaultModel || "whisper-1"}
                      value={settings.model}
                      onChange={(e) =>
                        setSettings({ ...settings, model: e.target.value })
                      }
                      onBlur={() => updateAndSaveSettings({ model: settings.model })}
                    />
                    {activePreset.id === "cloudflare" && (
                      <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "6px" }}>
                        {[
                          { id: "@cf/openai/whisper", label: "whisper (Standard)" },
                          { id: "@cf/openai/whisper-large-v3-turbo", label: "large-v3-turbo (Accurate)" },
                          { id: "@cf/openai/whisper-tiny-en", label: "tiny-en (Fast)" },
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
                    )}
                    <span className="form-hint">
                      {activePreset.id === "groq"
                        ? "Default: 'whisper-large-v3-turbo' (or 'whisper-large-v3')."
                        : activePreset.id === "cloudflare"
                        ? "Supports: '@cf/openai/whisper', '@cf/openai/whisper-large-v3-turbo', '@cf/openai/whisper-tiny-en'."
                        : "e.g. 'whisper-1' (OpenAI / local standard)."}
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
              </>
            )}

            {/* Section: LLM Post-Processing (Preview) */}
            {settingsNavTab === "llm" && (
              <div className="settings-section-card">
                <div className="settings-section-heading">
                  <span>LLM Post-Processing</span>
                  <span className="coming-soon-chip">In Development</span>
                </div>
                <div className="llm-preview-card">
                  <div className="llm-preview-header">
                    <span className="llm-preview-icon">🤖</span>
                    <div>
                      <h4 style={{ margin: "0 0 4px 0", fontSize: "14px", color: "var(--text-primary)" }}>
                        AI Speech Enhancement &amp; Transformations
                      </h4>
                      <p style={{ margin: 0, fontSize: "12px", color: "var(--text-secondary)", lineHeight: 1.4 }}>
                        Lipi will soon support chaining transcribed text with Large Language Models (LLMs) to automatically format, clean, synthesize, and translate your voice notes.
                      </p>
                    </div>
                  </div>

                  <div className="llm-features-grid">
                    <div className="llm-feature-item">
                      <span className="llm-feature-bullet">✨</span>
                      <div>
                        <strong>Punctuation &amp; Grammar Polish</strong>
                        <p>Turn raw spoken streams into structured paragraphs with proper casing and punctuation.</p>
                      </div>
                    </div>

                    <div className="llm-feature-item">
                      <span className="llm-feature-bullet">📝</span>
                      <div>
                        <strong>Action Items &amp; Summaries</strong>
                        <p>Automatically extract key action items, tasks, and concise bullet summaries from recordings.</p>
                      </div>
                    </div>

                    <div className="llm-feature-item">
                      <span className="llm-feature-bullet">🌐</span>
                      <div>
                        <strong>Multilingual Translation &amp; Tone</strong>
                        <p>Translate spoken speech into other languages or rephrase drafts into professional email tone.</p>
                      </div>
                    </div>

                    <div className="llm-feature-item">
                      <span className="llm-feature-bullet">⚡</span>
                      <div>
                        <strong>Local &amp; Cloud LLM Providers</strong>
                        <p>Configurable with local models (Ollama, llama.cpp) and cloud APIs (Groq, Cloudflare, OpenAI).</p>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* Section 3: General Audio & Transcription Preferences */}
            {settingsNavTab === "preferences" && (
              <div className="settings-section-card">
                <div className="settings-section-heading">
                  <span>Preferences</span>
                </div>

                <div className="form-group">
                  <label className="form-label">Language Code (Optional)</label>
                  <input
                    type="text"
                    className="form-input"
                    placeholder="en (leave empty for auto-detect)"
                    value={settings.language || ""}
                    onChange={(e) =>
                      setSettings({ ...settings, language: e.target.value })
                    }
                    onBlur={() => updateAndSaveSettings({ language: settings.language })}
                  />
                  <span className="form-hint">
                    e.g., 'en' for English, 'es' for Spanish, 'hi' for Hindi, 'bn' for Bengali, or leave blank for automatic detection.
                  </span>
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
      ) : (
        <main className="main-view">
          {/* Navbar */}
          <header className="navbar">
            <div className="navbar-left">
              <button
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
                  <span>Recording {formatTimer(recordSeconds)}</span>
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

              <div
                className="badge-engine"
                onClick={() => openSettings("asr")}
                title="Click to configure ASR speech engine in Settings"
              >
                {settings.engine_mode === "local" ? (
                  <span>
                    ⚡ {settings.local_engine === "whisper_cpu"
                      ? "Whisper CPU"
                      : settings.local_engine === "faster_whisper"
                      ? "Faster-Whisper"
                      : "Vulkan GPU"}{" "}
                    ({settings.local_model_size})
                  </span>
                ) : (
                  <span>🌐 API ({settings.model || "OpenAI-Compatible"})</span>
                )}
              </div>
            </div>

            <div className="navbar-right">
              <button
                className={`btn-icon ${settings.always_on_top ? "pinned" : ""}`}
                onClick={() => toggleAlwaysOnTop()}
                title={
                  settings.always_on_top
                    ? "Always on Top: ON (Click to unpin)"
                    : "Always on Top: OFF (Click to pin)"
                }
              >
                📌
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => toggleMiniMode(true)}
                title="Switch to compact floating wizard (Alt+M)"
                style={{ fontSize: "12px", padding: "5px 10px" }}
              >
                ⊡ Mini Wizard
              </button>
              <button
                className="btn-icon"
                onClick={() => openSettings("asr")}
                title="Settings & Audio Engine"
              >
                ⚙
              </button>
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

          {/* Scratchpad Text Area */}
          <div className="editor-container">
            <textarea
              className="scratchpad-textarea"
              placeholder="Type here, or press 'Record' [Alt+R] to speak. Transcribed speech automatically appends and copies to clipboard..."
              value={content}
              onChange={(e) => handleContentChange(e.target.value)}
              onBlur={handleBlurSave}
            />
          </div>

          {/* Bottom Floating Control Dock */}
          <footer className="bottom-bar">
            <div className="bottom-left">
              <button
                className="btn btn-secondary"
                onClick={handleNewNote}
                title="Start a fresh note"
              >
                + New Note
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => copyText(content)}
                disabled={!content.trim()}
                title="Copy current note to clipboard"
              >
                📋 Copy Note
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
                {isRecording
                  ? `■ Stop (${formatTimer(recordSeconds)})`
                  : isTranscribing
                  ? "⌛ Processing..."
                  : "● Record [Alt+R]"}
              </button>
            </div>

            <div className="bottom-right">
              <span className="stat-counter">
                {wordCount} {wordCount === 1 ? "word" : "words"} · {content.length} chars
              </span>
            </div>
          </footer>
        </main>
      )}
    </div>
  );
}
