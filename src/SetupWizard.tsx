import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface SystemSpecs {
  total_ram_gb: number;
  cpu_cores: number;
  os: string;
  recommended_mode: "local" | "cloud";
  recommended_whisper: "tiny" | "base" | "small";
  summary_text: string;
}

export interface PrerequisiteStatus {
  has_audio_device: boolean;
  audio_device_name: string | null;
}

export type LocalModelSize = "tiny" | "base" | "small" | "medium" | "turbo_q8";
type SetupPath = "local" | "cloud";
type PolishChoice = "later" | "api";
type ProviderKind = "cloudflare" | "groq" | "openai" | "ollama" | "custom";
type PreparePhase = "idle" | "runner" | "weights" | "ready" | "error";

interface SpeechModelOption {
  id: LocalModelSize;
  title: string;
  ram: string;
  ramGb: number;
  summary: string;
}

interface ProviderOption {
  id: ProviderKind;
  title: string;
  subtitle: string;
}

interface EndpointDraft {
  kind: ProviderKind;
  apiKey: string;
  accountId: string;
  baseUrl: string;
  tested: boolean;
  testing: boolean;
  error: string | null;
  detail: string | null;
  models: string[];
  speechModel: string;
  chatModel: string;
}

interface SavedProvider {
  id: string;
  name: string;
  provider_type: string;
  api_key: string;
  account_id: string;
  base_url: string;
  model: string;
}

const SPEECH_MODELS: SpeechModelOption[] = [
  {
    id: "turbo_q8",
    title: "Turbo v3",
    ram: "About 2.2 GB RAM",
    ramGb: 2.2,
    summary: "Best accuracy. Recommended when this PC can spare about 2.2 GB.",
  },
  {
    id: "medium",
    title: "Medium",
    ram: "About 2.5 GB RAM",
    ramGb: 2.5,
    summary: "Strong accuracy, and quicker than Turbo v3.",
  },
  {
    id: "small",
    title: "Small",
    ram: "About 1 GB RAM",
    ramGb: 1,
    summary: "Fast transcription, and better with accents than Base.",
  },
  {
    id: "base",
    title: "Base",
    ram: "About 0.5 GB RAM",
    ramGb: 0.5,
    summary: "Quick everyday transcription.",
  },
  {
    id: "tiny",
    title: "Tiny",
    ram: "About 0.3 GB RAM",
    ramGb: 0.3,
    summary: "Fastest and lightest. Good for short notes.",
  },
];

const PROVIDERS: ProviderOption[] = [
  { id: "cloudflare", title: "Cloudflare", subtitle: "Workers AI" },
  { id: "groq", title: "Groq", subtitle: "api.groq.com" },
  { id: "openai", title: "OpenAI", subtitle: "api.openai.com" },
  { id: "ollama", title: "Ollama", subtitle: "localhost:11434" },
  { id: "custom", title: "Custom", subtitle: "OpenAPI-compatible" },
];

const SETTINGS_HINT = "You can correct this provider in Settings.";

export interface SetupWizardProps {
  isOpen: boolean;
  onClose: () => void;
  onFinish: (
    settingsPatch: {
      engine_mode: "local" | "cloud";
      local_engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan";
      local_model_size: LocalModelSize;
      api_base_url: string;
      api_key: string;
      model: string;
      provider_id?: string;
      wizard_completed: boolean;
      live_dictation?: boolean;
    },
    llmPatch?: {
      enabled: boolean;
      auto_mode?: boolean;
      active_provider_id: string;
      model: string;
      provider?: SavedProvider;
      speech_provider?: SavedProvider;
    }
  ) => Promise<void>;
  onStartRecording: () => Promise<void>;
  currentSettings: {
    engine_mode: "local" | "cloud";
    local_engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan";
    local_model_size: LocalModelSize;
    api_base_url: string;
    api_key: string;
    model: string;
  };
  downloadProgress: number | null;
  downloadSpeedBps: number;
  downloadEtaSecs: number;
  isDownloading: boolean;
  onDownloadModel: (
    engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan",
    size: LocalModelSize
  ) => Promise<string | null>;
  showToast: (msg: string) => void;
}

function emptyEndpoint(kind: ProviderKind = "groq"): EndpointDraft {
  return {
    kind,
    apiKey: "",
    accountId: "",
    baseUrl: kind === "custom" ? "https://openrouter.ai/api/v1" : "",
    tested: false,
    testing: false,
    error: null,
    detail: null,
    models: [],
    speechModel: "",
    chatModel: "",
  };
}

function recommendedModel(ramGb: number): LocalModelSize {
  if (ramGb >= 8) return "turbo_q8";
  if (ramGb >= 6) return "small";
  if (ramGb >= 4) return "base";
  return "tiny";
}

function speechModel(id: LocalModelSize): SpeechModelOption {
  return SPEECH_MODELS.find((model) => model.id === id) ?? SPEECH_MODELS[3];
}

function endpointUrl(endpoint: EndpointDraft): string {
  if (endpoint.kind === "groq") return "https://api.groq.com/openai/v1";
  if (endpoint.kind === "openai") return "https://api.openai.com/v1";
  if (endpoint.kind === "ollama") return "http://localhost:11434/v1";
  if (endpoint.kind === "cloudflare") {
    const account = endpoint.accountId.trim();
    return account ? `https://api.cloudflare.com/client/v4/accounts/${account}/ai/v1` : "";
  }
  return endpoint.baseUrl.trim().replace(/\/$/, "");
}

function providerRecord(endpoint: EndpointDraft, model: string, role: "speech" | "polish"): SavedProvider {
  const names: Record<ProviderKind, string> = {
    cloudflare: "Cloudflare Workers AI",
    groq: "Groq Cloud",
    openai: "OpenAI",
    ollama: "Ollama (Local)",
    custom: "Custom",
  };
  const id =
    endpoint.kind === "custom"
      ? role === "speech"
        ? "CUSTOM_SPEECH"
        : "CUSTOM_POLISH"
      : endpoint.kind === "openai"
      ? "OPENAI_CUSTOM"
      : endpoint.kind === "groq"
      ? "GROQ_DEFAULT"
      : endpoint.kind === "ollama"
      ? "OLLAMA_DEFAULT"
      : "CLOUDFLARE_DEFAULT";
  return {
    id,
    name: names[endpoint.kind],
    provider_type: endpoint.kind,
    api_key: endpoint.apiKey.trim(),
    account_id: endpoint.kind === "cloudflare" ? endpoint.accountId.trim() : "",
    base_url: endpointUrl(endpoint),
    model,
  };
}

function canTest(endpoint: EndpointDraft): boolean {
  if (!endpointUrl(endpoint)) return false;
  if (endpoint.kind === "ollama") return true;
  return endpoint.apiKey.trim().length > 0;
}

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec <= 0) return "";
  if (bytesPerSec >= 1024 * 1024) return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
  return `${Math.round(bytesPerSec / 1024)} KB/s`;
}

function formatEta(seconds: number): string {
  if (seconds <= 0 || !isFinite(seconds)) return "";
  const s = Math.round(seconds);
  if (s < 60) return `~${s}s left`;
  const m = Math.floor(s / 60);
  return `~${m}m ${s % 60}s left`;
}

function ModelPicker({
  id,
  label,
  value,
  models,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  models: string[];
  onChange: (value: string) => void;
}) {
  return (
    <div className="form-group">
      <label className="form-label" htmlFor={id}>
        {label}
      </label>
      <select id={id} className="form-input" value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">Choose a model</option>
        {models.map((model) => (
          <option key={model} value={model}>
            {model}
          </option>
        ))}
      </select>
    </div>
  );
}

function ProviderForm({
  legend,
  endpoint,
  onChange,
  onTest,
  speechPicker,
  chatPicker,
}: {
  legend: string;
  endpoint: EndpointDraft;
  onChange: (patch: Partial<EndpointDraft>) => void;
  onTest: () => void;
  speechPicker: boolean;
  chatPicker: boolean;
}) {
  const fieldId = legend.toLowerCase().replace(/[^a-z]+/g, "-");
  return (
    <div className="wizard-subpanel">
      <h4 className="wizard-subheading">{legend}</h4>
      <div className="wizard-provider-tabs five">
        {PROVIDERS.map((item) => (
          <button
            key={item.id}
            type="button"
            className={`wizard-prov-btn ${endpoint.kind === item.id ? "active" : ""}`}
            onClick={() =>
              onChange({
                kind: item.id,
                tested: false,
                error: null,
                detail: null,
                models: [],
                speechModel: "",
                chatModel: "",
                baseUrl: item.id === "custom" ? endpoint.baseUrl || "https://openrouter.ai/api/v1" : "",
              })
            }
          >
            <span className="prov-btn-title">{item.title}</span>
            <span className="prov-btn-sub">{item.subtitle}</span>
          </button>
        ))}
      </div>

      {endpoint.kind === "custom" && (
        <div className="form-group">
          <label className="form-label" htmlFor={`${fieldId}-url`}>
            Base URL
          </label>
          <input
            id={`${fieldId}-url`}
            type="text"
            className="form-input"
            placeholder="https://openrouter.ai/api/v1"
            value={endpoint.baseUrl}
            onChange={(e) =>
              onChange({ baseUrl: e.target.value, tested: false, error: null, detail: null, models: [], speechModel: "", chatModel: "" })
            }
          />
          <span className="form-hint">OpenRouter and any other OpenAI-compatible API. Change the URL later in Settings.</span>
        </div>
      )}

      {endpoint.kind === "cloudflare" && (
        <div className="form-group">
          <label className="form-label" htmlFor={`${fieldId}-account`}>
            Account ID
          </label>
          <input
            id={`${fieldId}-account`}
            type="text"
            className="form-input"
            placeholder="Cloudflare account ID"
            value={endpoint.accountId}
            onChange={(e) =>
              onChange({ accountId: e.target.value, tested: false, error: null, detail: null, models: [], speechModel: "", chatModel: "" })
            }
          />
        </div>
      )}

      {endpoint.kind !== "ollama" && (
        <div className="form-group">
          <label className="form-label" htmlFor={`${fieldId}-key`}>
            API key
          </label>
          <div className="wizard-key-row">
            <input
              id={`${fieldId}-key`}
              type="password"
              className="form-input"
              placeholder="Paste the API key"
              value={endpoint.apiKey}
              onChange={(e) =>
                onChange({ apiKey: e.target.value, tested: false, error: null, detail: null, models: [], speechModel: "", chatModel: "" })
              }
            />
            <button type="button" className="btn btn-secondary" onClick={onTest} disabled={endpoint.testing || !canTest(endpoint)}>
              {endpoint.testing ? "Testing…" : endpoint.tested ? "Test again" : "Test connection"}
            </button>
          </div>
        </div>
      )}

      {endpoint.kind === "ollama" && (
        <div className="wizard-key-row">
          <span className="form-hint">Uses http://localhost:11434/v1. No API key.</span>
          <button type="button" className="btn btn-secondary" onClick={onTest} disabled={endpoint.testing}>
            {endpoint.testing ? "Testing…" : endpoint.tested ? "Test again" : "Test connection"}
          </button>
        </div>
      )}

      {endpoint.detail && <div className="wizard-fetch-success">{endpoint.detail}</div>}
      {endpoint.error && (
        <div className="wizard-fetch-error">
          {endpoint.error} {SETTINGS_HINT}
        </div>
      )}

      {endpoint.tested && speechPicker && (
        <ModelPicker
          id={`${fieldId}-speech-model`}
          label="Speech model"
          value={endpoint.speechModel}
          models={endpoint.models}
          onChange={(speechModel) => onChange({ speechModel })}
        />
      )}
      {endpoint.tested && chatPicker && (
        <ModelPicker
          id={`${fieldId}-chat-model`}
          label="Polish model"
          value={endpoint.chatModel}
          models={endpoint.models}
          onChange={(chatModel) => onChange({ chatModel })}
        />
      )}
    </div>
  );
}

export function SetupWizard({
  isOpen,
  onClose,
  onFinish,
  onStartRecording,
  currentSettings,
  downloadProgress,
  downloadSpeedBps,
  downloadEtaSecs,
  isDownloading,
  onDownloadModel,
  showToast,
}: SetupWizardProps) {
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [specs, setSpecs] = useState<SystemSpecs | null>(null);
  const [isLoadingSpecs, setIsLoadingSpecs] = useState(true);
  const [prereqs, setPrereqs] = useState<PrerequisiteStatus | null>(null);
  const [setupPath, setSetupPath] = useState<SetupPath>("local");
  const [polishChoice, setPolishChoice] = useState<PolishChoice>("later");
  const [localWhisperSize, setLocalWhisperSize] = useState<LocalModelSize>("base");
  const [speechEndpoint, setSpeechEndpoint] = useState<EndpointDraft>(emptyEndpoint());
  const [polishEndpoint, setPolishEndpoint] = useState<EndpointDraft>(emptyEndpoint("openai"));
  const [sameAsSpeech, setSameAsSpeech] = useState(true);
  const [deferPolish, setDeferPolish] = useState(false);
  const [preparePhase, setPreparePhase] = useState<PreparePhase>("idle");
  const [prepareError, setPrepareError] = useState<string | null>(null);
  const [isFinishing, setIsFinishing] = useState(false);

  const userChosePath = useRef(false);
  const userChoseModel = useRef(false);
  const prepareStarted = useRef<string | null>(null);
  const onDownloadModelRef = useRef(onDownloadModel);
  onDownloadModelRef.current = onDownloadModel;

  const selectedModel = speechModel(localWhisperSize);
  const totalRam = specs?.total_ram_gb ?? 8;
  const suggestedModel = recommendedModel(totalRam);
  const installing = preparePhase === "runner" || preparePhase === "weights" || isDownloading;
  const shareSpeechProvider = setupPath === "cloud" && polishChoice === "api" && sameAsSpeech && !deferPolish;
  const duplicateProvider =
    !deferPolish &&
    setupPath === "cloud" &&
    polishChoice === "api" &&
    !sameAsSpeech &&
    speechEndpoint.kind === polishEndpoint.kind &&
    (speechEndpoint.kind !== "custom" || endpointUrl(speechEndpoint) === endpointUrl(polishEndpoint));

  useEffect(() => {
    if (!isOpen) {
      prepareStarted.current = null;
      return;
    }
    setStep(1);
    setPolishChoice("later");
    setSpeechEndpoint(emptyEndpoint());
    setPolishEndpoint(emptyEndpoint("openai"));
    setSameAsSpeech(true);
    setDeferPolish(false);
    setPreparePhase("idle");
    setPrepareError(null);
    userChosePath.current = false;
    userChoseModel.current = false;
    setIsLoadingSpecs(true);

    invoke<SystemSpecs>("get_system_specs")
      .then((res) => {
        setSpecs(res);
        if (!userChosePath.current) setSetupPath(res.recommended_mode === "cloud" ? "cloud" : "local");
        if (!userChoseModel.current) setLocalWhisperSize(recommendedModel(res.total_ram_gb));
      })
      .catch((err) => console.error("Failed to read system specs:", err))
      .finally(() => setIsLoadingSpecs(false));

    invoke<PrerequisiteStatus>("check_system_prerequisites")
      .then((status) => setPrereqs(status))
      .catch((err) => console.warn("Failed to check microphone:", err));
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen || step !== 3 || setupPath !== "local") return;
    if (prepareStarted.current === localWhisperSize) return;
    prepareStarted.current = localWhisperSize;
    void runLocalPrepare(localWhisperSize);
  }, [isOpen, step, setupPath, localWhisperSize]);

  async function testEndpoint(target: "speech" | "polish") {
    const current = target === "speech" ? speechEndpoint : polishEndpoint;
    const patch = (next: Partial<EndpointDraft>) => {
      if (target === "speech") setSpeechEndpoint((prev) => ({ ...prev, ...next }));
      else setPolishEndpoint((prev) => ({ ...prev, ...next }));
    };
    const baseUrl = endpointUrl(current);
    if (!baseUrl) {
      patch({ tested: false, error: "Enter the account ID or base URL first." });
      return;
    }
    if (current.kind !== "ollama" && !current.apiKey.trim()) {
      patch({ tested: false, error: "Enter an API key first." });
      return;
    }
    patch({ testing: true, tested: false, error: null, detail: null, models: [], speechModel: "", chatModel: "" });
    try {
      const models = await invoke<string[]>("test_and_fetch_models", {
        baseUrl,
        apiKey: current.apiKey.trim(),
      });
      if (!models || models.length === 0) {
        patch({ testing: false, error: "The provider responded, but returned no models." });
        return;
      }
      patch({
        testing: false,
        tested: true,
        detail: `Connected. Choose a model from the ${models.length} returned.`,
        models,
      });
    } catch (err: unknown) {
      patch({ testing: false, tested: false, error: String(err) });
    }
  }

  async function runLocalPrepare(size: LocalModelSize) {
    prepareStarted.current = size;
    setPreparePhase("runner");
    setPrepareError(null);
    try {
      await invoke("prepare_engine", { engine: "whisper_cpu" });
      const status = await invoke<{ installed: boolean }>("get_model_status", {
        engine: "whisper_cpu",
        modelSize: size,
      });
      if (!status.installed) {
        setPreparePhase("weights");
        const err = await onDownloadModelRef.current("whisper_cpu", size);
        if (err) throw new Error(err);
      }
      setPreparePhase("ready");
    } catch (err: unknown) {
      setPrepareError(err instanceof Error ? err.message : String(err));
      setPreparePhase("error");
      prepareStarted.current = null;
    }
  }

  function speechReady(): boolean {
    if (setupPath !== "cloud") return true;
    return speechEndpoint.tested && speechEndpoint.speechModel.trim().length > 0;
  }

  function polishReady(): boolean {
    if (polishChoice !== "api" || deferPolish) return true;
    if (shareSpeechProvider) return speechEndpoint.tested && speechEndpoint.chatModel.trim().length > 0;
    return polishEndpoint.tested && polishEndpoint.chatModel.trim().length > 0;
  }

  function canContinueSetup(): boolean {
    return speechReady() && polishReady() && !duplicateProvider;
  }

  function setupBlockedReason(): string | null {
    if (duplicateProvider) return "Use the same provider, or pick a different one for AI polish.";
    if (setupPath === "cloud" && !speechEndpoint.tested) return "Test the speech provider before continuing.";
    if (setupPath === "cloud" && !speechEndpoint.speechModel) return "Choose a speech model.";
    if (deferPolish) return null;
    if (polishChoice === "api" && shareSpeechProvider && !speechEndpoint.chatModel) return "Choose a polish model.";
    if (polishChoice === "api" && !shareSpeechProvider && !polishEndpoint.tested) return "Test the AI polish provider before continuing.";
    if (polishChoice === "api" && !shareSpeechProvider && !polishEndpoint.chatModel) return "Choose a polish model.";
    return null;
  }

  async function handleSkip() {
    await onFinish({
      engine_mode: currentSettings.engine_mode || "local",
      local_engine: currentSettings.local_engine || "whisper_cpu",
      local_model_size: currentSettings.local_model_size || "base",
      api_base_url: currentSettings.api_base_url || "https://api.openai.com/v1",
      api_key: currentSettings.api_key || "",
      model: currentSettings.model || "whisper-1",
      wizard_completed: true,
    });
    onClose();
    showToast("Speech isn’t ready yet. Finish setup when you want to dictate.");
  }

  async function persistSetup() {
    const speechRecord = setupPath === "cloud" ? providerRecord(speechEndpoint, speechEndpoint.speechModel, "speech") : null;
    const speechSettings =
      setupPath === "local"
        ? {
            engine_mode: "local" as const,
            local_engine: "whisper_cpu" as const,
            local_model_size: localWhisperSize,
            api_base_url: currentSettings.api_base_url || "https://api.openai.com/v1",
            api_key: "",
            model: `whisper-${localWhisperSize}`,
            wizard_completed: true,
            live_dictation: false,
          }
        : {
            engine_mode: "cloud" as const,
            local_engine: "whisper_cpu" as const,
            local_model_size: "base" as const,
            api_base_url: speechRecord?.base_url || "",
            api_key: speechEndpoint.apiKey.trim(),
            model: speechEndpoint.speechModel,
            provider_id: speechRecord?.id,
            wizard_completed: true,
            live_dictation: false,
          };

    if (polishChoice !== "api" || deferPolish) {
      await onFinish(speechSettings, {
        enabled: false,
        auto_mode: false,
        active_provider_id: speechRecord?.id || "",
        model: "",
        ...(speechRecord ? { provider: speechRecord } : {}),
      });
      return;
    }

    const polishSource = shareSpeechProvider ? { ...speechEndpoint, chatModel: speechEndpoint.chatModel } : polishEndpoint;
    const polishRecord = providerRecord(polishSource, polishSource.chatModel, "polish");
    await onFinish(speechSettings, {
      enabled: true,
      auto_mode: true,
      active_provider_id: polishRecord.id,
      model: polishRecord.model,
      provider: polishRecord,
      ...(speechRecord && speechRecord.id !== polishRecord.id ? { speech_provider: speechRecord } : {}),
    });
  }

  async function finishSetup(startRecording: boolean) {
    if (preparePhase !== "ready" || isFinishing) return;
    setIsFinishing(true);
    try {
      await persistSetup();
      onClose();
      if (startRecording) await onStartRecording();
      else showToast("Setup complete. Press Alt+R to dictate.");
    } catch (err: unknown) {
      showToast(String(err));
    } finally {
      setIsFinishing(false);
    }
  }

  function goToPrepare() {
    if (!canContinueSetup()) return;
    if (setupPath === "cloud") setPreparePhase("ready");
    else if (prepareStarted.current !== localWhisperSize || preparePhase !== "ready") setPreparePhase("idle");
    setStep(3);
  }

  if (!isOpen) return null;

  const micReady = !!prereqs?.has_audio_device;
  const micLabel = micReady ? prereqs?.audio_device_name || "Default microphone" : "No microphone found";
  const modelTooHeavy = totalRam < selectedModel.ramGb + 1.5;
  const polishLabel = !polishChoice || polishChoice === "later" || deferPolish
    ? "Transcript only. Add a provider later in Settings."
    : shareSpeechProvider
    ? `${speechEndpoint.kind} · ${speechEndpoint.chatModel}`
    : `${polishEndpoint.kind} · ${polishEndpoint.chatModel}`;

  return (
    <div className="wizard-backdrop">
      <div className="wizard-container" role="dialog" aria-modal="true" aria-labelledby="wizard-title">
        <div className="wizard-header">
          <div className="wizard-brand">
            <span className="wizard-icon">🎙️</span>
            <div>
              <h2 id="wizard-title" className="wizard-title">Lipi Setup</h2>
              <p className="wizard-subtitle">
                {isLoadingSpecs ? "Checking this PC…" : `${totalRam} GB RAM · ${specs?.cpu_cores ?? "?"} cores · ${micLabel}`}
              </p>
            </div>
          </div>
          <button type="button" className="wizard-btn-skip" onClick={() => void handleSkip()} title="Leave setup for later">
            Set up later
          </button>
        </div>

        <div className="wizard-stepper">
          <button type="button" className={`wizard-step-pill ${step === 1 ? "active" : ""} ${step > 1 ? "completed" : ""}`} onClick={() => { if (!installing) setStep(1); }}>
            <span className="step-num">1</span> Choose
          </button>
          <div className="step-divider" />
          <button type="button" className={`wizard-step-pill ${step === 2 ? "active" : ""} ${step > 2 ? "completed" : ""}`} onClick={() => { if (!installing && step > 2) setStep(2); }}>
            <span className="step-num">2</span> Setup
          </button>
          <div className="step-divider" />
          <button type="button" className={`wizard-step-pill ${step === 3 ? "active" : ""}`} disabled={step < 3}>
            <span className="step-num">3</span> Prepare
          </button>
        </div>

        <div className="wizard-body">
          {step === 1 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">What should Lipi do?</h3>
              <p className="wizard-step-desc">Pick where speech is transcribed, then whether the text stays as you said it or gets an AI polish.</p>

              <h4 className="wizard-choice-label">Speech</h4>
              <div className="wizard-path-selector">
                <label className={`wizard-path-card ${setupPath === "local" ? "selected" : ""}`}>
                  <input type="radio" name="setup_path" checked={setupPath === "local"} onChange={() => { userChosePath.current = true; setSetupPath("local"); }} />
                  <div>
                    <div className="path-title">
                      <span>On this computer</span>
                      {specs?.recommended_mode !== "cloud" && <span className="path-pill-rec">Recommended</span>}
                    </div>
                    <div className="path-desc">Private and offline. Lipi downloads a speech model and runs it here.</div>
                  </div>
                </label>
                <label className={`wizard-path-card ${setupPath === "cloud" ? "selected" : ""}`}>
                  <input type="radio" name="setup_path" checked={setupPath === "cloud"} onChange={() => { userChosePath.current = true; setSetupPath("cloud"); }} />
                  <div>
                    <div className="path-title">
                      <span>In the cloud</span>
                      {specs?.recommended_mode === "cloud" && <span className="path-pill-rec">Recommended</span>}
                    </div>
                    <div className="path-desc">Uses a provider from the same list as Settings. You will test the key next.</div>
                  </div>
                </label>
              </div>

              <h4 className="wizard-choice-label">After you speak</h4>
              <div className="wizard-path-selector stack">
                <label className={`wizard-path-card ${polishChoice === "later" ? "selected" : ""}`}>
                  <input type="radio" name="result_mode" checked={polishChoice === "later"} onChange={() => setPolishChoice("later")} />
                  <div>
                    <div className="path-title"><span>Transcript only</span></div>
                    <div className="path-desc">Lipi writes what you said. You can add a provider for AI polish later in Settings.</div>
                  </div>
                </label>
                <label className={`wizard-path-card ${polishChoice === "api" ? "selected" : ""}`}>
                  <input type="radio" name="result_mode" checked={polishChoice === "api"} onChange={() => setPolishChoice("api")} />
                  <div>
                    <div className="path-title"><span>Polish with AI</span></div>
                    <div className="path-desc">Cloudflare, Groq, OpenAI, Ollama, or a custom OpenAPI endpoint rewrites the text. Speech can stay on this computer.</div>
                  </div>
                </label>
              </div>

              {!micReady && prereqs && (
                <p className="wizard-inline-warn">No microphone was found. You can finish setup, then connect a mic before dictating.</p>
              )}

              <div className="wizard-footer-actions">
                <span />
                <button type="button" className="btn btn-primary" onClick={() => setStep(2)}>Next</button>
              </div>
            </div>
          )}

          {step === 2 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">{setupPath === "local" ? "Choose a speech model" : "Connect speech"}</h3>
              <p className="wizard-step-desc">
                {setupPath === "local"
                  ? "Turbo v3 needs about 2.2 GB of RAM and is recommended for better accuracy. Medium, Small, and Base are for fast, quick transcription."
                  : "Use the same provider types as Settings. Test the key, then choose the model yourself."}
              </p>

              {setupPath === "local" ? (
                <>
                  <div className="wizard-model-sizes-grid">
                    {SPEECH_MODELS.map((model) => (
                      <label key={model.id} className={`wizard-size-card ${localWhisperSize === model.id ? "active" : ""}`}>
                        <input
                          type="radio"
                          name="whisper_size"
                          checked={localWhisperSize === model.id}
                          onChange={() => {
                            userChoseModel.current = true;
                            setLocalWhisperSize(model.id);
                          }}
                        />
                        <div className="size-title">
                          {model.title}
                          {model.id === suggestedModel && <span className="tag-recommended">Recommended</span>}
                        </div>
                        <div className="size-ram">{model.ram}</div>
                        <div className="size-desc">{model.summary}</div>
                      </label>
                    ))}
                  </div>
                  {modelTooHeavy && (
                    <p className="wizard-inline-warn">
                      {selectedModel.title} wants about {selectedModel.ramGb} GB. This PC has {totalRam} GB, so a smaller model will stay more responsive.
                    </p>
                  )}
                  <p className="wizard-step-note">A different engine or live dictation can be changed later in Settings.</p>
                </>
              ) : (
                <ProviderForm
                  legend="Speech provider"
                  endpoint={speechEndpoint}
                  onChange={(patch) => setSpeechEndpoint((prev) => ({ ...prev, ...patch }))}
                  onTest={() => void testEndpoint("speech")}
                  speechPicker
                  chatPicker={shareSpeechProvider}
                />
              )}

              {polishChoice === "later" && (
                <p className="wizard-step-note">AI polish is off. Add Cloudflare, Groq, OpenAI, Ollama, or a custom endpoint later in Settings.</p>
              )}

              {polishChoice === "api" && (
                <div className="wizard-defer-toggle">
                  <label className="checkbox-label">
                    <input
                      type="checkbox"
                      className="checkbox-input"
                      checked={deferPolish}
                      onChange={(e) => setDeferPolish(e.target.checked)}
                    />
                    <span>Set up the AI provider later</span>
                  </label>
                  <p className="wizard-step-note">
                    Continue with no key and no model. Add or change Cloudflare, Groq, OpenAI, Ollama, or a custom endpoint later in Settings.
                  </p>
                </div>
              )}

              {setupPath === "cloud" && polishChoice === "api" && !deferPolish && (
                <label className="checkbox-label" style={{ marginTop: "8px" }}>
                  <input type="checkbox" className="checkbox-input" checked={sameAsSpeech} onChange={(e) => setSameAsSpeech(e.target.checked)} />
                  <span>Use this same provider for AI polish</span>
                </label>
              )}

              {polishChoice === "api" && !deferPolish && !shareSpeechProvider && (
                <ProviderForm
                  legend="AI polish provider"
                  endpoint={polishEndpoint}
                  onChange={(patch) => setPolishEndpoint((prev) => ({ ...prev, ...patch }))}
                  onTest={() => void testEndpoint("polish")}
                  speechPicker={false}
                  chatPicker
                />
              )}

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={() => setStep(1)}>Back</button>
                <div className="wizard-footer-next">
                  {setupBlockedReason() && <span className="wizard-step-note">{setupBlockedReason()}</span>}
                  <button type="button" className="btn btn-primary" disabled={!canContinueSetup()} onClick={goToPrepare}>
                    Next: Prepare
                  </button>
                </div>
              </div>
            </div>
          )}

          {step === 3 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">Prepare</h3>
              <p className="wizard-step-desc">
                {setupPath === "local"
                  ? `Installing ${selectedModel.title} now (${selectedModel.ram}). Dictation starts when this finishes.`
                  : "The provider key worked. Nothing else needs to download."}
              </p>

              <div className="wizard-summary-card">
                <div className="summary-row">
                  <span className="summary-label">Speech</span>
                  <span className="summary-value">
                    {setupPath === "local"
                      ? `${selectedModel.title} on this computer`
                      : `${speechEndpoint.kind} · ${speechEndpoint.speechModel}`}
                  </span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">Result</span>
                  <span className="summary-value">{polishLabel}</span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">Microphone</span>
                  <span className="summary-value">{micLabel}</span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">Install</span>
                  <span className="summary-value">
                    {setupPath === "cloud"
                      ? "Provider tested"
                      : preparePhase === "runner"
                      ? "Installing the speech engine…"
                      : preparePhase === "weights" || isDownloading
                      ? "Downloading the speech model…"
                      : preparePhase === "ready"
                      ? "Ready"
                      : preparePhase === "error"
                      ? "Could not finish"
                      : "Starting…"}
                  </span>
                </div>
              </div>

              {setupPath === "local" && (preparePhase === "weights" || isDownloading) && (
                <div className="wizard-prepare-progress">
                  <div className="downloading-info">
                    <span>Downloading {selectedModel.title}{downloadProgress !== null ? `… ${downloadProgress}%` : "…"}</span>
                    <span className="downloading-stats">
                      {downloadSpeedBps > 0 && formatSpeed(downloadSpeedBps)}
                      {downloadEtaSecs > 0 && ` · ${formatEta(downloadEtaSecs)}`}
                    </span>
                  </div>
                  <div className="progress-bar-bg">
                    <div className="progress-bar-fill" style={{ width: `${Math.min(downloadProgress ?? 0, 100)}%` }} />
                  </div>
                </div>
              )}

              {setupPath === "local" && preparePhase === "runner" && !isDownloading && (
                <div className="wizard-loading-card"><span className="spinner-dot" /> Installing the speech engine…</div>
              )}
              {prepareError && <div className="wizard-fetch-error">{prepareError}</div>}
              {preparePhase === "ready" && (
                <p className="wizard-step-note">Press <kbd className="key-hint">Alt+R</kbd> in any app to dictate, or start a recording here.</p>
              )}

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" disabled={installing || isFinishing} onClick={() => setStep(2)}>Back</button>
                <div className="wizard-footer-next">
                  {preparePhase === "error" && (
                    <button type="button" className="btn btn-secondary" onClick={() => void runLocalPrepare(localWhisperSize)}>Try again</button>
                  )}
                  {preparePhase === "ready" && (
                    <button type="button" className="btn btn-secondary" disabled={isFinishing} onClick={() => void finishSetup(false)}>Not now</button>
                  )}
                  <button type="button" className="btn btn-primary" disabled={preparePhase !== "ready" || isFinishing} onClick={() => void finishSetup(true)}>
                    {isFinishing ? "Starting…" : "Start recording"}
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
