import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface MeetingSession {
  id: number;
  title: string;
  language: string;
  started_at: string;
  started_epoch: number;
  ended_at?: string | null;
  audio_path: string;
  status: string;
  summary: string;
  decisions: string;
  note_id?: number | null;
  style_id?: string;
  style_prefix?: string;
}

interface MeetingSegment {
  id: number;
  session_id: number;
  start_ms: number;
  end_ms: number;
  text: string;
}

interface ActionItem {
  id: number;
  session_id: number;
  text: string;
  due_at?: string | null;
  due_epoch?: number | null;
  done: boolean;
  reminded: boolean;
}

interface MeetingDetail {
  session: MeetingSession;
  segments: MeetingSegment[];
  actions: ActionItem[];
}

interface SegmentEvent {
  session_id: number;
  start_ms: number;
  end_ms: number;
  text: string;
}

interface AudioCapture {
  id: number;
  name: string;
  bytes: number;
  error: string;
  created_at: string;
  session_id?: number | null;
  start_ms?: number | null;
}

interface PromptChoice {
  id: string;
  label: string;
  prompt: string;
}

interface CaptureSources {
  microphone: boolean;
  playback: boolean;
  note: string;
}

type MeetingScreen = "new" | "recording" | "review";

const MEETING_STYLE_KEY = "lipi.meeting.style";
const MEETING_PREFIX_KEY = "lipi.meeting.prefix";
const MEETING_CAPTURE_KEY = "lipi.meeting.capture";

export function Meetings({
  onBack,
  onOpenNote,
  onOpenPreferences,
}: {
  onBack: () => void;
  onOpenNote: (id: number) => void;
  onOpenPreferences: () => void;
}) {
  const [sessions, setSessions] = useState<MeetingSession[]>([]);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [screen, setScreen] = useState<MeetingScreen>("new");
  const [phase, setPhase] = useState<"idle" | "starting" | "recording" | "saving">("idle");
  const [chunkBusy, setChunkBusy] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [styles, setStyles] = useState<PromptChoice[]>([]);
  const [styleId, setStyleId] = useState(() => localStorage.getItem(MEETING_STYLE_KEY) || "notes");
  const [stylePrefix, setStylePrefix] = useState(() => localStorage.getItem(MEETING_PREFIX_KEY) || "");
  const [failed, setFailed] = useState<AudioCapture[]>([]);
  const [retryingId, setRetryingId] = useState<number | null>(null);
  const [sources, setSources] = useState<CaptureSources | null>(() => {
    const saved = sessionStorage.getItem(MEETING_CAPTURE_KEY);
    if (!saved) return null;
    try {
      return JSON.parse(saved) as CaptureSources;
    } catch {
      return null;
    }
  });
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const screenRef = useRef(screen);
  screenRef.current = screen;
  const activeIdRef = useRef<number | null>(null);
  const recordingIdRef = useRef<number | null>(null);
  const stopRef = useRef<(id: number) => void>(() => {});

  async function loadFailed(sessionId: number) {
    try {
      const rows = await invoke<AudioCapture[]>("list_session_captures", { sessionId });
      setFailed(rows);
    } catch {
      setFailed([]);
    }
  }

  async function showSession(id: number, asRecording: boolean) {
    const next = await invoke<MeetingDetail>("get_meeting", { sessionId: id });
    activeIdRef.current = next.session.id;
    setDetail(next);
    if (asRecording || next.session.status === "recording") {
      recordingIdRef.current = next.session.id;
      setScreen("recording");
      setPhase("recording");
    } else {
      setScreen("review");
      setPhase("idle");
    }
    await loadFailed(next.session.id);
  }

  async function refreshList() {
    const list = await invoke<MeetingSession[]>("list_meetings");
    setSessions(list);
    const live = list.find((session) => session.status === "recording");
    if (live) {
      await showSession(live.id, true);
      return;
    }
    recordingIdRef.current = null;
    if (screenRef.current === "review" && activeIdRef.current && list.some((session) => session.id === activeIdRef.current)) {
      await showSession(activeIdRef.current, false);
      return;
    }
    activeIdRef.current = null;
    setDetail(null);
    setFailed([]);
    setSources(null);
    sessionStorage.removeItem(MEETING_CAPTURE_KEY);
    setScreen("new");
    setPhase("idle");
  }

  useEffect(() => {
    invoke<PromptChoice[]>("list_meeting_styles")
      .then((rows) => {
        setStyles(rows);
        setStyleId((current) => {
          if (current === "custom" || rows.some((style) => style.id === current)) return current;
          return rows[0]?.id || "notes";
        });
      })
      .catch(() => {});
    refreshList().catch((err) => setError(String(err)));
    const unsubs: Array<() => void> = [];
    let cancelled = false;
    const track = <T,>(event: string, handler: (payload: T) => void) => {
      listen<T>(event, (e) => handler(e.payload)).then((unlisten) => {
        if (cancelled) unlisten();
        else unsubs.push(unlisten);
      });
    };
    track("meeting_segment", (event: SegmentEvent) => {
      if (event.session_id !== activeIdRef.current) return;
      setDetail((current) => {
        if (!current || current.session.id !== event.session_id) return current;
        if (current.segments.some((seg) => seg.start_ms === event.start_ms && seg.text === event.text)) {
          return current;
        }
        return {
          ...current,
          segments: [
            ...current.segments,
            {
              id: Date.now(),
              session_id: event.session_id,
              start_ms: event.start_ms,
              end_ms: event.end_ms,
              text: event.text,
            },
          ],
        };
      });
    });
    track("meeting_transcribing", (busy: boolean) => setChunkBusy(busy));
    track("meeting_error", (message: string) => {
      setError(message);
      const id = recordingIdRef.current ?? activeIdRef.current;
      if (id) void loadFailed(id);
    });
    track("meeting_capture", (sources: CaptureSources) => {
      sessionStorage.setItem(MEETING_CAPTURE_KEY, JSON.stringify(sources));
      setSources(sources);
    });
    track("meeting_limit", () => {
      const id = recordingIdRef.current;
      if (id && phaseRef.current === "recording") stopRef.current(id);
    });
    return () => {
      cancelled = true;
      unsubs.forEach((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (screen !== "recording" || !detail) return;
    const tick = () => setElapsed(Math.max(0, Math.floor(Date.now() / 1000) - detail.session.started_epoch));
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [screen, detail?.session.id, detail?.session.started_epoch]);

  useEffect(() => {
    const node = transcriptRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [detail?.segments.length]);

  function showNew() {
    if (recordingIdRef.current) return;
    setError(null);
    setConfirmDelete(false);
    setFailed([]);
    activeIdRef.current = null;
    setDetail(null);
    setScreen("new");
    setPhase("idle");
  }

  async function openSession(id: number) {
    if (recordingIdRef.current && recordingIdRef.current !== id) return;
    setError(null);
    setConfirmDelete(false);
    try {
      await showSession(id, recordingIdRef.current === id);
    } catch (err) {
      setError(String(err));
    }
  }

  async function start() {
    setConsentOpen(false);
    setError(null);
    setSources(null);
    sessionStorage.removeItem(MEETING_CAPTURE_KEY);
    setPhase("starting");
    try {
      localStorage.setItem(MEETING_STYLE_KEY, styleId);
      localStorage.setItem(MEETING_PREFIX_KEY, stylePrefix);
      const session = await invoke<MeetingSession>("start_meeting", {
        styleId,
        stylePrefix: styleId === "custom" ? stylePrefix : "",
      });
      activeIdRef.current = session.id;
      recordingIdRef.current = session.id;
      setDetail({ session, segments: [], actions: [] });
      setFailed([]);
      setScreen("recording");
      setPhase("recording");
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
    } catch (err) {
      setPhase("idle");
      setScreen("new");
      setError(String(err));
    }
  }

  async function stop(id: number) {
    if (phaseRef.current === "saving") return;
    const stoppingLive = recordingIdRef.current === id;
    if (stoppingLive) setPhase("saving");
    setError(null);
    try {
      const next = await invoke<MeetingDetail>("stop_meeting", { sessionId: id });
      if (stoppingLive) {
        recordingIdRef.current = null;
        setChunkBusy(false);
        setSources(null);
        sessionStorage.removeItem(MEETING_CAPTURE_KEY);
        setScreen("review");
        setPhase("idle");
      }
      if (activeIdRef.current === id || stoppingLive) {
        activeIdRef.current = id;
        setDetail(next);
        await loadFailed(id);
      }
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
    } catch (err) {
      setPhase(recordingIdRef.current ? "recording" : "idle");
      setError(String(err));
    }
  }
  stopRef.current = (id) => {
    void stop(id);
  };

  async function cancel(id: number) {
    setError(null);
    try {
      await invoke("cancel_meeting", { sessionId: id });
      recordingIdRef.current = null;
      activeIdRef.current = null;
      setDetail(null);
      setFailed([]);
      setSources(null);
      sessionStorage.removeItem(MEETING_CAPTURE_KEY);
      setChunkBusy(false);
      setScreen("new");
      setPhase("idle");
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
    } catch (err) {
      setError(String(err));
    }
  }

  async function toggleAction(item: ActionItem) {
    try {
      await invoke("set_action_item_done", { id: item.id, done: !item.done });
      if (detail) {
        const next = await invoke<MeetingDetail>("get_meeting", { sessionId: detail.session.id });
        setDetail(next);
      }
    } catch (err) {
      setError(String(err));
    }
  }

  async function remove(id: number) {
    try {
      await invoke("delete_meeting", { sessionId: id });
      setConfirmDelete(false);
      activeIdRef.current = null;
      setDetail(null);
      setFailed([]);
      setScreen("new");
      setPhase("idle");
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
    } catch (err) {
      setError(String(err));
    }
  }

  async function retryChunk(id: number) {
    if (!detail || retryingId != null) return;
    setRetryingId(id);
    setError(null);
    try {
      await invoke("retry_audio_capture", { id });
      const next = await invoke<MeetingDetail>("get_meeting", { sessionId: detail.session.id });
      setDetail(next);
      await loadFailed(detail.session.id);
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
    } catch (err) {
      setError(String(err));
      await loadFailed(detail.session.id);
    } finally {
      setRetryingId(null);
    }
  }

  async function discardChunk(id: number) {
    if (!detail) return;
    try {
      await invoke("delete_audio_capture", { id });
      await loadFailed(detail.session.id);
    } catch (err) {
      setError(String(err));
    }
  }

  const busy = phase === "starting" || phase === "saving";
  const decisions = parseDecisions(detail?.session.decisions || "[]");
  const canSaveInterrupted = screen === "review" && detail?.session.status === "interrupted";
  const styleTitle = styles.find((style) => style.id === styleId)?.prompt || "How the meeting note is written";

  return (
    <div className="meetings-page">
      <header className="meetings-header">
        <button type="button" className="btn-back" onClick={onBack}>
          ← Notes
        </button>
        <h2>Meetings</h2>
        {screen !== "recording" && (
          <button type="button" className="btn btn-secondary" onClick={showNew}>
            New meeting
          </button>
        )}
        <span className="meetings-header-note">Audio and notes stay on this computer.</span>
      </header>

      <div className="meetings-body">
        <aside className="meetings-list">
          {sessions.length === 0 ? (
            <div className="history-empty">No meetings yet.</div>
          ) : (
            sessions.map((session) => (
              <button
                key={session.id}
                type="button"
                className={`meetings-list-item ${screen !== "new" && detail?.session.id === session.id ? "active" : ""}`}
                onClick={() => openSession(session.id)}
              >
                <span className="history-item-title">{session.title || "Meeting"}</span>
                <span className="history-item-time">{session.started_at}</span>
                <span className="meetings-status">{session.status}</span>
              </button>
            ))
          )}
        </aside>

        <section className="meetings-detail">
          {error && <div className="toast-error-message meetings-error">{error}</div>}

          {screen === "new" && (
            <div className="meetings-setup">
              <h3>New meeting</h3>
              <p className="form-hint">
                Note styles are stored separately from dictation voice presets. This choice stays with the meeting.
              </p>
              <label>
                Note style
                <select
                  className="form-select"
                  value={styleId}
                  onChange={(e) => setStyleId(e.target.value)}
                  title={styleTitle}
                >
                  {styles.map((style) => (
                    <option key={style.id} value={style.id}>
                      {style.label}
                    </option>
                  ))}
                  <option value="custom">Custom prefix</option>
                </select>
              </label>
              {styleId === "custom" && (
                <label>
                  Style prefix
                  <textarea
                    className="form-input meetings-custom-prompt"
                    value={stylePrefix}
                    onChange={(e) => setStylePrefix(e.target.value)}
                    placeholder="Add instructions for how this meeting note should read..."
                  />
                </label>
              )}
              <div className="meetings-controls">
                <button
                  type="button"
                  className="btn btn-record"
                  onClick={() => setConsentOpen(true)}
                  disabled={busy}
                >
                  {phase === "starting" ? "Starting…" : "● Start recording"}
                </button>
                <span className="form-hint">Up to 2 hours. Dictation with Alt+R is unchanged.</span>
              </div>
            </div>
          )}

          {screen === "recording" && (
            <div className="meetings-controls">
              <button
                type="button"
                className="btn btn-record recording"
                onClick={() => {
                  const id = recordingIdRef.current;
                  if (id) void stop(id);
                }}
                disabled={busy}
              >
                {phase === "saving" ? "Writing note…" : "■ Stop"}
              </button>
              <span className="record-elapsed-time">{formatClock(elapsed)}</span>
              {chunkBusy && <span className="live-wait" title="Transcribing the last pause" />}
              <button
                type="button"
                className="btn btn-discard"
                onClick={() => {
                  const id = recordingIdRef.current;
                  if (id) void cancel(id);
                }}
                disabled={busy}
              >
                Cancel
              </button>
            </div>
          )}

          {screen === "recording" && sources && <p className="form-hint">{sources.note}</p>}

          {canSaveInterrupted && detail && (
            <div className="meetings-controls">
              <button type="button" className="btn btn-secondary" onClick={() => stop(detail.session.id)} disabled={busy}>
                {phase === "saving" ? "Writing note…" : "Save note"}
              </button>
            </div>
          )}

          {failed.length > 0 && screen !== "new" && (
            <div className="meetings-failed">
              <h4>Saved audio</h4>
              <p className="form-hint">These pieces failed. Switch provider in Settings, then retry them into this meeting.</p>
              {failed.map((item) => (
                <div key={item.id} className="meetings-failed-item">
                  <div className="saved-audio-meta">
                    <span className="saved-audio-name">{formatMs(item.start_ms || 0)}</span>
                    <span className="saved-audio-size">{formatBytes(item.bytes)}</span>
                  </div>
                  {item.error && <div className="saved-audio-error" title={item.error}>{item.error}</div>}
                  <div className="saved-audio-actions">
                    <button
                      type="button"
                      className="btn btn-secondary btn-compact"
                      onClick={() => retryChunk(item.id)}
                      disabled={retryingId != null || busy}
                    >
                      {retryingId === item.id ? "Retrying…" : "Retry"}
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary btn-compact"
                      onClick={() => discardChunk(item.id)}
                      disabled={retryingId != null}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}

          {screen !== "new" && detail && (
            <>
              <div className="meetings-summary">
                <h3>{detail.session.title || "Meeting"}</h3>
                {screen === "review" && detail.session.summary && <p>{detail.session.summary}</p>}
                {screen === "review" && decisions.length > 0 && (
                  <>
                    <h4>Decisions</h4>
                    <ul>
                      {decisions.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                  </>
                )}
                {screen === "review" && detail.actions.length > 0 && (
                  <>
                    <h4>Action items</h4>
                    <ul className="meetings-actions">
                      {detail.actions.map((item) => (
                        <li key={item.id}>
                          <label>
                            <input type="checkbox" checked={item.done} onChange={() => toggleAction(item)} />
                            <span>{item.text}</span>
                            {item.due_at && <span className="meetings-due">due {item.due_at}</span>}
                          </label>
                        </li>
                      ))}
                    </ul>
                    <p className="form-hint">
                      Reminders show while Lipi is open.{" "}
                      <button type="button" className="link-button" onClick={onOpenPreferences}>
                        Start Lipi at login
                      </button>{" "}
                      if you want them after a restart.
                    </p>
                  </>
                )}
                {screen === "review" && (
                  <div className="meetings-detail-actions">
                    {detail.session.note_id != null && (
                      <button type="button" className="btn btn-secondary" onClick={() => onOpenNote(detail.session.note_id as number)}>
                        Open note
                      </button>
                    )}
                    <button type="button" className="btn btn-secondary" onClick={() => setConfirmDelete(true)}>
                      Delete meeting
                    </button>
                  </div>
                )}
              </div>

              <div className="meetings-transcript" ref={transcriptRef}>
                {detail.segments.length === 0 ? (
                  <div className="history-empty">The transcript shows up here as you talk.</div>
                ) : (
                  detail.segments.map((seg) => (
                    <p key={`${seg.id}-${seg.start_ms}`}>
                      <span className="meetings-stamp">{formatMs(seg.start_ms)}</span>
                      {seg.text}
                    </p>
                  ))
                )}
              </div>
            </>
          )}
        </section>
      </div>

      {consentOpen && (
        <div className="wizard-backdrop" onClick={() => setConsentOpen(false)}>
          <div className="confirm-modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Tell people they are being recorded</h3>
            <p>
              Start the meeting only after the other people know. Lipi records your microphone and the audio this
              computer is playing, including sound in headphones. The audio and the note stay on this computer.
            </p>
            <div className="confirm-modal-actions">
              <button type="button" className="btn btn-secondary" onClick={() => setConsentOpen(false)}>
                Cancel
              </button>
              <button type="button" className="btn btn-primary" onClick={start}>
                Start recording
              </button>
            </div>
          </div>
        </div>
      )}

      {confirmDelete && detail && (
        <div className="wizard-backdrop" onClick={() => setConfirmDelete(false)}>
          <div className="confirm-modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Delete this meeting?</h3>
            <p>The recording and transcript for this meeting are removed. The saved note stays in History.</p>
            <div className="confirm-modal-actions">
              <button type="button" className="btn btn-secondary" onClick={() => setConfirmDelete(false)}>
                Keep
              </button>
              <button type="button" className="btn btn-primary" onClick={() => remove(detail.session.id)}>
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function parseDecisions(raw: string): string[] {
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return parsed.filter((item) => typeof item === "string" && item.trim());
    }
  } catch {
    if (raw.trim() && raw.trim() !== "[]") return [raw.trim()];
  }
  return [];
}

function formatMs(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function formatClock(total: number): string {
  return formatMs(total * 1000);
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}
