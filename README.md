# Lipi (লিপি)

> **Lipi** (লিপি) means *"script"* or *"writing"* in Bengali.

Lipi is a lightweight, local-first desktop speech-to-text and voice note application. Built with **Tauri v2**, **Rust**, and **React 19**, it lets you record audio, transcribe speech with OpenAI Whisper or any OpenAI-compatible local/remote endpoint, and manage your notes seamlessly.

---

## ✨ Features

- 🎙️ **Fast Speech-to-Text**: Record high-quality microphone audio encoded to 16kHz WAV and transcribe using OpenAI Whisper or self-hosted OpenAI-compatible APIs (Ollama, Whisper.cpp, LocalAI).
- 📝 **Local Notes Management**: All transcribed notes are stored locally in an embedded SQLite database (`lipi.db`). Create, edit, and search your history anytime.
- 📋 **Auto-Copy to Clipboard**: Instantly copies transcribed speech into your clipboard ready to paste into any editor or workflow.
- 🪟 **Mini Floating Mode**: Shrink Lipi into an unobtrusive, always-on-top floating pill widget for dictating while working across apps.
- ⌨️ **Keyboard Shortcut**: Press `Alt + R` anywhere in the app to start or stop recording immediately.
- 🔒 **Privacy First**: Audio is recorded and handled locally; recordings are sent strictly to the transcription endpoint you configure.

---

## 🛠️ Tech Stack

- **Desktop Framework**: [Tauri v2](https://v2.tauri.app/)
- **Frontend**: [React 19](https://react.dev/), [TypeScript](https://www.typescriptlang.org/), [Vite](https://vitejs.dev/)
- **Backend**: [Rust](https://www.rust-lang.org/)
- **Audio Capture**: [`cpal`](https://github.com/RustAudio/cpal) & [`hound`](https://github.com/ruuda/hound)
- **Database**: Embedded SQLite via [`rusqlite`](https://github.com/rusqlite/rusqlite)
- **HTTP Client**: [`reqwest`](https://github.com/seanmonstar/reqwest) (rustls)

---

## 🚀 Getting Started

### Prerequisites

1. **Node.js**: v18+ recommended (npm, pnpm, or yarn)
2. **Rust toolchain**: Install via [rustup](https://rustup.rs/)
3. **System Dependencies** (Linux only):
   ```bash
   # Debian / Ubuntu
   sudo apt update
   sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev
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

4. **Build production binaries**:
   ```bash
   npm run tauri build
   ```

---

## ⚙️ Configuration

In Lipi, click the **⚙ Settings** button in the sidebar to configure:

- **API Base URL**: Default is `https://api.openai.com/v1`. Point this to a local server (e.g. `http://localhost:8080/v1` for `whisper.cpp` or LocalAI) for completely offline/local transcription.
- **API Key**: Required for OpenAI. Leave blank if using a local mock endpoint without authentication.
- **Model**: Default `whisper-1`. Customize for local server models.
- **Language Code**: Optional ISO-639-1 language code (e.g., `en`, `bn` for Bengali, etc.). Leave empty for automatic language detection.
- **Auto-Copy to Clipboard**: Automatically copy transcription upon completion.
- **Always on Top**: Keep the window floating above other desktop windows.

---

## 📂 Project Structure

```
lipi/
├── src/                # React frontend (Vite + TypeScript)
│   ├── App.tsx         # Main UI, state, shortcut handling
│   ├── App.css         # Styling, mini-mode, themes
│   └── main.tsx        # React root entry
├── src-tauri/          # Tauri Rust application
│   ├── src/
│   │   ├── audio.rs    # CPAL audio recording & resampling
│   │   ├── db.rs       # SQLite storage for notes & settings
│   │   ├── transcribe.rs # HTTP multipart upload to Whisper API
│   │   ├── lib.rs      # Tauri commands & App state
│   │   └── main.rs     # Application entry
│   ├── Cargo.toml      # Rust dependencies & metadata
│   └── tauri.conf.json # Tauri configuration & window settings
├── package.json        # Frontend dependencies & scripts
└── README.md
```

---

## 📄 License

MIT License. See [LICENSE](LICENSE) for details.
