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
- 🪟 **Mini Floating Mode**:
  - Shrink Lipi into an unobtrusive, always-on-top floating pill widget for seamless dictation while multitasking (`Alt + M`).
- ⌨️ **Global Keyboard Shortcut**:
  - Press `Alt + R` anywhere in the app to toggle recording immediately.
- 📝 **Local Notes Management**:
  - All transcribed notes are stored locally in an embedded SQLite database (`lipi.db`). Search, edit, and copy anytime.
- 📋 **Auto-Copy to Clipboard**:
  - Instantly copies completed transcripts to your system clipboard ready to paste into any editor or workflow.
- 📋 **Activity & Diagnostic Logs**:
  - Built-in logging card with error inspect, expandable server traces, and one-click diagnostic report copying.
- 🤖 **Future LLM Pipeline (Coming Soon)**:
  - Dedicated architecture for chaining ASR transcripts into LLMs for punctuation cleanup, bullet summaries, action-item extraction, and translation.

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

### 2. 🤖 LLM (Coming Soon)
- Preview for upcoming AI post-processing features: grammar and punctuation polish, action items & bullet summaries, multilingual translation, and local/cloud LLM connectivity (Ollama, Groq, OpenAI).

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
│   ├── App.tsx               # Main UI, settings tabs, shortcut handling, logs
│   ├── App.css               # Styling, themes, mini wizard, settings tabs
│   └── main.tsx              # React root entry
├── src-tauri/                # Tauri Rust application
│   ├── src/
│   │   ├── audio.rs          # CPAL audio recording & resampling
│   │   ├── db.rs             # SQLite storage for notes & settings
│   │   ├── engine.rs         # Local Whisper execution & RAM supervisor
│   │   ├── models.rs         # Model weights download & runner management
│   │   ├── transcribe.rs     # Remote API client (Groq, Cloudflare, OpenAI)
│   │   ├── lib.rs            # Tauri commands & AppState
│   │   └── main.rs           # Application entry
│   ├── Cargo.toml            # Rust dependencies & metadata
│   └── tauri.conf.json       # Tauri configuration & window settings
├── package.json              # Frontend dependencies & scripts
└── README.md
```

---

## 📄 License

MIT License. See [LICENSE](LICENSE) for details.
