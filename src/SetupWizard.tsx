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
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [specs, setSpecs] = useState<SystemSpecs | null>(null);
  const [isLoadingSpecs, setIsLoadingSpecs] = useState(true);

  // Setup path
  const [setupPath, setSetupPath] = useState<"local" | "cloud">("local");

  // Local settings
  const [localWhisperSize, setLocalWhisperSize] = useState<"tiny" | "base" | "small">("base");
  const [localWhisperEngine, setLocalWhisperEngine] = useState<"whisper_cpu" | "faster_whisper">("whisper_cpu");
  const [localLlmMode, setLocalLlmMode] = useState<"ollama" | "none" | "cloud">("ollama");
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

  // Load specs on open
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
      })
      .catch((err) => {
        console.error("Failed to read system specs:", err);
      })
      .finally(() => {
        setIsLoadingSpecs(false);
      });

    checkOllamaStatus();
  }, [isOpen]);

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
        // Pick smart default if model isn't set
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
      const llmEnabled = localLlmMode !== "none";
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

        {/* Wizard Stepper Tabs */}
        <div className="wizard-stepper">
          <button
            type="button"
            className={`wizard-step-pill ${step === 1 ? "active" : ""} ${step > 1 ? "completed" : ""}`}
            onClick={() => setStep(1)}
          >
            <span className="step-num">1</span> Machine Specs
          </button>
          <div className="step-divider" />
          <button
            type="button"
            className={`wizard-step-pill ${step === 2 ? "active" : ""} ${step > 2 ? "completed" : ""}`}
            onClick={() => setStep(2)}
          >
            <span className="step-num">2</span> Configure Mode
          </button>
          <div className="step-divider" />
          <button
            type="button"
            className={`wizard-step-pill ${step === 3 ? "active" : ""}`}
            onClick={() => setStep(3)}
          >
            <span className="step-num">3</span> Ready to Dictate
          </button>
        </div>

        {/* Wizard Step Content */}
        <div className="wizard-body">
          {/* STEP 1: Specs & Greeting */}
          {step === 1 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">Checking your machine hardware...</h3>
              <p className="wizard-step-desc">
                Lipi automatically checks your CPU cores and available system memory to recommend the best setup for
                smooth, low-latency transcription.
              </p>

              {isLoadingSpecs ? (
                <div className="wizard-loading-card">
                  <span className="spinner-dot" /> Detecting hardware specs...
                </div>
              ) : specs ? (
                <>
                  <div className="wizard-specs-grid">
                    <div className="wizard-spec-card">
                      <span className="spec-icon">💾</span>
                      <div className="spec-label">System RAM</div>
                      <div className="spec-value">{specs.total_ram_gb} GB</div>
                    </div>
                    <div className="wizard-spec-card">
                      <span className="spec-icon">⚙️</span>
                      <div className="spec-label">CPU Cores</div>
                      <div className="spec-value">{specs.cpu_cores} Cores</div>
                    </div>
                    <div className="wizard-spec-card">
                      <span className="spec-icon">💻</span>
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
                        Recommendation: {specs.recommended_mode === "local" ? "🏠 Local Offline Setup" : "☁️ Fast Cloud / Hybrid"}
                      </span>
                    </div>
                    <p className="rec-text">{specs.summary_text}</p>
                  </div>
                </>
              ) : (
                <div className="wizard-loading-card">Standard system specs detected (~8.0 GB RAM)</div>
              )}

              <div className="wizard-footer-actions">
                <button type="button" className="btn btn-secondary" onClick={handleSkip}>
                  Skip Setup
                </button>
                <button type="button" className="btn btn-primary" onClick={() => setStep(2)}>
                  Next: Choose Setup Mode →
                </button>
              </div>
            </div>
          )}

          {/* STEP 2: Configure Path (Local vs Cloud) */}
          {step === 2 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">Choose your preferred setup</h3>
              <p className="wizard-step-desc">
                Select between 100% offline local transcription or fast, free online API providers.
              </p>

              {/* Path Toggle Cards */}
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
                      {isHighRam && <span className="path-pill-rec">Recommended</span>}
                    </div>
                    <div className="path-desc">
                      Zero audio or notes leave your machine. Runs local Whisper + local Ollama models.
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
                      Uses generous free tiers from Groq, Cloudflare, or Google AI Studio. Instant and zero local RAM cost.
                    </div>
                  </div>
                </label>
              </div>

              {/* OPTION A: Local Setup Details */}
              {setupPath === "local" && (
                <div className="wizard-subpanel">
                  <h4 className="wizard-subheading">1. Local Whisper Speech Model</h4>
                  <div className="wizard-model-sizes-grid">
                    <label className={`wizard-size-card ${localWhisperSize === "tiny" ? "active" : ""}`}>
                      <input
                        type="radio"
                        name="whisper_size"
                        checked={localWhisperSize === "tiny"}
                        onChange={() => setLocalWhisperSize("tiny")}
                      />
                      <div className="size-title">Tiny (~75 MB)</div>
                      <div className="size-desc">Fastest, minimal RAM (~300 MB). Good for quick voice memos.</div>
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
                      <div className="size-desc">Balanced accuracy and speed (~500 MB RAM). Ideal for daily use.</div>
                    </label>

                    <label className={`wizard-size-card ${localWhisperSize === "small" ? "active" : ""}`}>
                      <input
                        type="radio"
                        name="whisper_size"
                        checked={localWhisperSize === "small"}
                        onChange={() => setLocalWhisperSize("small")}
                      />
                      <div className="size-title">Small (~466 MB)</div>
                      <div className="size-desc">High accuracy for accents and technical terms (~1 GB RAM).</div>
                    </label>
                  </div>

                  <div className="wizard-engine-runner-row">
                    <span className="engine-runner-label">Runner:</span>
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
                      <span>faster-whisper (Python)</span>
                    </label>
                  </div>

                  {/* Model weights status & download */}
                  <div className="wizard-download-status-card">
                    <div className="status-left">
                      <span className="status-icon">
                        {modelStatus?.installed && modelStatus.model_size === localWhisperSize ? "✓" : "⬇️"}
                      </span>
                      <div>
                        <div className="status-main-label">
                          {modelStatus?.installed && modelStatus.model_size === localWhisperSize
                            ? `Whisper weights (${localWhisperSize}) installed and ready`
                            : `Whisper weights (${localWhisperSize}) not downloaded yet`}
                        </div>
                        <div className="form-hint">
                          Official whisper.cpp GGML model weights run natively on CPU / Vulkan.
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

                  {/* Local LLM (Ollama) */}
                  <div style={{ marginTop: "20px" }}>
                    <h4 className="wizard-subheading">2. Local AI Transformation (Ollama)</h4>
                    <p className="form-hint" style={{ marginBottom: "12px" }}>
                      Automatically format, copyedit, or summarize notes with a local LLM via Ollama.
                    </p>

                    <div className="wizard-llm-choice-row">
                      <label className="radio-label">
                        <input
                          type="radio"
                          name="local_llm_mode"
                          checked={localLlmMode === "ollama"}
                          onChange={() => setLocalLlmMode("ollama")}
                        />
                        <span>Enable Ollama (Local Models)</span>
                      </label>
                      <label className="radio-label">
                        <input
                          type="radio"
                          name="local_llm_mode"
                          checked={localLlmMode === "none"}
                          onChange={() => setLocalLlmMode("none")}
                        />
                        <span>Disable AI Edits (Pure Transcription Only)</span>
                      </label>
                    </div>

                    {localLlmMode === "ollama" && (
                      <div className="wizard-ollama-box">
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

                        <div className="form-group" style={{ marginTop: "12px" }}>
                          <label className="form-label">Select Ollama Model:</label>
                          <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
                            <input
                              type="text"
                              className="form-input"
                              value={selectedOllamaModel}
                              onChange={(e) => setSelectedOllamaModel(e.target.value)}
                              placeholder="Enter or select installed model name"
                              list="wizard-ollama-suggestions"
                            />
                            <datalist id="wizard-ollama-suggestions">
                              {ollamaModels.map((m) => (
                                <option key={m} value={m} />
                              ))}
                            </datalist>
                          </div>

                          {/* Dynamic RAM Feedback for the chosen model */}
                          {selectedOllamaModel.trim() && (
                            <div className="wizard-model-ram-feedback">
                              <span className="ram-badge-item">
                                {getModelRamFeedback(selectedOllamaModel, totalRam).label}
                              </span>
                            </div>
                          )}

                          {ollamaModels.length > 0 && (
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
                          )}
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* OPTION B: Cloud / OpenAPI Setup Details */}
              {setupPath === "cloud" && (
                <div className="wizard-subpanel">
                  <h4 className="wizard-subheading">Choose Online Provider</h4>
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
                      <span className="prov-btn-sub">10k neurons/day free</span>
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
                      <span className="prov-btn-sub">Compatible endpoint</span>
                    </button>
                  </div>

                  {/* Cloudflare Form */}
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
                    /* OpenAPI / Custom Form (Groq, Google AI Studio, Custom) */
                    <div className="wizard-provider-form">
                      <div className="form-info-banner">
                        {openapiTemplate === "groq" && (
                          <>
                            <span>⚡ Groq offers ultra-fast speech transcription and AI editing with generous free rate limits.</span>
                            <a href="https://console.groq.com/keys" target="_blank" rel="noreferrer" className="banner-link">
                              Get Free Groq Key ↗
                            </a>
                          </>
                        )}
                        {openapiTemplate === "google" && (
                          <>
                            <span>✨ Google AI Studio provides free API keys with generous quotas via standard OpenAI compatible endpoint.</span>
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

                      {/* Live Models Selection */}
                      <div className="wizard-model-row" style={{ marginTop: "12px", display: "grid", gridTemplateColumns: "1fr 1fr", gap: "12px" }}>
                        <div className="form-group">
                          <label className="form-label">Speech Model (ASR):</label>
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

                        <div className="form-group">
                          <label className="form-label">LLM Transform Model:</label>
                          <input
                            type="text"
                            className="form-input"
                            value={cloudLlmModel}
                            onChange={(e) => setCloudLlmModel(e.target.value)}
                            list="wizard-cloud-llm-models"
                          />
                          <datalist id="wizard-cloud-llm-models">
                            {cloudAvailableModels.map((m) => (
                              <option key={m} value={m} />
                            ))}
                          </datalist>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
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
                    Next: Review & Start →
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* STEP 3: Review & Finish */}
          {step === 3 && (
            <div className="wizard-step-content">
              <h3 className="wizard-step-heading">Everything is configured!</h3>
              <p className="wizard-step-desc">
                Review your configuration below. You can always change any of these settings later.
              </p>

              <div className="wizard-summary-card">
                <div className="summary-row">
                  <span className="summary-label">Mode:</span>
                  <span className="summary-value">{setupPath === "local" ? "🏠 Local (Offline)" : "☁️ Cloud / Hybrid"}</span>
                </div>
                <div className="summary-row">
                  <span className="summary-label">Speech-to-Text:</span>
                  <span className="summary-value">
                    {setupPath === "local"
                      ? `Whisper ${localWhisperSize.toUpperCase()} (${localWhisperEngine === "whisper_cpu" ? "whisper.cpp" : "Faster Whisper"})`
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
                        : "Disabled"
                      : cloudCategory === "cloudflare"
                      ? "Cloudflare Workers AI"
                      : cloudLlmModel || "Enabled"}
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
                <button type="button" className="btn btn-secondary" onClick={() => setStep(2)}>
                  ← Back
                </button>
                <button type="button" className="btn btn-primary" onClick={handleCompleteSetup}>
                  ✓ Complete Setup & Start Dictating
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
