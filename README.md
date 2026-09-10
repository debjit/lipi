# Lipi (লিপি)

> **Lipi** (লিপি) means *"script"* or *"writing"* in Bengali.

Lipi is a lightweight, local-first desktop speech-to-text and voice note application. Built with **Tauri v2**, **Rust**, and **React 19**, it lets you record microphone audio, transcribe speech with either cloud AI providers or 100% offline local Whisper engines, and manage your notes seamlessly.

---

## ✨ Features

- 🎙️ **Dual ASR Architecture (Cloud API & 100% Offline Local)**:
  - **Remote Providers**: One-click presets for **Groq** (`whisper-large-v3-turbo`), **Cloudflare Workers AI** (binary streaming & turbo JSON with Base64), **OpenAI**, and custom OpenAI-compatible endpoints (Ollama, LocalAI, vLLM).
  - **Local Offline Engines**: Run speech recognition completely on-device without internet via `whisper-cli` (CPU), `faster-whisper` (isolated Python virtualenv), or Vulkan GPU acceleration.
- 💾 **Model Weights & Custom Storage Management**:
  - Stream model weights directly from Hugging Face with real-time download progress bars.
  - Choose model sizes (`tiny`, `base`, `small`, etc.).
  - Configurable storage folder: store heavy weights on external drives or USB SSDs to protect internal disk space.
- ⚡ **In-Memory RAM Supervisor & Hardware Health**:
  - Keeps local models warm in RAM between dictations to prevent latency and eliminate repetitive disk read wear on consumer drives.
  - Configurable idle inactivity auto-unload timeout (2m, 5m, 10m, 30m, or Never) to free system memory when idle.
  - Manual "Release RAM Now" controls and live RAM status tracking.
- 🪟 **Mini Floating Mode & Window State Persistence**:
  - Shrink Lipi into an unobtrusive, always-on-top floating pill widget for seamless dictation while multitasking (`Alt + M`).
  - Automatically remembers and restores window coordinates `(x, y)`, dimensions, and maximized state across app sessions and mode transitions.
- 📜 **Smart History Sidebar**:
  - Distraction-free editing: History is hidden by default in compact/windowed mode and opens by default in full screen.
  - Window resizing never interrupts workflow or auto-hides/shows the sidebar—toggle visibility anytime via the `☰` button.
- ⏱️ **Extended & Custom Request Timeouts**:
  - Robust 3-minute (180s) minimum timeout prevents premature connection drops during long audio processing.
  - Configurable custom timeout in Settings (with 3m, 5m, 10m quick pills) applied across both ASR transcription and LLM transformations.
- 🤖 **LLM Post-Processing & Voice Presets**:
  - Automatic speech transformation, grammar rectification, summarization, and translation via OpenAI-compatible endpoints (Cloudflare Workers AI, Groq, OpenAI, Ollama, vLLM).
  - Open Markdown-based presets (`presets/*.md`) with frontmatter metadata and live editing.
  - Optional auto-mode: transcribes, transforms with LLM, and copies to clipboard in a single stroke.

---

## 🛠️ Tech Stack

- **Desktop Framework**: [Tauri v2](https://v2.tauri.app/)
- **Frontend**: [React 19](https://react.dev/), [TypeScript](https://www.typescriptlang.org/), [Vite](https://vitejs.dev/)
- **Backend**: [Rust](https://www.rust-lang.org/)
- **Audio Capture**: [`cpal`](https://github.com/RustAudio/cpal) (16kHz mono capture) & [`hound`](https://github.com/ruuda/hound) (WAV encoding)
- **Database**: Embedded SQLite via [`rusqlite`](https://github.com/rusqlite/rusqlite)
- **HTTP Client**: [`reqwest`](https://github.com/seanmonstar/reqwest) (rustls, multipart audio upload & base64 JSON streaming)
- **Local ASR**: `whisper.cpp` (`whisper-cli`) and `faster-whisper` in an isolated virtual environment

---

## 🚀 Getting Started

### Prerequisites

1. **Node.js**: v18+ recommended (npm, pnpm, or yarn)
2. **Rust toolchain**: Install via [rustup](https://rustup.rs/)
3. **System Dependencies** (Linux):
   ```bash
   # Debian / Ubuntu
   sudo apt update
   sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev python3 python3-venv zenity
   ```

### Installation & Setup

1. **Clone the repository**:
   ```bash
   git clone https://github.com/<your-username>/lipi.git
   cd lipi
   ```

2. **Install Node dependencies**:
   ```bash
   npm install
   ```

3. **Run in development mode**:
   ```bash
   npm run tauri dev
   ```

4. **Run automated tests**:
   ```bash
   # Rust backend tests
   cargo test --manifest-path src-tauri/Cargo.toml

   # Frontend TypeScript check and build
   npm run build
   ```

5. **Build production binaries**:
   ```bash
   npm run tauri build
   ```

---

## ⚙️ Settings & Configuration

Lipi provides a modular full-page Settings dashboard organized into four tabs:

### 1. 🎙 ASR (Automatic Speech Recognition)
- **Engine Mode**:
  - **Remote API**: Select from presets (**Groq**, **Cloudflare Workers AI**, **OpenAI**, **Local / Self-Hosted**, or **Custom**).
  - **Local Offline Engine**: Select runner (`whisper_cpu`, `faster_whisper`, or `whisper_vulkan`).
- **Runner Setup**:
  - One-click setup for runner binary or isolated Python virtual environment.
- **Model Storage & Weights**:
  - Select or browse storage directory (e.g. `/media/external/models`).
  - Stream download weights (`tiny`, `base`, `small`) with live progress.
- **RAM Supervisor**:
  - Configure idle unload timeout (2 min, 5 min, 10 min default, 30 min, or Never).
  - Live RAM usage monitor and manual unload button.
- **Request Timeout**:
  - Unified timeout control with 3m (180s default/minimum), 5m, and 10m quick selectors or custom seconds input.

### 2. 🤖 LLM (Post-Processing & Rectification)
- **Configured Providers**:
  - Add and manage multiple AI endpoints: **Cloudflare Workers AI**, **Groq Cloud**, **OpenAI**, and **Custom / Local** (Ollama, vLLM, Speaches).
  - Live model directory lookup via `/models` endpoints with curated fallbacks.
- **Voice Presets & Markdown Prompt Engine**:
  - Switch transformation instructions on the fly: Grammar Fix, Professional Tone, Casual, Concise Summary, Bullet Points, or custom.
  - Presets are stored as open `.md` files with YAML frontmatter in `presets/`—editable via your favorite text editor.
- **Auto-Transform Mode**:
  - Immediately post-process transcribed speech with your active LLM preset when recording stops.
- **Request Timeout**:
  - Shared timeout limit ensuring LLMs have adequate time for large generation tasks.

### 3. ⚙ Preferences
- **Language Code**: Optional ISO-639-1 code (e.g., `en`, `bn`, `es`, `hi`) or empty for auto-detection.
- **Auto-Copy to Clipboard**: Copy transcript immediately upon recording completion.
- **Always on Top**: Keep the window floating above all desktop applications.

### 4. 📋 Diagnostic Logs
- Real-time log history of transcription attempts, backend errors, endpoints, and models.
- One-click `📋 Copy All Logs` to generate a formatted diagnostic report for troubleshooting.

---

## 📂 Project Structure

```
lipi/
├── src/                      # React frontend (Vite + TypeScript)
│   ├── App.tsx               # Main UI, dual editor, settings tabs, shortcuts
│   ├── App.css               # Responsive styling, mini wizard, settings themes
│   └── main.tsx              # React root entry
├── src-tauri/                # Tauri Rust application
│   ├── src/
│   │   ├── audio.rs          # CPAL audio recording & linear resampling
│   │   ├── db.rs             # SQLite storage (notes, app settings, window state)
│   │   ├── engine.rs         # Local Whisper runner & RAM supervisor
│   │   ├── env_config.rs     # Multi-provider LLM configuration & .env sync
│   │   ├── llm.rs            # LLM API client & chat completions
│   │   ├── models.rs         # Hugging Face download & binary setup
│   │   ├── presets.rs        # Markdown voice preset manager
│   │   ├── transcribe.rs     # Cloud ASR client (Groq, Cloudflare, OpenAI)
│   │   ├── lib.rs            # Tauri commands, lifecycle events, window state
│   │   └── main.rs           # Application entry
│   ├── Cargo.toml            # Rust dependencies & optimization profiles
│   └── tauri.conf.json       # Tauri configuration & window settings
├── package.json              # Frontend dependencies & scripts
└── README.md
```

---

## 📄 License

MIT License. See [LICENSE](LICENSE) for details.
