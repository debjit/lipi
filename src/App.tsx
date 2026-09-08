import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  const [showSettings, setShowSettings] = useState(false);
  const [miniMode, setMiniMode] = useState(false);

  const [settings, setSettings] = useState<AppSettings>({
    api_base_url: "https://api.openai.com/v1",
    api_key: "",
    model: "whisper-1",
    language: "",
    auto_copy: true,
    always_on_top: true,
  });

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

  // Keyboard shortcuts (Alt+R to toggle record, Alt+M to toggle mini wizard)
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
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

  async function loadNotes() {
    try {
      const list: Note[] = await invoke("get_notes");
      setNotes(list);
    } catch (e) {
      console.error("Failed to load notes:", e);
    }
  }

  async function loadSettings() {
    try {
      const s: AppSettings = await invoke("get_settings");
      setSettings(s);
      await invoke("set_always_on_top", { alwaysOnTop: s.always_on_top });
    } catch (e) {
      console.error("Failed to load settings:", e);
    }
  }

  async function toggleAlwaysOnTop(val?: boolean) {
    const nextVal = val !== undefined ? val : !settings.always_on_top;
    const nextSettings = { ...settings, always_on_top: nextVal };
    setSettings(nextSettings);
    try {
      await invoke("save_settings", { settings: nextSettings });
      await invoke("set_always_on_top", { alwaysOnTop: nextVal });
    } catch (e) {
      console.error("Failed to toggle always on top:", e);
    }
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
          // 1. Copy to clipboard if option enabled
          if (settingsRef.current.auto_copy) {
            await navigator.clipboard.writeText(transcript);
            showToast("✓ Copied to clipboard!");
          } else {
            showToast("✓ Transcribed!");
          }

          // 2. Append to active note in scratchpad & persist to SQLite
          const current = activeContentRef.current;
          const nextContent = current ? `${current.trim()} ${transcript}` : transcript;
          setContent(nextContent);
          await persistNote(nextContent, activeIdRef.current);
        }
      } catch (err: any) {
        setErrorMsg(String(err));
      } finally {
        setIsTranscribing(false);
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
    try {
      await navigator.clipboard.writeText(text);
      if (noteId) {
        setCopiedId(noteId);
        setTimeout(() => setCopiedId(null), 1500);
      } else {
        showToast("✓ Note copied to clipboard!");
      }
    } catch (e) {
      console.error("Clipboard copy failed", e);
    }
  }

  async function handleSaveSettings(e: React.FormEvent) {
    e.preventDefault();
    try {
      await invoke("save_settings", { settings });
      setShowSettings(false);
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

      {/* Sidebar: History */}
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

      {/* Main Area */}
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
              onClick={() => setShowSettings(true)}
              title="API Settings"
            >
              ⚙
            </button>
          </div>
        </header>

        {/* Error Notification */}
        {errorMsg && (
          <div className="toast-error">
            <span>{errorMsg}</span>
            <button className="btn-icon" onClick={() => setErrorMsg(null)}>
              ✕
            </button>
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

      {/* Settings Modal */}
      {showSettings && (
        <div className="modal-overlay" onClick={() => setShowSettings(false)}>
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h3 className="modal-title">API Configuration</h3>
              <button className="btn-icon" onClick={() => setShowSettings(false)}>
                ✕
              </button>
            </div>

            <form onSubmit={handleSaveSettings}>
              <div className="form-group" style={{ marginBottom: "12px" }}>
                <label className="form-label">API Base URL</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="https://api.openai.com/v1"
                  value={settings.api_base_url}
                  onChange={(e) =>
                    setSettings({ ...settings, api_base_url: e.target.value })
                  }
                  required
                />
                <span className="form-hint">
                  OpenAI, Groq (https://api.groq.com/openai/v1), or local vLLM/Ollama.
                </span>
              </div>

              <div className="form-group" style={{ marginBottom: "12px" }}>
                <label className="form-label">API Key</label>
                <input
                  type="password"
                  className="form-input"
                  placeholder="sk-..."
                  value={settings.api_key}
                  onChange={(e) =>
                    setSettings({ ...settings, api_key: e.target.value })
                  }
                />
                <span className="form-hint">
                  Stored locally in SQLite. Never sent elsewhere.
                </span>
              </div>

              <div className="form-group" style={{ marginBottom: "12px" }}>
                <label className="form-label">Model Name</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="whisper-1"
                  value={settings.model}
                  onChange={(e) =>
                    setSettings({ ...settings, model: e.target.value })
                  }
                  required
                />
                <span className="form-hint">
                  whisper-1 (OpenAI) or whisper-large-v3 (Groq).
                </span>
              </div>

              <div className="form-group" style={{ marginBottom: "16px" }}>
                <label className="form-label">Language Code (Optional)</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="en (leave empty for auto-detect)"
                  value={settings.language || ""}
                  onChange={(e) =>
                    setSettings({ ...settings, language: e.target.value })
                  }
                />
              </div>

              <div className="form-group" style={{ marginBottom: "16px" }}>
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    className="checkbox-input"
                    checked={settings.auto_copy}
                    onChange={(e) =>
                      setSettings({ ...settings, auto_copy: e.target.checked })
                    }
                  />
                  <span>Automatically copy transcription to clipboard</span>
                </label>
                <span className="form-hint" style={{ marginLeft: "26px" }}>
                  When unchecked, transcript only writes to Scratchpad.
                </span>
              </div>

              <div className="form-group" style={{ marginBottom: "16px" }}>
                <label className="checkbox-label">
                  <input
                    type="checkbox"
                    className="checkbox-input"
                    checked={settings.always_on_top}
                    onChange={(e) => {
                      const checked = e.target.checked;
                      setSettings({ ...settings, always_on_top: checked });
                      invoke("set_always_on_top", { alwaysOnTop: checked }).catch(() => {});
                    }}
                  />
                  <span>Keep window and wizard always on top</span>
                </label>
                <span className="form-hint" style={{ marginLeft: "26px" }}>
                  Prevents other windows from covering this app.
                </span>
              </div>

              <div className="modal-footer">
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setShowSettings(false)}
                >
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Save Settings
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
