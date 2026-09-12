import { useState, useEffect } from "react";
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
  os: string;
  has_audio_device: boolean;
  audio_device_name: string | null;
  whisper_binary_found: boolean;
  whisper_binary_path: string | null;
  python_found: boolean;
  python_path: string | null;
  venv_python_found: boolean;
  ollama_binary_found: boolean;
}

export interface SetupWizardProps {
  isOpen: boolean;
  onClose: () => void;
  onFinish: (
    settingsPatch: {
      engine_mode: "local" | "cloud";
      local_engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan";
      local_model_size: "tiny" | "base" | "small";
      api_base_url: string;
      api_key: string;
      model: string;
      provider_id?: string;
      wizard_completed: boolean;
    },
    llmPatch?: {
      enabled: boolean;
      active_provider_id: string;
      model: string;
      provider?: {
        id: string;
        name: string;
        provider_type: string;
        api_key: string;
        account_id: string;
        base_url: string;
        model: string;
      };
    }
  ) => Promise<void>;
  currentSettings: {
    engine_mode: "local" | "cloud";
    local_engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan";
    local_model_size: "tiny" | "base" | "small";
    api_base_url: string;
    api_key: string;
    model: string;
  };
  modelStatus: {
    installed: boolean;
    model_size: string;
  } | null;
  downloadProgress: number | null;
  downloadSpeedBps: number;
  downloadEtaSecs: number;
  isDownloading: boolean;
  onDownloadModel: (engine: "whisper_cpu" | "faster_whisper" | "whisper_vulkan", size: "tiny" | "base" | "small") => Promise<void>;
  showToast: (msg: string) => void;
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
  const rem = s % 60;
  return `~${m}m ${rem}s left`;
}

function getModelRamFeedback(modelName: string, totalRamGb: number) {
  const lower = (modelName || "").toLowerCase();
  let paramSizeBillion = 3.0;
  if (lower.includes(":1b") || lower.includes("1.3b") || lower.includes("1b") || lower.includes("smollm")) {
    paramSizeBillion = 1.2;
  } else if (lower.includes(":2b") || lower.includes("2b")) {
    paramSizeBillion = 2.0;
  } else if (lower.includes(":3b") || lower.includes("3b")) {
    paramSizeBillion = 3.2;
  } else if (lower.includes(":7b") || lower.includes("7b") || lower.includes("bonsai") || lower.includes("mistral")) {
    paramSizeBillion = 7.0;
  } else if (lower.includes(":8b") || lower.includes("8b")) {
    paramSizeBillion = 8.0;
  } else if (lower.includes(":14b") || lower.includes("14b")) {
    paramSizeBillion = 14.0;
  } else if (lower.includes(":32b") || lower.includes("32b")) {
    paramSizeBillion = 32.0;
  } else if (lower.includes(":70b") || lower.includes("70b")) {
    paramSizeBillion = 70.0;
  }

  const ramNeededGb = Math.round((paramSizeBillion * 0.75 + 1.2) * 10) / 10;
  if (totalRamGb >= ramNeededGb + 3.0) {
    return {
      status: "great",
      badgeClass: "wizard-ram-badge-great",
      label: `🟢 Runs smooth on this PC (~${ramNeededGb} GB RAM needed)`,
    };
  } else if (totalRamGb >= ramNeededGb + 0.5) {
    return {
      status: "moderate",
      badgeClass: "wizard-ram-badge-moderate",
      label: `🟡 Fits in RAM (~${ramNeededGb} GB RAM needed, tight)`,
    };
  } else {
    return {
      status: "heavy",
      badgeClass: "wizard-ram-badge-heavy",
      label: `🔴 Exceeds RAM (~${ramNeededGb} GB needed) - Cloud recommended`,
    };
  }
}

export function SetupWizard({
  isOpen,
  onClose,
  onFinish,
  currentSettings,
  modelStatus,
  downloadProgress,
  downloadSpeedBps,
  downloadEtaSecs,
  isDownloading,
  onDownloadModel,
  showToast,
}: SetupWizardProps) {
  const [step, setStep] = useState<1 | 2 | 3 | 4 | 5>(1);
  const [specs, setSpecs] = useState<SystemSpecs | null>(null);
  const [isLoadingSpecs, setIsLoadingSpecs] = useState(true);

  // Prerequisites state
  const [prereqs, setPrereqs] = useState<PrerequisiteStatus | null>(null);
  const [isLoadingPrereqs, setIsLoadingPrereqs] = useState(false);
  const [activeOsTab, setActiveOsTab] = useState<"linux" | "windows">("linux");
  const [isSettingUpEngine, setIsSettingUpEngine] = useState(false);
  const [engineSetupMsg, setEngineSetupMsg] = useState<string | null>(null);

  // Setup path
  const [setupPath, setSetupPath] = useState<"local" | "cloud">("local");

  // Local ASR model settings
  const [localWhisperSize, setLocalWhisperSize] = useState<"tiny" | "base" | "small">("base");
  const [localWhisperEngine, setLocalWhisperEngine] = useState<"whisper_cpu" | "faster_whisper">("whisper_cpu");

  // Local LLM (Ollama) settings
  const [localLlmMode, setLocalLlmMode] = useState<"ollama" | "none">("ollama");
  const [ollamaAvailable, setOllamaAvailable] = useState<boolean | null>(null);
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [selectedOllamaModel, setSelectedOllamaModel] = useState<string>("llama3.2");
  const [isCheckingOllama, setIsCheckingOllama] = useState(false);

  // Cloud / OpenAPI settings
  const [cloudCategory, setCloudCategory] = useState<"cloudflare" | "openapi">("openapi");
  const [openapiTemplate, setOpenapiTemplate] = useState<"groq" | "google" | "openai" | "custom">("groq");
  const [cloudBaseUrl, setCloudBaseUrl] = useState("https://api.groq.com/openai/v1");
  const [cloudApiKey, setCloudApiKey] = useState("");
  const [cloudAccountId, setCloudAccountId] = useState("");
  const [cloudModel, setCloudModel] = useState("whisper-large-v3-turbo");
  const [cloudLlmModel, setCloudLlmModel] = useState("llama-3.3-70b-versatile");
  const [cloudAvailableModels, setCloudAvailableModels] = useState<string[]>([]);
  const [isFetchingCloudModels, setIsFetchingCloudModels] = useState(false);
  const [cloudModelFetchSuccess, setCloudModelFetchSuccess] = useState<string | null>(null);
  const [cloudModelFetchError, setCloudModelFetchError] = useState<string | null>(null);

  // Load specs & initial checks on open
  useEffect(() => {
    if (!isOpen) return;
    setStep(1);
    setIsLoadingSpecs(true);

    invoke<SystemSpecs>("get_system_specs")
      .then((res) => {
        setSpecs(res);
        if (res.recommended_mode === "cloud") {
          setSetupPath("cloud");
        } else {
          setSetupPath("local");
        }
        if (res.recommended_whisper) {
          setLocalWhisperSize(res.recommended_whisper);
        }
        if (res.os && res.os.toLowerCase().includes("win")) {
          setActiveOsTab("windows");
        } else {
          setActiveOsTab("linux");
        }
      })
      .catch((err) => {
        console.error("Failed to read system specs:", err);
      })
      .finally(() => {
        setIsLoadingSpecs(false);
      });

    checkOllamaStatus();
    fetchPrereqs();
  }, [isOpen]);

  async function fetchPrereqs() {
    setIsLoadingPrereqs(true);
    try {
      const p = await invoke<PrerequisiteStatus>("check_system_prerequisites");
      setPrereqs(p);
      if (p.os && p.os.toLowerCase().includes("win")) {
        setActiveOsTab("windows");
      }
    } catch (err) {
      console.warn("Failed to check prerequisites:", err);
    } finally {
      setIsLoadingPrereqs(false);
    }
  }

  async function checkOllamaStatus() {
    setIsCheckingOllama(true);
    try {
      const models: string[] = await invoke("test_and_fetch_models", {
        baseUrl: "http://localhost:11434/v1",
        apiKey: "",
      });
      setOllamaAvailable(true);
      if (models && models.length > 0) {
        setOllamaModels(models);
        setSelectedOllamaModel(models[0]);
      } else {
        setOllamaModels([]);
      }
    } catch {
      setOllamaAvailable(false);
      setOllamaModels([]);
    } finally {
      setIsCheckingOllama(false);
    }
  }

  async function handleSetupRunner() {
    setIsSettingUpEngine(true);
    setEngineSetupMsg(null);
    try {
      const res = await invoke<string>("prepare_engine", { engine: localWhisperEngine });
      setEngineSetupMsg(`✓ ${res}`);
      showToast(res);
      await fetchPrereqs();
    } catch (err: any) {
      const errText = String(err);
      setEngineSetupMsg(`⚠️ ${errText}`);
      showToast(`Runner setup error: ${errText}`);
    } finally {
      setIsSettingUpEngine(false);
    }
  }

  function handleSelectOpenApiTemplate(tmpl: "groq" | "google" | "openai" | "custom") {
    setOpenapiTemplate(tmpl);
    setCloudModelFetchSuccess(null);
    setCloudModelFetchError(null);
    setCloudAvailableModels([]);

    if (tmpl === "groq") {
      setCloudBaseUrl("https://api.groq.com/openai/v1");
      setCloudModel("");
      setCloudLlmModel("");
    } else if (tmpl === "google") {
      setCloudBaseUrl("https://generativelanguage.googleapis.com/v1beta/openai");
      setCloudModel("");
      setCloudLlmModel("");
    } else if (tmpl === "openai") {
      setCloudBaseUrl("https://api.openai.com/v1");
      setCloudModel("");
      setCloudLlmModel("");
    } else {
      setCloudBaseUrl("https://api.openai.com/v1");
      setCloudModel("");
      setCloudLlmModel("");
    }
  }

  async function handleFetchCloudModels() {
    if (!cloudBaseUrl.trim()) {
      setCloudModelFetchError("Enter an API base URL first");
      return;
    }
    setIsFetchingCloudModels(true);
    setCloudModelFetchError(null);
    setCloudModelFetchSuccess(null);
    try {
      const models: string[] = await invoke("test_and_fetch_models", {
        baseUrl: cloudBaseUrl.trim(),
        apiKey: cloudApiKey.trim(),
      });
      if (models && models.length > 0) {
        setCloudAvailableModels(models);
        setCloudModelFetchSuccess(`✓ Found ${models.length} live models from API`);
        if (openapiTemplate === "groq") {
          const llm = models.find((m) => m.includes("llama-3.3") || m.includes("llama-3.1")) || models[0];
          setCloudLlmModel(llm);
          const asr = models.find((m) => m.includes("whisper")) || "whisper-large-v3-turbo";
          setCloudModel(asr);
        } else if (openapiTemplate === "google") {
          const gemini = models.find((m) => m.includes("gemini-2.5-flash") || m.includes("gemini-2.0") || m.includes("gemini-1.5")) || models[0];
          setCloudLlmModel(gemini);
        } else {
          setCloudLlmModel(models[0]);
        }
      } else {
        setCloudModelFetchError("No models returned by endpoint");
      }
    } catch (err: any) {
      setCloudModelFetchError(String(err));
    } finally {
      setIsFetchingCloudModels(false);
    }
  }

  async function handleSkip() {
    await onFinish({
      engine_mode: currentSettings.engine_mode || "cloud",
      local_engine: currentSettings.local_engine || "whisper_cpu",
      local_model_size: currentSettings.local_model_size || "base",
      api_base_url: currentSettings.api_base_url || "https://api.openai.com/v1",
      api_key: currentSettings.api_key || "",
      model: currentSettings.model || "whisper-1",
      wizard_completed: true,
    });
    onClose();
    showToast("Setup skipped. You can configure anytime in Settings.");
  }

  async function handleCompleteSetup() {
    if (setupPath === "local") {
      const llmEnabled = localLlmMode === "ollama";
      await onFinish(
        {
          engine_mode: "local",
          local_engine: localWhisperEngine,
          local_model_size: localWhisperSize,
          api_base_url: "https://api.openai.com/v1",
          api_key: "",
          model: `whisper-${localWhisperSize}`,
          wizard_completed: true,
        },
        llmEnabled
          ? {
              enabled: true,
              active_provider_id: "OLLAMA_DEFAULT",
              model: selectedOllamaModel || "llama3.2",
              provider: {
                id: "OLLAMA_DEFAULT",
                name: "Ollama (Local)",
                provider_type: "ollama",
                api_key: "",
                account_id: "",
                base_url: "http://localhost:11434/v1",
                model: selectedOllamaModel || "llama3.2",
              },
            }
          : {
              enabled: false,
              active_provider_id: "OLLAMA_DEFAULT",
              model: "llama3.2",
            }
      );
    } else {
      // Cloud mode
      if (cloudCategory === "cloudflare") {
        const acc = cloudAccountId.trim();
        const base = acc
          ? `https://api.cloudflare.com/client/v4/accounts/${acc}/ai/v1`
          : "https://api.cloudflare.com/client/v4/accounts/<account_id>/ai/v1";
        await onFinish(
          {
            engine_mode: "cloud",
            local_engine: "whisper_cpu",
            local_model_size: "base",
            api_base_url: base,
            api_key: cloudApiKey.trim(),
            model: "@cf/openai/whisper-large-v3-turbo",
            provider_id: "CLOUDFLARE_DEFAULT",
            wizard_completed: true,
          },
          {
            enabled: true,
            active_provider_id: "CLOUDFLARE_DEFAULT",
            model: "@cf/meta/llama-3.1-8b-instruct",
            provider: {
              id: "CLOUDFLARE_DEFAULT",
              name: "Cloudflare Workers AI",
              provider_type: "cloudflare",
              api_key: cloudApiKey.trim(),
              account_id: acc,
              base_url: base,
              model: "@cf/meta/llama-3.1-8b-instruct",
            },
          }
        );
      } else {
        // OpenAPI / Custom
        const provId =
          openapiTemplate === "groq"
            ? "GROQ_DEFAULT"
            : openapiTemplate === "google"
            ? "GOOGLE_CUSTOM"
            : "OPENAI_CUSTOM";
        const provName =
          openapiTemplate === "groq"
            ? "Groq Cloud"
            : openapiTemplate === "google"
            ? "Google AI Studio (Gemini)"
            : openapiTemplate === "openai"
            ? "OpenAI"
            : "Custom OpenAPI";

        await onFinish(
          {
            engine_mode: "cloud",
            local_engine: "whisper_cpu",
            local_model_size: "base",
            api_base_url: cloudBaseUrl.trim() || "https://api.openai.com/v1",
            api_key: cloudApiKey.trim(),
            model: cloudModel.trim() || "whisper-1",
            provider_id: provId,
            wizard_completed: true,
          },
          {
            enabled: true,
            active_provider_id: provId,
            model: cloudLlmModel.trim(),
            provider: {
              id: provId,
              name: provName,
              provider_type: "custom",
              api_key: cloudApiKey.trim(),
              account_id: "",
              base_url: cloudBaseUrl.trim() || "https://api.openai.com/v1",
              model: cloudLlmModel.trim(),
            },
          }
        );
      }
    }

    onClose();
    showToast("✓ Setup complete! Welcome to Lipi.");
  }

  if (!isOpen) return null;

  const totalRam = specs?.total_ram_gb ?? 8.0;
  const isHighRam = totalRam >= 15.0;
  const isMediumRam = totalRam >= 7.5;

  return (
    <div className="wizard-backdrop">
      <div className="wizard-container" role="dialog" aria-modal="true" aria-labelledby="wizard-title">
        {/* Wizard Top Header */}
        <div className="wizard-header">
          <div className="wizard-brand">
            <span className="wizard-icon">🎙️</span>
            <div>
              <h2 id="wizard-title" className="wizard-title">
                Lipi Setup Wizard
              </h2>
              <p className="wizard-subtitle">Private Voice Transcription & AI Assistant</p>
            </div>
          </div>
          <button
            type="button"
            className="wizard-btn-skip"
            onClick={handleSkip}
            title="Skip wizard and use defaults"
          >
            Skip Setup for Now ✕
          </button>
        </div>

        {/* Wizard Stepper Tabs - 5 Modular Steps */}
        <div className="wizard-stepper">
          <button
            type="button"
            className={`wizard-step-pill ${step === 1 ? "active" : ""} ${step > 1 ? "completed" : ""}`}
            onClick={() => setStep(1)}
          >
            <span className="step-num">1</span> Specs & Mode
          </button>
          <div className="step-divider" />
          <button
            type="button"
            className={`wizard-step-pill ${step === 2 ? "active" : ""} ${step > 2 ? "completed" : ""}`}
            onClick={() => setStep(2)}
          >
            <span className="step-num">2</span> ASR Model
          </button>
          <div className="step-divider" />
          <button
            type="button"
            className={`wizard-step-pill ${step === 3 ? "active" : ""} ${step > 3 ? "completed" : ""}`}
            onClick={() => setStep(3)}
          >
            <span className="step-num">3</span> AI Assistant
          </button>
          <div className="step-divider" />
          <button
            type="button"
            className={`wizard-step-pill ${step === 4 ? "active" : ""} ${step > 4 ? "completed" : ""}`}
            onClick={() => {
              setStep(4);
              fetchPrereqs();
            }}
          >
            <span className="step-num">4</span> Prerequisites
          </button>
          <div className="step-divider" />
          <button
            type="button"
            className={`wizard-step-pill ${step === 5 ? "active" : ""}`}
            onClick={() => setStep(5)}
          >
            <span className="step-num">5</span> Ready
          </button>
        </div>

        {/* Wizard Step Body */}
        <div className="wizard-body">
          {/* STEP 1: Specs & Mode Selection */}
          {step === 1 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">Hardware Assessment & Setup Mode</h3>
              <p className="wizard-step-desc">
                Lipi checks your machine specifications to suggest whether an offline local model or cloud API
                is ideal for your workstation.
              </p>

              {isLoadingSpecs ? (
                <div className="wizard-loading-card">
                  <span className="spinner-dot" /> Detecting hardware specs...
                </div>
              ) : specs ? (
                <>
                  <div className="wizard-specs-grid">
                    <div className="wizard-spec-card">
                      <span className="spec-icon" title="System Memory">
                        <svg className="spec-svg-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                          <path d="M6 19v-3" />
                          <path d="M10 19v-3" />
                          <path d="M14 19v-3" />
                          <path d="M18 19v-3" />
                          <rect x="2" y="5" width="20" height="11" rx="1" />
                          <path d="M6 9h.01" />
                          <path d="M10 9h.01" />
                          <path d="M14 9h.01" />
                          <path d="M18 9h.01" />
                        </svg>
                      </span>
                      <div className="spec-label">System RAM</div>
                      <div className="spec-value">{specs.total_ram_gb} GB</div>
                    </div>
                    <div className="wizard-spec-card">
                      <span className="spec-icon" title="Processor">
                        <svg className="spec-svg-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                          <rect x="4" y="4" width="16" height="16" rx="2" />
                          <rect x="9" y="9" width="6" height="6" />
                          <path d="M9 1v3" />
                          <path d="M15 1v3" />
                          <path d="M9 20v3" />
                          <path d="M15 20v3" />
                          <path d="M20 9h3" />
                          <path d="M20 14h3" />
                          <path d="M1 9h3" />
                          <path d="M1 14h3" />
                        </svg>
                      </span>
                      <div className="spec-label">CPU Cores</div>
                      <div className="spec-value">{specs.cpu_cores} Cores</div>
                    </div>
                    <div className="wizard-spec-card">
                      <span className="spec-icon" title="Platform OS">
                        <svg className="spec-svg-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                          <rect x="2" y="3" width="20" height="14" rx="2" />
                          <line x1="8" y1="21" x2="16" y2="21" />
                          <line x1="12" y1="17" x2="12" y2="21" />
                        </svg>
                      </span>
                      <div className="spec-label">Platform</div>
                      <div className="spec-value">{specs.os.toUpperCase()}</div>
                    </div>
                  </div>

                  <div className={`wizard-recommendation-box ${isHighRam ? "rec-high" : isMediumRam ? "rec-med" : "rec-low"}`}>
                    <div className="rec-header">
                      <span className="rec-badge">
                        {isHighRam ? "🌟 High Performance Machine" : isMediumRam ? "⚡ Balanced Hardware" : "☁️ Cloud Optimized"}
                      </span>
                      <span className="rec-suggested-mode">
                        Recommendation: {specs.recommended_mode === "local" ? "🏠 100% Local Setup" : "☁️ Fast Cloud / Hybrid"}
                      </span>
                    </div>
                    <p className="rec-text">{specs.summary_text}</p>
                  </div>
                </>
              ) : (
                <div className="wizard-loading-card">Standard system specs detected (~8.0 GB RAM)</div>
              )}

              {/* Mode Selection Cards right underneath specs */}
              <div style={{ marginTop: "20px" }}>
                <h4 className="wizard-subheading" style={{ marginBottom: "10px" }}>
                  Select Your Preferred Setup Mode
                </h4>
                <div className="wizard-path-selector">
                  <label
                    className={`wizard-path-card ${setupPath === "local" ? "selected" : ""}`}
                    onClick={() => setSetupPath("local")}
                  >
                    <input
                      type="radio"
                      name="setup_path"
                      checked={setupPath === "local"}
                      onChange={() => setSetupPath("local")}
                    />
                    <div>
                      <div className="path-title">
                        <span>🏠 100% Local (Private & Offline)</span>
                        {isMediumRam && <span className="path-pill-rec">Recommended</span>}
                      </div>
                      <div className="path-desc">
                        Zero audio or notes leave your machine. Runs local ASR models and optional local Ollama LLMs completely offline.
                      </div>
                    </div>
                  </label>

                  <label
                    className={`wizard-path-card ${setupPath === "cloud" ? "selected" : ""}`}
                    onClick={() => setSetupPath("cloud")}
                  >
                    <input
                      type="radio"
                      name="setup_path"
                      checked={setupPath === "cloud"}
                      onChange={() => setSetupPath("cloud")}
                    />
                    <div>
                      <div className="path-title">
                        <span>☁️ Cloud / Hybrid (Fast & Free APIs)</span>
                        {!isMediumRam && <span className="path-pill-rec">Recommended</span>}
                      </div>
                      <div className="path-desc">
                        Uses generous free tiers from Groq, Cloudflare, or Google AI Studio. Lightning-fast response with zero local RAM impact.
                      </div>
                    </div>
                  </label>
                </div>
              </div>

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={handleSkip}>
                  Skip Setup
                </button>
                <button type="button" className="btn btn-primary" onClick={() => setStep(2)}>
                  Next: Configure ASR Model →
                </button>
              </div>
            </div>
          )}

          {/* STEP 2: Speech-to-Text / ASR Model Settings */}
          {step === 2 && (
            <div className="wizard-step-content">
              {setupPath === "local" ? (
                <>
                  <h3 className="wizard-step-heading">Local ASR Model</h3>
                  <p className="wizard-step-desc">
                    Configure your on-device Automatic Speech Recognition (ASR) engine and model weights.
                  </p>

                  <div className="wizard-subpanel">
                    <h4 className="wizard-subheading">Model Size & Precision</h4>
                    <div className="wizard-model-sizes-grid">
                      <label className={`wizard-size-card ${localWhisperSize === "tiny" ? "active" : ""}`}>
                        <input
                          type="radio"
                          name="whisper_size"
                          checked={localWhisperSize === "tiny"}
                          onChange={() => setLocalWhisperSize("tiny")}
                        />
                        <div className="size-title">Tiny (~75 MB)</div>
                        <div className="size-desc">Fastest transcription, minimal RAM (~300 MB). Good for quick voice memos.</div>
                      </label>

                      <label className={`wizard-size-card ${localWhisperSize === "base" ? "active" : ""}`}>
                        <input
                          type="radio"
                          name="whisper_size"
                          checked={localWhisperSize === "base"}
                          onChange={() => setLocalWhisperSize("base")}
                        />
                        <div className="size-title">
                          Base (~142 MB) <span className="tag-recommended">Recommended</span>
                        </div>
                        <div className="size-desc">Balanced accuracy and speed (~500 MB RAM). Ideal for everyday dictation.</div>
                      </label>

                      <label className={`wizard-size-card ${localWhisperSize === "small" ? "active" : ""}`}>
                        <input
                          type="radio"
                          name="whisper_size"
                          checked={localWhisperSize === "small"}
                          onChange={() => setLocalWhisperSize("small")}
                        />
                        <div className="size-title">Small (~466 MB)</div>
                        <div className="size-desc">High accuracy for accents and specialized technical terms (~1 GB RAM).</div>
                      </label>
                    </div>

                    <div className="wizard-engine-runner-row" style={{ marginTop: "14px" }}>
                      <span className="engine-runner-label">ASR Runner:</span>
                      <label className="radio-label">
                        <input
                          type="radio"
                          name="whisper_engine"
                          checked={localWhisperEngine === "whisper_cpu"}
                          onChange={() => setLocalWhisperEngine("whisper_cpu")}
                        />
                        <span>whisper.cpp (Native C++)</span>
                      </label>
                      <label className="radio-label">
                        <input
                          type="radio"
                          name="whisper_engine"
                          checked={localWhisperEngine === "faster_whisper"}
                          onChange={() => setLocalWhisperEngine("faster_whisper")}
                        />
                        <span>faster-whisper (Python Virtualenv)</span>
                      </label>
                    </div>

                    {/* Model weights status & download */}
                    <div className="wizard-download-status-card" style={{ marginTop: "16px" }}>
                      <div className="status-left">
                        <span className="status-icon">
                          {modelStatus?.installed && modelStatus.model_size === localWhisperSize ? "✓" : "⬇️"}
                        </span>
                        <div>
                          <div className="status-main-label">
                            {modelStatus?.installed && modelStatus.model_size === localWhisperSize
                              ? `Local weights (${localWhisperSize}) are installed and ready`
                              : `Local weights (${localWhisperSize}) not downloaded yet`}
                          </div>
                          <div className="form-hint">
                            Official weights are downloaded once from HuggingFace and run completely offline.
                          </div>
                        </div>
                      </div>

                      <div>
                        {isDownloading ? (
                          <div className="wizard-downloading-box">
                            <div className="downloading-info">
                              <span>Downloading {localWhisperSize} weights... {downloadProgress ?? 0}%</span>
                              <span className="downloading-stats">
                                {downloadSpeedBps > 0 && formatSpeed(downloadSpeedBps)}
                                {downloadEtaSecs > 0 && ` · ${formatEta(downloadEtaSecs)}`}
                              </span>
                            </div>
                            <div className="progress-bar-bg">
                              <div
                                className="progress-bar-fill"
                                style={{ width: `${Math.min(downloadProgress ?? 0, 100)}%` }}
                              />
                            </div>
                          </div>
                        ) : (
                          <button
                            type="button"
                            className="btn btn-secondary"
                            onClick={() => onDownloadModel(localWhisperEngine, localWhisperSize)}
                          >
                            {modelStatus?.installed && modelStatus.model_size === localWhisperSize
                              ? "Re-download Weights"
                              : "Download Weights Now"}
                          </button>
                        )}
                      </div>
                    </div>
                  </div>
                </>
              ) : (
                /* Cloud ASR Setup */
                <>
                  <h3 className="wizard-step-heading">Cloud ASR Configuration</h3>
                  <p className="wizard-step-desc">
                    Choose an online provider for fast, cloud-hosted speech transcription.
                  </p>

                  <div className="wizard-subpanel">
                    <h4 className="wizard-subheading">Choose Provider</h4>
                    <div className="wizard-provider-tabs">
                      <button
                        type="button"
                        className={`wizard-prov-btn ${cloudCategory === "openapi" && openapiTemplate === "groq" ? "active" : ""}`}
                        onClick={() => {
                          setCloudCategory("openapi");
                          handleSelectOpenApiTemplate("groq");
                        }}
                      >
                        <span className="prov-btn-title">⚡ Groq Cloud</span>
                        <span className="prov-btn-sub">Ultra-fast free tier</span>
                      </button>

                      <button
                        type="button"
                        className={`wizard-prov-btn ${cloudCategory === "openapi" && openapiTemplate === "google" ? "active" : ""}`}
                        onClick={() => {
                          setCloudCategory("openapi");
                          handleSelectOpenApiTemplate("google");
                        }}
                      >
                        <span className="prov-btn-title">✨ Google AI Studio</span>
                        <span className="prov-btn-sub">Generous free tier</span>
                      </button>

                      <button
                        type="button"
                        className={`wizard-prov-btn ${cloudCategory === "cloudflare" ? "active" : ""}`}
                        onClick={() => setCloudCategory("cloudflare")}
                      >
                        <span className="prov-btn-title">🛡️ Cloudflare AI</span>
                        <span className="prov-btn-sub">10k neurons/day</span>
                      </button>

                      <button
                        type="button"
                        className={`wizard-prov-btn ${cloudCategory === "openapi" && (openapiTemplate === "openai" || openapiTemplate === "custom") ? "active" : ""}`}
                        onClick={() => {
                          setCloudCategory("openapi");
                          handleSelectOpenApiTemplate("custom");
                        }}
                      >
                        <span className="prov-btn-title">🔧 Custom / OpenAI</span>
                        <span className="prov-btn-sub">OpenAI-compatible</span>
                      </button>
                    </div>

                    {cloudCategory === "cloudflare" ? (
                      <div className="wizard-provider-form">
                        <div className="form-info-banner">
                          <span>🛡️ Cloudflare Workers AI offers 10,000 neurons/day free tier.</span>
                          <a
                            href="https://dash.cloudflare.com"
                            target="_blank"
                            rel="noreferrer"
                            className="banner-link"
                          >
                            Get Account ID & Token ↗
                          </a>
                        </div>

                        <div className="form-group">
                          <label className="form-label">Cloudflare Account ID:</label>
                          <input
                            type="text"
                            className="form-input"
                            placeholder="e.g. c3a0b12984fe721..."
                            value={cloudAccountId}
                            onChange={(e) => setCloudAccountId(e.target.value)}
                          />
                        </div>

                        <div className="form-group">
                          <label className="form-label">Cloudflare API Token:</label>
                          <input
                            type="password"
                            className="form-input"
                            placeholder="Workers AI API Token"
                            value={cloudApiKey}
                            onChange={(e) => setCloudApiKey(e.target.value)}
                          />
                        </div>
                      </div>
                    ) : (
                      <div className="wizard-provider-form">
                        <div className="form-info-banner">
                          {openapiTemplate === "groq" && (
                            <>
                              <span>⚡ Groq offers ultra-fast whisper transcription with generous free rate limits.</span>
                              <a href="https://console.groq.com/keys" target="_blank" rel="noreferrer" className="banner-link">
                                Get Free Groq Key ↗
                              </a>
                            </>
                          )}
                          {openapiTemplate === "google" && (
                            <>
                              <span>✨ Google AI Studio provides API keys with generous quotas via OpenAI-compatible endpoints.</span>
                              <a href="https://aistudio.google.com/app/apikey" target="_blank" rel="noreferrer" className="banner-link">
                                Get Free Google Key ↗
                              </a>
                            </>
                          )}
                          {openapiTemplate === "custom" && (
                            <span>🔧 Standard OpenAPI / OpenAI-compatible endpoint. Defaults to api.openai.com.</span>
                          )}
                        </div>

                        <div className="form-group">
                          <label className="form-label">API Base URL:</label>
                          <input
                            type="text"
                            className="form-input"
                            value={cloudBaseUrl}
                            onChange={(e) => setCloudBaseUrl(e.target.value)}
                          />
                        </div>

                        <div className="form-group">
                          <label className="form-label">API Key:</label>
                          <div style={{ display: "flex", gap: "8px" }}>
                            <input
                              type="password"
                              className="form-input"
                              placeholder="Enter API key"
                              value={cloudApiKey}
                              onChange={(e) => setCloudApiKey(e.target.value)}
                            />
                            <button
                              type="button"
                              className="btn btn-secondary"
                              onClick={handleFetchCloudModels}
                              disabled={isFetchingCloudModels || !cloudApiKey.trim()}
                            >
                              {isFetchingCloudModels ? "Testing..." : "Test & Fetch Models"}
                            </button>
                          </div>
                        </div>

                        {cloudModelFetchSuccess && (
                          <div className="wizard-fetch-success">{cloudModelFetchSuccess}</div>
                        )}
                        {cloudModelFetchError && (
                          <div className="wizard-fetch-error">⚠️ {cloudModelFetchError}</div>
                        )}

                        <div className="form-group" style={{ marginTop: "12px" }}>
                          <label className="form-label">ASR Speech Model:</label>
                          <input
                            type="text"
                            className="form-input"
                            value={cloudModel}
                            onChange={(e) => setCloudModel(e.target.value)}
                            list="wizard-cloud-asr-models"
                          />
                          <datalist id="wizard-cloud-asr-models">
                            {cloudAvailableModels.filter((m) => m.toLowerCase().includes("whisper")).map((m) => (
                              <option key={m} value={m} />
                            ))}
                          </datalist>
                        </div>
                      </div>
                    )}
                  </div>
                </>
              )}

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={() => setStep(1)}>
                  ← Back to Specs
                </button>
                <div style={{ display: "flex", gap: "10px" }}>
                  <button type="button" className="btn btn-secondary" onClick={handleSkip}>
                    Skip Setup
                  </button>
                  <button type="button" className="btn btn-primary" onClick={() => setStep(3)}>
                    Next: AI Assistant Settings →
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* STEP 3: AI Assistant / LLM Settings */}
          {step === 3 && (
            <div className="wizard-step-content">
              {setupPath === "local" ? (
                <>
                  <h3 className="wizard-step-heading">Local AI Assistant (LLM)</h3>
                  <p className="wizard-step-desc">
                    Enhance your raw transcriptions with local grammar correction, formatting, or summarization via Ollama.
                  </p>

                  <div className="wizard-subpanel">
                    <div className="wizard-llm-choice-row">
                      <label className="radio-label">
                        <input
                          type="radio"
                          name="local_llm_mode"
                          checked={localLlmMode === "ollama"}
                          onChange={() => setLocalLlmMode("ollama")}
                        />
                        <span>Enable Ollama (Local AI Models)</span>
                      </label>
                      <label className="radio-label">
                        <input
                          type="radio"
                          name="local_llm_mode"
                          checked={localLlmMode === "none"}
                          onChange={() => setLocalLlmMode("none")}
                        />
                        <span>Pure Transcription Only (No AI Edits)</span>
                      </label>
                    </div>

                    {localLlmMode === "ollama" && (
                      <div className="wizard-ollama-box" style={{ marginTop: "14px" }}>
                        <div className="ollama-status-row">
                          <span className={`ollama-dot ${ollamaAvailable ? "dot-online" : "dot-offline"}`} />
                          <span className="ollama-status-label">
                            {isCheckingOllama
                              ? "Checking Ollama on localhost:11434..."
                              : ollamaAvailable
                              ? "Ollama service connected (localhost:11434)"
                              : "Ollama not running on localhost:11434 (start Ollama or pick model)"}
                          </span>
                          <button
                            type="button"
                            className="btn-text-action"
                            onClick={checkOllamaStatus}
                            title="Refresh Ollama status"
                          >
                            ↻ Check
                          </button>
                        </div>

                        <div className="form-group" style={{ marginTop: "14px" }}>
                          <label className="form-label">Select Ollama Model:</label>
                          <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
                            <input
                              type="text"
                              className="form-input"
                              value={selectedOllamaModel}
                              onChange={(e) => setSelectedOllamaModel(e.target.value)}
                              placeholder="e.g. llama3.2, mistral, gemma2:2b"
                              list="wizard-ollama-suggestions"
                            />
                            <datalist id="wizard-ollama-suggestions">
                              {ollamaModels.map((m) => (
                                <option key={m} value={m} />
                              ))}
                            </datalist>
                          </div>

                          {/* Dynamic RAM feedback based on chosen model and actual machine RAM */}
                          {selectedOllamaModel.trim() && (
                            <div className="wizard-model-ram-feedback">
                              <span className="ram-badge-item">
                                {getModelRamFeedback(selectedOllamaModel, totalRam).label}
                              </span>
                            </div>
                          )}

                          {ollamaModels.length > 0 ? (
                            <div className="wizard-quick-model-pills">
                              <span style={{ fontSize: "12px", color: "var(--text-muted)" }}>Available in Ollama:</span>
                              {ollamaModels.slice(0, 8).map((m) => (
                                <button
                                  key={m}
                                  type="button"
                                  className={`model-pill ${selectedOllamaModel === m ? "active" : ""}`}
                                  onClick={() => setSelectedOllamaModel(m)}
                                >
                                  {m}
                                </button>
                              ))}
                            </div>
                          ) : (
                            <div className="wizard-quick-model-pills">
                              <span style={{ fontSize: "12px", color: "var(--text-muted)" }}>Popular models:</span>
                              {["llama3.2", "mistral", "gemma2:2b", "qwen2.5:3b"].map((m) => (
                                <button
                                  key={m}
                                  type="button"
                                  className={`model-pill ${selectedOllamaModel === m ? "active" : ""}`}
                                  onClick={() => setSelectedOllamaModel(m)}
                                >
                                  {m}
                                </button>
                              ))}
                            </div>
                          )}
                        </div>
                      </div>
                    )}
                  </div>
                </>
              ) : (
                /* Cloud LLM Setup */
                <>
                  <h3 className="wizard-step-heading">Cloud AI Assistant (LLM)</h3>
                  <p className="wizard-step-desc">
                    Choose which cloud LLM will refine, proofread, and summarize your voice dictations.
                  </p>

                  <div className="wizard-subpanel">
                    <div className="form-group">
                      <label className="form-label">Cloud AI Model:</label>
                      <input
                        type="text"
                        className="form-input"
                        value={cloudLlmModel}
                        onChange={(e) => setCloudLlmModel(e.target.value)}
                        placeholder="e.g. llama-3.3-70b-versatile, gemini-2.5-flash"
                        list="wizard-cloud-llm-models"
                      />
                      <datalist id="wizard-cloud-llm-models">
                        {cloudAvailableModels.map((m) => (
                          <option key={m} value={m} />
                        ))}
                      </datalist>
                    </div>

                    <div className="wizard-quick-model-pills" style={{ marginTop: "12px" }}>
                      <span style={{ fontSize: "12px", color: "var(--text-muted)" }}>Quick presets:</span>
                      {(openapiTemplate === "groq"
                        ? ["llama-3.3-70b-versatile", "llama-3.1-8b-instant"]
                        : openapiTemplate === "google"
                        ? ["gemini-2.5-flash", "gemini-2.0-flash"]
                        : ["gpt-4o-mini", "llama-3.1-8b-instruct"]
                      ).map((m) => (
                        <button
                          key={m}
                          type="button"
                          className={`model-pill ${cloudLlmModel === m ? "active" : ""}`}
                          onClick={() => setCloudLlmModel(m)}
                        >
                          {m}
                        </button>
                      ))}
                    </div>
                  </div>
                </>
              )}

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={() => setStep(2)}>
                  ← Back to ASR
                </button>
                <div style={{ display: "flex", gap: "10px" }}>
                  <button type="button" className="btn btn-secondary" onClick={handleSkip}>
                    Skip Setup
                  </button>
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={() => {
                      setStep(4);
                      fetchPrereqs();
                    }}
                  >
                    Next: Check Prerequisites →
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* STEP 4: System Readiness & Prerequisites */}
          {step === 4 && (
            <div className="wizard-step-content">
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", flexWrap: "wrap", gap: "10px" }}>
                <div>
                  <h3 className="wizard-step-heading">System Readiness & Prerequisites</h3>
                  <p className="wizard-step-desc">
                    Lipi verifies hardware, audio input, runner binaries, and offline weights before you begin.
                  </p>
                </div>
                {/* Windows vs Linux Switcher Tabs */}
                <div className="wizard-os-toggle-group">
                  <button
                    type="button"
                    className={`wizard-os-pill ${activeOsTab === "linux" ? "active" : ""}`}
                    onClick={() => setActiveOsTab("linux")}
                  >
                    🐧 Linux
                  </button>
                  <button
                    type="button"
                    className={`wizard-os-pill ${activeOsTab === "windows" ? "active" : ""}`}
                    onClick={() => setActiveOsTab("windows")}
                  >
                    🪟 Windows
                  </button>
                </div>
              </div>

              {/* Prerequisites Checklist Cards */}
              <div className="wizard-prereq-list">
                {/* 1. Microphone Input Device */}
                <div className="wizard-prereq-card">
                  <div className="prereq-left">
                    <span className="prereq-icon">
                      {prereqs?.has_audio_device ? "🎙️" : "⚠️"}
                    </span>
                    <div>
                      <div className="prereq-title">
                        Microphone Audio Input
                        <span className={`prereq-status-badge ${prereqs?.has_audio_device ? "status-ready" : "status-warn"}`}>
                          {prereqs?.has_audio_device ? "Ready" : "Not Found"}
                        </span>
                      </div>
                      <div className="prereq-desc">
                        {prereqs?.has_audio_device
                          ? `Device: ${prereqs.audio_device_name || "Default System Input"}`
                          : "No default audio recording device detected. Ensure microphone is connected and permissions granted."}
                      </div>
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={fetchPrereqs}
                    disabled={isLoadingPrereqs}
                  >
                    {isLoadingPrereqs ? "Checking..." : "↻ Re-check"}
                  </button>
                </div>

                {/* 2. Local ASR Runner Engine (Local mode only) */}
                {setupPath === "local" && (
                  <div className="wizard-prereq-card">
                    <div className="prereq-left">
                      <span className="prereq-icon">
                        {localWhisperEngine === "whisper_cpu"
                          ? prereqs?.whisper_binary_found ? "⚙️" : "📦"
                          : prereqs?.venv_python_found ? "🐍" : "📦"}
                      </span>
                      <div>
                        <div className="prereq-title">
                          ASR Engine Runner ({localWhisperEngine === "whisper_cpu" ? "whisper.cpp" : "faster-whisper"})
                          <span
                            className={`prereq-status-badge ${
                              (localWhisperEngine === "whisper_cpu" && prereqs?.whisper_binary_found) ||
                              (localWhisperEngine === "faster_whisper" && prereqs?.venv_python_found)
                                ? "status-ready"
                                : "status-action"
                            }`}
                          >
                            {(localWhisperEngine === "whisper_cpu" && prereqs?.whisper_binary_found) ||
                            (localWhisperEngine === "faster_whisper" && prereqs?.venv_python_found)
                              ? "Ready"
                              : "Setup Needed"}
                          </span>
                        </div>
                        <div className="prereq-desc">
                          {localWhisperEngine === "whisper_cpu"
                            ? prereqs?.whisper_binary_found
                              ? `whisper-cli executable ready at ${prereqs.whisper_binary_path}`
                              : "Native whisper-cli runner binary not installed in Lipi bin directory."
                            : prereqs?.venv_python_found
                            ? "faster-whisper Python virtual environment is installed and active."
                            : "faster-whisper module not installed in application virtualenv."}
                        </div>
                        {engineSetupMsg && (
                          <div className="form-hint" style={{ marginTop: "4px", color: "var(--accent)" }}>
                            {engineSetupMsg}
                          </div>
                        )}
                      </div>
                    </div>

                    <div>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={handleSetupRunner}
                        disabled={isSettingUpEngine}
                      >
                        {isSettingUpEngine ? "Setting up..." : "⚡ 1-Click Setup Runner"}
                      </button>
                    </div>
                  </div>
                )}

                {/* 3. ASR Model Weights (Local mode only) */}
                {setupPath === "local" && (
                  <div className="wizard-prereq-card">
                    <div className="prereq-left">
                      <span className="prereq-icon">
                        {modelStatus?.installed && modelStatus.model_size === localWhisperSize ? "💾" : "⬇️"}
                      </span>
                      <div>
                        <div className="prereq-title">
                          Model Weights ({localWhisperSize.toUpperCase()})
                          <span
                            className={`prereq-status-badge ${
                              modelStatus?.installed && modelStatus.model_size === localWhisperSize
                                ? "status-ready"
                                : "status-action"
                            }`}
                          >
                            {modelStatus?.installed && modelStatus.model_size === localWhisperSize
                              ? "Installed"
                              : "Download Needed"}
                          </span>
                        </div>
                        <div className="prereq-desc">
                          {modelStatus?.installed && modelStatus.model_size === localWhisperSize
                            ? "Model weights are present on disk and ready for low-latency offline dictation."
                            : `Weights for '${localWhisperSize}' have not been downloaded yet.`}
                        </div>
                      </div>
                    </div>

                    <div>
                      {isDownloading ? (
                        <div className="wizard-downloading-box">
                          <div className="downloading-info">
                            <span>Downloading {localWhisperSize}... {downloadProgress ?? 0}%</span>
                          </div>
                          <div className="progress-bar-bg">
                            <div
                              className="progress-bar-fill"
                              style={{ width: `${Math.min(downloadProgress ?? 0, 100)}%` }}
                            />
                          </div>
                        </div>
                      ) : (
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm"
                          onClick={() => onDownloadModel(localWhisperEngine, localWhisperSize)}
                        >
                          {modelStatus?.installed && modelStatus.model_size === localWhisperSize
                            ? "Re-download"
                            : "⬇️ Download Weights"}
                        </button>
                      )}
                    </div>
                  </div>
                )}

                {/* 4. Local AI Assistant (Ollama) */}
                {setupPath === "local" && localLlmMode === "ollama" && (
                  <div className="wizard-prereq-card">
                    <div className="prereq-left">
                      <span className="prereq-icon">🦙</span>
                      <div>
                        <div className="prereq-title">
                          Ollama LLM Service (localhost:11434)
                          <span
                            className={`prereq-status-badge ${
                              ollamaAvailable ? "status-ready" : "status-warn"
                            }`}
                          >
                            {ollamaAvailable ? "Online" : "Offline"}
                          </span>
                        </div>
                        <div className="prereq-desc">
                          {ollamaAvailable
                            ? `Connected with ${ollamaModels.length} models installed. Selected: ${selectedOllamaModel}`
                            : "Ollama is not running. Launch Ollama app or start daemon via terminal."}
                        </div>
                      </div>
                    </div>

                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={checkOllamaStatus}
                      disabled={isCheckingOllama}
                    >
                      {isCheckingOllama ? "Testing..." : "↻ Re-test"}
                    </button>
                  </div>
                )}

                {/* Cloud Mode API Verification */}
                {setupPath === "cloud" && (
                  <div className="wizard-prereq-card">
                    <div className="prereq-left">
                      <span className="prereq-icon">☁️</span>
                      <div>
                        <div className="prereq-title">
                          Cloud API Connection
                          <span
                            className={`prereq-status-badge ${
                              cloudApiKey.trim() ? "status-ready" : "status-warn"
                            }`}
                          >
                            {cloudApiKey.trim() ? "Key Provided" : "Key Missing"}
                          </span>
                        </div>
                        <div className="prereq-desc">
                          {cloudApiKey.trim()
                            ? `Configured for ${cloudBaseUrl}`
                            : "Return to Step 2 to enter your API key or token."}
                        </div>
                      </div>
                    </div>
                  </div>
                )}
              </div>

              {/* OS Specific Setup Instructions Box */}
              <div className="wizard-os-guide-box">
                <div className="os-guide-header">
                  <span>💡 {activeOsTab === "linux" ? "Linux Setup Guide (Debian / Ubuntu / Fedora / Arch)" : "Windows Setup Guide (Windows 10 / 11)"}</span>
                </div>
                {activeOsTab === "linux" ? (
                  <div className="os-guide-body">
                    <p style={{ margin: "0 0 6px 0", fontSize: "12px", color: "var(--text-secondary)" }}>
                      If runners or audio libraries are missing on Linux, install required packages via terminal:
                    </p>
                    <code className="os-guide-cmd">
                      sudo apt update &amp;&amp; sudo apt install -y python3 python3-venv libasound2-dev curl
                    </code>
                    {localLlmMode === "ollama" && !ollamaAvailable && (
                      <p style={{ margin: "8px 0 0 0", fontSize: "12px", color: "var(--text-secondary)" }}>
                        Install and start Ollama: <code className="os-inline-code">curl -fsSL https://ollama.com/install.sh | sh &amp;&amp; ollama run {selectedOllamaModel || "llama3.2"}</code>
                      </p>
                    )}
                  </div>
                ) : (
                  <div className="os-guide-body">
                    <p style={{ margin: "0 0 6px 0", fontSize: "12px", color: "var(--text-secondary)" }}>
                      On Windows, Lipi can automatically download official whisper-cli.exe binaries or use winget:
                    </p>
                    <code className="os-guide-cmd">
                      winget install Python.Python.3.11 &amp; winget install Ollama.Ollama
                    </code>
                    <p style={{ margin: "8px 0 0 0", fontSize: "12px", color: "var(--text-secondary)" }}>
                      Ensure microphone permissions are enabled under <strong>Windows Settings → Privacy &amp; Security → Microphone</strong>.
                    </p>
                  </div>
                )}
              </div>

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={() => setStep(3)}>
                  ← Back to AI Assistant
                </button>
                <div style={{ display: "flex", gap: "10px" }}>
                  <button type="button" className="btn btn-secondary" onClick={handleSkip}>
                    Skip Setup
                  </button>
                  <button type="button" className="btn btn-primary" onClick={() => setStep(5)}>
                    Next: Review &amp; Start →
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* STEP 5: Ready to Dictate / Final Review */}
          {step === 5 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">Everything is configured!</h3>
              <p className="wizard-step-desc">
                Review your configuration below. You can always change any of these settings later in Settings.
              </p>

              <div className="wizard-summary-card">
                <div className="summary-row">
                  <span className="summary-label">Setup Mode:</span>
                  <span className="summary-value">{setupPath === "local" ? "🏠 100% Local (Offline)" : "☁️ Cloud / Hybrid"}</span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">Speech-to-Text (ASR):</span>
                  <span className="summary-value">
                    {setupPath === "local"
                      ? `Whisper ${localWhisperSize.toUpperCase()} (${localWhisperEngine === "whisper_cpu" ? "whisper.cpp" : "faster-whisper"})`
                      : cloudCategory === "cloudflare"
                      ? "Cloudflare Workers AI"
                      : `${cloudModel || "Configured model"} via ${cloudBaseUrl}`}
                  </span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">AI Transformation:</span>
                  <span className="summary-value">
                    {setupPath === "local"
                      ? localLlmMode === "ollama"
                        ? `Ollama (${selectedOllamaModel || "Active model"})`
                        : "Disabled (Transcription Only)"
                      : cloudCategory === "cloudflare"
                      ? "Cloudflare Workers AI"
                      : cloudLlmModel || "Enabled"}
                  </span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">Platform & Audio:</span>
                  <span className="summary-value">
                    {specs?.os.toUpperCase() || "DESKTOP"} · {prereqs?.audio_device_name ? "Microphone Detected" : "Default Audio"}
                  </span>
                </div>
              </div>

              {/* Cheatsheet for hotkeys */}
              <div className="wizard-cheatsheet-box">
                <h4 style={{ margin: "0 0 8px 0", fontSize: "13px", fontWeight: 600, color: "var(--text-primary)" }}>
                  💡 Helpful Shortcuts:
                </h4>
                <ul style={{ margin: 0, paddingLeft: "18px", fontSize: "13px", color: "var(--text-secondary)", lineHeight: 1.6 }}>
                  <li>
                    <kbd className="key-hint">Alt+R</kbd>: Toggle dictation recording on / off from anywhere.
                  </li>
                  <li>
                    <kbd className="key-hint">Alt+M</kbd>: Switch to compact transparent floating mini widget.
                  </li>
                  <li>
                    <kbd className="key-hint">Esc</kbd>: Exit Settings back to your notes scratchpad.
                  </li>
                </ul>
              </div>

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={() => setStep(4)}>
                  ← Back to Prerequisites
                </button>
                <button type="button" className="btn btn-primary" onClick={handleCompleteSetup}>
                  ✓ Complete Setup &amp; Start Dictating
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
