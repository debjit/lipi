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
  const [phase, setPhase] = useState<"idle" | "starting" | "recording" | "saving">("idle");
  const [chunkBusy, setChunkBusy] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const activeIdRef = useRef<number | null>(null);
  const recordingIdRef = useRef<number | null>(null);
  const stopRef = useRef<(id: number) => void>(() => {});

  async function refreshList(selectId?: number) {
    const list = await invoke<MeetingSession[]>("list_meetings");
    setSessions(list);
    const id = selectId ?? activeIdRef.current ?? list[0]?.id;
    if (id) {
      const next = await invoke<MeetingDetail>("get_meeting", { sessionId: id });
      activeIdRef.current = next.session.id;
      setDetail(next);
      if (next.session.status === "recording") {
        recordingIdRef.current = next.session.id;
        setPhase("recording");
      }
    } else {
      activeIdRef.current = null;
      setDetail(null);
    }
  }

  useEffect(() => {
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
    track("meeting_error", (message: string) => setError(message));
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
    if (phase !== "recording" || !detail) return;
    const tick = () => setElapsed(Math.max(0, Math.floor(Date.now() / 1000) - detail.session.started_epoch));
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [phase, detail?.session.id, detail?.session.started_epoch]);

  useEffect(() => {
    const node = transcriptRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [detail?.segments.length]);

  async function openSession(id: number) {
    setError(null);
    setConfirmDelete(false);
    const next = await invoke<MeetingDetail>("get_meeting", { sessionId: id });
    activeIdRef.current = id;
    setDetail(next);
  }

  async function start() {
    setConsentOpen(false);
    setError(null);
    setPhase("starting");
    try {
      const session = await invoke<MeetingSession>("start_meeting");
      activeIdRef.current = session.id;
      recordingIdRef.current = session.id;
      setDetail({ session, segments: [], actions: [] });
      setPhase("recording");
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
    } catch (err) {
      setPhase("idle");
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
      if (activeIdRef.current === id) setDetail(next);
      const list = await invoke<MeetingSession[]>("list_meetings");
      setSessions(list);
      if (stoppingLive) {
        recordingIdRef.current = null;
        setChunkBusy(false);
      }
      setPhase(recordingIdRef.current ? "recording" : "idle");
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
      setPhase("idle");
      setChunkBusy(false);
      await refreshList(id);
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
      await refreshList();
    } catch (err) {
      setError(String(err));
    }
  }

  const recording = phase === "recording";
  const busy = phase === "starting" || phase === "saving";
  const decisions = parseDecisions(detail?.session.decisions || "[]");
  const canSaveInterrupted = detail?.session.status === "interrupted";

  return (
    <div className="meetings-page">
      <header className="meetings-header">
        <button type="button" className="btn-back" onClick={onBack}>
          ← Notes
        </button>
        <h2>Meetings</h2>
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
                className={`meetings-list-item ${detail?.session.id === session.id ? "active" : ""}`}
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

          <div className="meetings-controls">
            {recording ? (
              <>
                <button
                  type="button"
                  className="btn btn-record recording"
                  onClick={() => {
                    const id = recordingIdRef.current;
                    if (id) void stop(id);
                  }}
                  disabled={busy}
                >
                  ■ <span className="btn-record-text">Stop</span>
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
                >
                  Cancel
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn-record"
                onClick={() => setConsentOpen(true)}
                disabled={busy}
              >
                {phase === "starting" ? "Starting…" : phase === "saving" ? "Writing note…" : "● Record meeting"}
              </button>
            )}
            {canSaveInterrupted && (
              <button type="button" className="btn btn-secondary" onClick={() => detail && stop(detail.session.id)} disabled={busy}>
                Save note
              </button>
            )}
            <span className="form-hint">Up to 2 hours. Dictation with Alt+R is unchanged.</span>
          </div>

          {detail && (
            <>
              <div className="meetings-summary">
                <h3>{detail.session.title || "Meeting"}</h3>
                {detail.session.summary && <p>{detail.session.summary}</p>}
                {decisions.length > 0 && (
                  <>
                    <h4>Decisions</h4>
                    <ul>
                      {decisions.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                  </>
                )}
                {detail.actions.length > 0 && (
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
              Start the meeting only after the other people know. Lipi keeps the audio and the note on this computer.
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
