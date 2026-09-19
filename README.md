# Lipi (লিপি)

> **Lipi** (*"script"* or *"writing"* in Bengali) is a fast, lightweight, local-first desktop voice dictation and AI assistant.

### 📥 Download Pre-Release

Get pre-compiled binaries for your OS from **[Latest GitHub Releases](https://github.com/debjit/lipi/releases)**:

| Platform | Package Formats | Download |
| :--- | :--- | :--- |
| 🪟 **Windows** | `.msi`, `setup.exe` (NSIS) | **[Windows Binaries ➔](https://github.com/debjit/lipi/releases)** |
| 🐧 **Linux** | `.deb`, `.AppImage` (x86_64) | **[Linux Binaries ➔](https://github.com/debjit/lipi/releases)** |

---

Press **`Alt + R`** anywhere to speak. Lipi transcribes your voice—either 100% offline with local Whisper or via high-speed cloud AI—optionally cleans up grammar or reformats with LLMs, and automatically pastes the result directly into your active window (VSCode, Cursor, browser, terminal).

---

## ⚡ Overview

- 🎙️ **System-Wide Dictation (`Alt + R`)**: Speak from anywhere without leaving your working window. Lipi restores your target app and auto-pastes the text.
- 🔒 **Offline or Cloud Speech-to-Text**: Run private Whisper models on CPU/GPU without internet, or connect fast cloud APIs (Groq, Cloudflare Workers AI, OpenAI).
- 🤖 **Automated AI Polish**: Post-process speech on the fly—fix grammar, summarize, or reformat using local or remote LLMs.
- 🪟 **Compact Floating Widget (`Alt + M`)**: Collapse Lipi into a minimalist, always-on-top pill widget that stays out of your way while multitasking.
- 📝 **Local-First History**: Search and manage all previous dictations, notes, and AI transformations locally in SQLite.

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
   git clone https://github.com/debjit/lipi.git
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
- **Auto-Paste to Previous Application**: Restore previous app window and send `Ctrl+V` after dictation/auto-transform.
- **Always on Top**: Keep the window floating above all desktop applications.
- **Start Lipi on System Startup**: Automatically launches Lipi upon desktop login.
- **Alt+R Shortcut Recording Behavior**: Configure whether `Alt+R` creates an independent new note (default) or appends onto the active note.
- **Mini Wizard Recording Behavior**: Configure whether finishing a recording in the Mini Wizard saves as a new note (default) or appends to the current note.

### 4. 📋 Diagnostic Logs
- Real-time log history of transcription attempts, backend errors, endpoints, and models.
- One-click `📋 Copy All Logs` to generate a formatted diagnostic report for troubleshooting.

---

## 💡 How-To & FAQ

### How does Auto-Paste work?

1. Open **Settings > Preferences** (or the ⚙ menu) and enable **"Paste into previous application"**.
2. Work in your favorite app (VSCode, Cursor, browser, Slack, terminal).
3. Press **`Alt + R`** from anywhere to start recording.
4. Speak, then press **`Alt + R`** again to stop.
5. Lipi transcribes (and optionally applies your active AI prompt), restores your target app, and types `Ctrl+V` directly into your editor.

> [!NOTE]
> **Working in Lipi:** If you press `Alt + R` while working directly inside Lipi's main window, Lipi automatically keeps the text in Lipi's own editor and will **not** minimize or switch away to a background window.

---

### Platform Paste Behavior: Linux vs. Windows

| Platform | Auto-Paste Behavior | Setup Required |
| :--- | :--- | :--- |
| **Windows** | **Works seamlessly out of the box.** Lipi automatically restores your foreground window and simulates `Ctrl+V`. | None. |
| **Linux (X11)** | **Works out of the box.** Window activation and `Ctrl+V` are simulated directly. | None. |
| **Linux (Wayland)** | **Requires Screen Control approval.** Wayland sandboxes input between windows. When auto-paste triggers, GNOME shows a **"Remote Desktop"** prompt. Click **Allow** to permit `Ctrl+V` injection. | Click **Allow** on the GNOME prompt, or install `ydotool` for prompt-less background pasting. |

#### Why does GNOME Wayland show a "Remote Desktop" prompt?
Modern Wayland compositors (such as GNOME Mutter on Ubuntu / Fedora) isolate applications for security, preventing any background app from silently injecting synthetic keystrokes into other windows. To send `Ctrl+V`, Lipi routes the key through the official desktop portal, which prompts you to authorize input control.

> **Optional (Bypass Prompt on Wayland):**
> If you prefer completely silent background key injection on Wayland without any system prompt, you can install `ydotool`:
> ```bash
> sudo apt install ydotool
> systemctl --user enable --now ydotoold
> ```
> When `ydotool` is detected on your system, Lipi uses it automatically.

---

### FAQ

#### Q: How do I choose whether `Alt + R` creates a new note or appends?
In **Settings > Preferences**, look for **"Alt+R Shortcut Recording Behavior"**:
- **Create a new note for each recording (Default)**: Each recording starts a fresh note and clean transcription/conversion.
- **Append to current active note**: Adds newly dictated speech to the end of the currently open note.

#### Q: What is the Mini Floating Wizard?
Press **`Alt + M`** (or click the `⊡ Mini Wizard` button in the navbar) to collapse Lipi into a compact, always-on-top floating pill. It lets you record, view audio levels, and monitor AI progress while multitasking. Press `Alt + M` again to expand back to full mode.

#### Q: Where is my data stored?
All recordings, notes, and preferences are stored locally on your machine in an embedded SQLite database located at `~/.local/share/com.lipi.app/lipi.db` (Linux) or `%LOCALAPPDATA%\com.lipi.app\lipi.db` (Windows). No audio or notes are uploaded to any external server unless you configure a remote cloud AI provider.

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

GNU Affero General Public License v3.0 (AGPL-3.0). See [LICENSE](LICENSE) for details.
