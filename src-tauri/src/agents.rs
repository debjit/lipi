use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use crate::models;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentId {
    Hermes,
    Opencode,
    Agy,
    Cursor,
}

impl AgentId {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Hermes => "hermes",
            AgentId::Opencode => "opencode",
            AgentId::Agy => "agy",
            AgentId::Cursor => "cursor",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            AgentId::Hermes => "Hermes",
            AgentId::Opencode => "OpenCode",
            AgentId::Agy => "Agy (Antigravity)",
            AgentId::Cursor => "Cursor",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "hermes" => Ok(AgentId::Hermes),
            "opencode" | "open-code" => Ok(AgentId::Opencode),
            "agy" | "antigravity" => Ok(AgentId::Agy),
            "cursor" | "cursor-agent" => Ok(AgentId::Cursor),
            other => Err(format!(
                "Unknown agent '{other}'. Use hermes, opencode, agy, or cursor."
            )),
        }
    }

    pub fn all() -> [AgentId; 4] {
        [AgentId::Hermes, AgentId::Opencode, AgentId::Agy, AgentId::Cursor]
    }

    fn bin_names(self) -> &'static [&'static str] {
        match self {
            AgentId::Hermes => {
                if cfg!(windows) {
                    &["hermes.exe", "hermes.cmd", "hermes"]
                } else {
                    &["hermes"]
                }
            }
            AgentId::Opencode => {
                if cfg!(windows) {
                    &["opencode.exe", "opencode.cmd", "opencode"]
                } else {
                    &["opencode"]
                }
            }
            AgentId::Agy => {
                if cfg!(windows) {
                    &["agy.exe", "agy.cmd", "agy"]
                } else {
                    &["agy"]
                }
            }
            AgentId::Cursor => {
                if cfg!(windows) {
                    &["cursor-agent.exe", "cursor-agent.cmd", "cursor-agent.ps1", "cursor-agent"]
                } else {
                    &["cursor-agent"]
                }
            }
        }
    }

    pub fn docs_url(self) -> &'static str {
        match self {
            AgentId::Hermes => "https://hermes-agent.nousresearch.com/docs/getting-started/installation",
            AgentId::Opencode => "https://opencode.ai/docs",
            AgentId::Agy => "https://antigravity.google/docs/cli/install/",
            AgentId::Cursor => "https://cursor.com/docs/cli/installation",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOverride {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub binary_path: String,
    #[serde(default)]
    pub model: String,
}

impl Default for AgentOverride {
    fn default() -> Self {
        Self {
            enabled: true,
            binary_path: String::new(),
            model: String::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_destination() -> String {
    "notes".into()
}

fn default_agent() -> String {
    "hermes".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_destination")]
    pub destination: String,
    #[serde(default = "default_agent")]
    pub default_agent: String,
    #[serde(default)]
    pub workspace_dir: String,
    #[serde(default = "default_true")]
    pub review_before_send: bool,
    #[serde(default)]
    pub continue_last_session: bool,
    #[serde(default = "default_true")]
    pub apply_edits: bool,
    #[serde(default)]
    pub opencode_auto_approve: bool,
    #[serde(default)]
    pub hermes: AgentOverride,
    #[serde(default)]
    pub opencode: AgentOverride,
    #[serde(default)]
    pub agy: AgentOverride,
    #[serde(default)]
    pub cursor: AgentOverride,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            destination: default_destination(),
            default_agent: default_agent(),
            workspace_dir: String::new(),
            review_before_send: true,
            continue_last_session: false,
            apply_edits: true,
            opencode_auto_approve: false,
            hermes: AgentOverride::default(),
            opencode: AgentOverride::default(),
            agy: AgentOverride::default(),
            cursor: AgentOverride::default(),
        }
    }
}

impl AgentConfig {
    pub fn override_for(&self, id: AgentId) -> &AgentOverride {
        match id {
            AgentId::Hermes => &self.hermes,
            AgentId::Opencode => &self.opencode,
            AgentId::Agy => &self.agy,
            AgentId::Cursor => &self.cursor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentScanItem {
    pub id: String,
    pub name: String,
    pub found: bool,
    pub path: Option<String>,
    pub docs_url: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub agent_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub output: String,
}

pub struct AgentRunner {
    child: Mutex<Option<Child>>,
}

impl AgentRunner {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }
}

pub fn parse_config(raw: Option<&str>) -> AgentConfig {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

pub fn serialize_config(config: &AgentConfig) -> Result<String, String> {
    serde_json::to_string(config).map_err(|e| e.to_string())
}

fn extra_search_dirs(id: AgentId) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    match id {
        AgentId::Cursor => {
            if let Some(local) = &local {
                dirs.push(local.join("cursor-agent"));
            }
        }
        AgentId::Hermes => {
            if let Some(local) = &local {
                dirs.push(local.join("hermes").join("hermes-agent").join("bin"));
                dirs.push(local.join("hermes").join("bin"));
            }
            if let Some(home) = &home {
                dirs.push(home.join(".hermes").join("bin"));
            }
        }
        AgentId::Opencode => {
            if let Some(roaming) = &roaming {
                dirs.push(roaming.join("npm"));
            }
            if let Some(local) = &local {
                dirs.push(local.join("pnpm"));
            }
            if let Some(home) = &home {
                dirs.push(home.join(".opencode").join("bin"));
            }
        }
        AgentId::Agy => {}
    }

    if let Some(home) = home {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join("bin"));
    }
    dirs
}

fn probe_dir(dir: &Path, id: AgentId) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    for name in id.bin_names() {
        let probe = dir.join(name);
        if probe.is_file() {
            return Some(probe);
        }
    }
    None
}

pub fn find_binary(id: AgentId, custom_path: &str) -> Option<PathBuf> {
    let custom = custom_path.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
        return None;
    }

    for name in id.bin_names() {
        if let Some(found) = models::which_command(name) {
            if found.is_file() {
                return Some(found);
            }
        }
    }

    for dir in extra_search_dirs(id) {
        if let Some(found) = probe_dir(&dir, id) {
            return Some(found);
        }
    }

    None
}

pub fn scan_agents(config: &AgentConfig) -> Vec<AgentScanItem> {
    AgentId::all()
        .into_iter()
        .map(|id| {
            let ov = config.override_for(id);
            let path = find_binary(id, &ov.binary_path);
            AgentScanItem {
                id: id.as_str().to_string(),
                name: id.display_name().to_string(),
                found: path.is_some(),
                path: path.map(|p| p.to_string_lossy().to_string()),
                docs_url: id.docs_url().to_string(),
                enabled: ov.enabled,
            }
        })
        .collect()
}

fn push_model_flag(args: &mut Vec<String>, id: AgentId, model: &str) {
    let model = normalize_model(id, model);
    if !model.is_empty() {
        args.push("--model".into());
        args.push(model);
    }
}

fn normalize_model(id: AgentId, raw: &str) -> String {
    let raw = raw.trim().trim_matches('"').replace('\r', "");
    if raw.is_empty() {
        return String::new();
    }
    match id {
        AgentId::Agy => agy_model_from_line(&raw).unwrap_or(raw),
        _ => raw
            .split('\t')
            .next()
            .unwrap_or(&raw)
            .trim()
            .to_string(),
    }
}

fn agy_model_from_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let lower = line.to_ascii_lowercase();
    if looks_like_noise(line) || lower.starts_with("agy ") || matches!(lower.as_str(), "name" | "label" | "model" | "id") {
        return None;
    }
    if let Some((_, right)) = line.split_once('\t') {
        let right = right.trim();
        if !right.is_empty() {
            return Some(right.to_string());
        }
    }
    if let Some(idx) = line.find("  ") {
        let right = line[idx..].trim();
        if right.contains('(') || right.contains(' ') {
            return Some(right.to_string());
        }
    }
    Some(line.to_string())
}

pub fn build_dispatch_args(
    id: AgentId,
    prompt: &str,
    workspace: Option<&Path>,
    continue_session: bool,
    apply_edits: bool,
    opencode_auto_approve: bool,
    model: &str,
) -> Result<Vec<String>, String> {
    if prompt.trim().is_empty() {
        return Err("Prompt is empty.".into());
    }
    if let Some(workspace) = workspace {
        if !workspace.exists() {
            return Err(format!("Workspace folder does not exist: {}", workspace.display()));
        }
    }

    let mut args = Vec::new();
    match id {
        AgentId::Hermes => {
            if continue_session {
                args.push("--continue".into());
            }
            args.push("chat".into());
            push_model_flag(&mut args, id, model);
            args.push("-q".into());
            args.push(prompt.to_string());
        }
        AgentId::Opencode => {
            args.push("run".into());
            if let Some(workspace) = workspace {
                args.push("--dir".into());
                args.push(workspace.to_string_lossy().to_string());
            }
            if continue_session {
                args.push("--continue".into());
            }
            if opencode_auto_approve {
                args.push("--auto".into());
            }
            push_model_flag(&mut args, id, model);
            args.push(prompt.to_string());
        }
        AgentId::Agy => {
            push_model_flag(&mut args, id, model);
            args.push("-p".into());
            args.push(prompt.to_string());
            if continue_session {
                args.push("--continue".into());
            }
        }
        AgentId::Cursor => {
            push_model_flag(&mut args, id, model);
            args.push("-p".into());
            args.push(prompt.to_string());
            if apply_edits {
                args.push("--force".into());
            }
        }
    }
    Ok(args)
}

fn list_model_commands(id: AgentId) -> Vec<Vec<String>> {
    match id {
        AgentId::Hermes => vec![
            vec!["models".into(), "list".into(), "--json".into()],
            vec!["models".into(), "list".into()],
        ],
        AgentId::Opencode => vec![vec!["models".into()]],
        AgentId::Agy => vec![vec!["models".into()]],
        AgentId::Cursor => vec![vec!["--list-models".into()], vec!["models".into()]],
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn looks_like_noise(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return true;
    }
    let lower = line.to_ascii_lowercase();
    lower.starts_with("usage:")
        || lower.starts_with("error:")
        || lower.starts_with("unknown ")
        || lower.starts_with("available models")
        || lower.starts_with("tip:")
        || lower.starts_with("warning:")
        || (line.starts_with('-') && line.contains("help"))
}

fn is_model_token(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() < 160
        && !s.contains(' ')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@+".contains(c))
}

fn uniq_models(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in items {
        let s = raw.trim().to_string();
        if s.is_empty() || s.len() > 200 {
            continue;
        }
        if seen.insert(s.clone()) {
            out.push(s);
        }
        if out.len() >= 500 {
            break;
        }
    }
    out
}

fn collect_model_entries(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(arr) => {
            for x in arr {
                match x {
                    Value::String(s) => out.push(s.clone()),
                    Value::Object(map) => {
                        if let Some(s) = map
                            .get("id")
                            .and_then(|x| x.as_str())
                            .or_else(|| map.get("model").and_then(|x| x.as_str()))
                            .or_else(|| map.get("model_id").and_then(|x| x.as_str()))
                            .or_else(|| map.get("name").and_then(|x| x.as_str()))
                        {
                            out.push(s.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Object(map) => {
            for (k, val) in map {
                if let Value::String(s) = val {
                    out.push(s.clone());
                } else {
                    out.push(k.clone());
                }
            }
        }
        Value::String(s) => out.push(s.clone()),
        _ => {}
    }
}

fn collect_json_models(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(arr) => {
            for x in arr {
                if let Value::String(s) = x {
                    out.push(s.clone());
                } else {
                    collect_json_models(x, out);
                }
            }
        }
        Value::Object(map) => {
            if let Some(models) = map.get("models") {
                collect_model_entries(models, out);
            }
            if let Some(data) = map.get("data") {
                collect_model_entries(data, out);
            }
            if let Some(providers) = map.get("providers") {
                collect_json_models(providers, out);
            }
            if !map.contains_key("models")
                && !map.contains_key("providers")
                && !map.contains_key("data")
            {
                if let Some(s) = map
                    .get("id")
                    .and_then(|x| x.as_str())
                    .or_else(|| map.get("model").and_then(|x| x.as_str()))
                    .or_else(|| map.get("model_id").and_then(|x| x.as_str()))
                {
                    out.push(s.to_string());
                }
            }
        }
        Value::String(s) => out.push(s.clone()),
        _ => {}
    }
}

fn parse_json_value(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Some(v);
    }
    let start = t.find(['{', '['])?;
    let slice = &t[start..];
    let mut de = serde_json::Deserializer::from_str(slice);
    Value::deserialize(&mut de).ok()
}

fn extract_json_models(text: &str) -> Vec<String> {
    let Some(value) = parse_json_value(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_json_models(&value, &mut out);
    uniq_models(out)
}

fn parse_cursor_models(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = strip_ansi(line).trim().to_string();
        if looks_like_noise(&line) {
            continue;
        }
        if let Some((id, _)) = line.split_once(" - ") {
            if is_model_token(id) {
                out.push(id.trim().to_string());
            }
        } else if is_model_token(&line) {
            out.push(line);
        }
    }
    uniq_models(out)
}

fn parse_provider_slash_models(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = strip_ansi(line).trim().to_string();
        if looks_like_noise(&line) {
            continue;
        }
        if line.contains('/') || is_model_token(&line) {
            out.push(line);
        }
    }
    uniq_models(out)
}

fn parse_display_name_models(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = strip_ansi(line).trim().to_string();
        if let Some(name) = agy_model_from_line(&line) {
            out.push(name);
        }
    }
    uniq_models(out)
}

pub(crate) fn parse_listed_models(id: AgentId, output: &str) -> Vec<String> {
    let cleaned = strip_ansi(output);
    let json_ids = extract_json_models(&cleaned);
    if !json_ids.is_empty() {
        return json_ids;
    }
    match id {
        AgentId::Cursor => parse_cursor_models(&cleaned),
        AgentId::Opencode => parse_provider_slash_models(&cleaned),
        AgentId::Agy => parse_display_name_models(&cleaned),
        AgentId::Hermes => {
            let slash = parse_provider_slash_models(&cleaned);
            if slash.is_empty() {
                parse_display_name_models(&cleaned)
            } else {
                slash
            }
        }
    }
}

const LIST_TIMEOUT: Duration = Duration::from_secs(45);

fn is_auth_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("authentication failed")
        || lower.contains("not logged in")
        || (lower.contains("credentials") && lower.contains("expired"))
        || lower.contains("cursor_api_key")
}

fn list_error_message(id: AgentId, args: &[String], output: &str) -> String {
    if is_auth_failure(output) {
        let first = output
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Authentication required.");
        return format!(
            "{}. Run `{} login` in a terminal, then Fetch models again.",
            first.trim(),
            id.bin_names().first().copied().unwrap_or(id.as_str())
        );
    }
    let snippet: String = output.chars().filter(|c| *c != '\0').take(240).collect();
    let snippet = snippet.trim();
    if snippet.is_empty() {
        format!(
            "{} did not return a model list (`{}`). Type a model id, or restart Lipi after installing the CLI.",
            id.display_name(),
            args.join(" ")
        )
    } else {
        format!(
            "{} did not return a model list (`{}`): {snippet}",
            id.display_name(),
            args.join(" ")
        )
    }
}

#[cfg(windows)]
fn npm_node_command(cmd_file: &Path) -> Option<Command> {
    let dir = cmd_file.parent()?;
    let script = dir
        .join("node_modules")
        .join("opencode-ai")
        .join("bin")
        .join("opencode");
    if !script.is_file() {
        return None;
    }
    let node = if dir.join("node.exe").is_file() {
        dir.join("node.exe")
    } else {
        models::which_command("node")?
    };
    let mut cmd = Command::new(node);
    cmd.arg(script);
    apply_no_window(&mut cmd);
    Some(cmd)
}

fn cli_command(binary: &Path, args: &[String]) -> Command {
    #[cfg(windows)]
    {
        let ext = binary
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if matches!(ext.as_str(), "ps1" | "cmd" | "bat") {
            let ps1 = if ext == "ps1" {
                binary.to_path_buf()
            } else {
                binary.with_extension("ps1")
            };
            if ps1.is_file() {
                let mut cmd = Command::new("powershell.exe");
                cmd.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-WindowStyle",
                    "Hidden",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ]);
                cmd.arg(&ps1);
                cmd.args(args);
                return cmd;
            }

            if let Some(mut cmd) = npm_node_command(binary) {
                cmd.args(args);
                return cmd;
            }

            let mut cmd = Command::new("cmd.exe");
            cmd.arg("/D");
            cmd.arg("/S");
            cmd.arg("/C");
            let mut inner = format!("\"{}\"", binary.display());
            for a in args {
                inner.push(' ');
                if a.is_empty() || a.chars().any(|c| c.is_whitespace() || "\"&<>|^".contains(c)) {
                    inner.push('"');
                    inner.push_str(&a.replace('"', "\\\""));
                    inner.push('"');
                } else {
                    inner.push_str(a);
                }
            }
            cmd.arg(inner);
            return cmd;
        }

        let mut cmd = Command::new(binary);
        cmd.args(args);
        apply_no_window(&mut cmd);
        cmd
    }

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(binary);
        cmd.args(args);
        apply_no_window(&mut cmd);
        cmd
    }
}

fn run_short(binary: &Path, args: &[String], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = cli_command(binary, args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "dumb");

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_h = stdout.map(|pipe| thread::spawn(move || read_pipe(pipe)));
    let err_h = stderr.map(|pipe| thread::spawn(move || read_pipe(pipe)));
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > LIST_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Timed out waiting for the model list.".into());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    let stdout_text = out_h.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr_text = err_h.and_then(|h| h.join().ok()).unwrap_or_default();
    let mut output = stdout_text;
    if !stderr_text.trim().is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&stderr_text);
    }
    Ok(output)
}

pub fn list_models_blocking(config: &AgentConfig, agent_id_raw: &str) -> Result<Vec<String>, String> {
    let id = AgentId::parse(agent_id_raw)?;
    let ov = config.override_for(id);
    if !ov.enabled {
        return Err(format!("{} is disabled in Agents settings.", id.display_name()));
    }
    let binary = find_binary(id, &ov.binary_path).ok_or_else(|| {
        format!(
            "{} CLI was not found. Put it on PATH or set a custom binary path.",
            id.display_name()
        )
    })?;
    let cwd = {
        let workspace = config.workspace_dir.trim();
        if !workspace.is_empty() && Path::new(workspace).is_dir() {
            Some(PathBuf::from(workspace))
        } else {
            None
        }
    };

    let mut last_err = format!(
        "{} does not expose a model list. Type a model id if the CLI accepts --model.",
        id.display_name()
    );
    for args in list_model_commands(id) {
        match run_short(&binary, &args, cwd.as_deref()) {
            Ok(output) => {
                let models = parse_listed_models(id, &output);
                if !models.is_empty() {
                    return Ok(models);
                }
                last_err = list_error_message(id, &args, &output);
            }
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

fn apply_no_window(cmd: &mut Command) {
    models::apply_no_window(cmd);
}

fn read_pipe(pipe: impl Read) -> String {
    let mut buf = String::new();
    let mut reader = pipe;
    let _ = reader.read_to_string(&mut buf);
    buf
}

pub fn cancel_run(runner: &AgentRunner) -> Result<(), String> {
    let mut guard = runner.child.lock().map_err(|e| e.to_string())?;
    if let Some(child) = guard.as_mut() {
        let _ = child.kill();
    }
    Ok(())
}

pub fn dispatch_blocking(
    runner: &AgentRunner,
    config: &AgentConfig,
    agent_id_raw: &str,
    prompt: &str,
    workspace_override: Option<&str>,
    continue_override: Option<bool>,
    model_override: Option<&str>,
) -> Result<AgentRunResult, String> {
    let id = AgentId::parse(agent_id_raw)?;
    let ov = config.override_for(id);
    if !ov.enabled {
        return Err(format!("{} is disabled in Agents settings.", id.display_name()));
    }

    let binary = find_binary(id, &ov.binary_path).ok_or_else(|| {
        format!(
            "{} CLI was not found. Put it on PATH or set a custom binary path. Docs: {}",
            id.display_name(),
            id.docs_url()
        )
    })?;

    let workspace_raw = workspace_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(config.workspace_dir.trim());
    let workspace = if workspace_raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(workspace_raw))
    };
    let model = model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(ov.model.trim());

    let args = build_dispatch_args(
        id,
        prompt,
        workspace.as_deref(),
        continue_override.unwrap_or(config.continue_last_session),
        config.apply_edits,
        config.opencode_auto_approve,
        model,
    )?;

    {
        let mut guard = runner.child.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = guard.as_mut() {
            let _ = existing.kill();
            let _ = existing.wait();
        }
        *guard = None;
    }

    let mut cmd = cli_command(&binary, &args);
    if let Some(dir) = workspace.as_ref() {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        format!("Failed to start {} ({}): {e}", id.display_name(), binary.display())
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    {
        let mut guard = runner.child.lock().map_err(|e| e.to_string())?;
        *guard = Some(child);
    }

    let out_h = stdout.map(|pipe| thread::spawn(move || read_pipe(pipe)));
    let err_h = stderr.map(|pipe| thread::spawn(move || read_pipe(pipe)));

    let status = loop {
        let wait_result = {
            let mut guard = runner.child.lock().map_err(|e| e.to_string())?;
            match guard.as_mut() {
                Some(child) => child.try_wait().map_err(|e| e.to_string())?,
                None => break None,
            }
        };
        if let Some(st) = wait_result {
            break Some(st);
        }
        thread::sleep(Duration::from_millis(50));
    };

    {
        let mut guard = runner.child.lock().map_err(|e| e.to_string())?;
        *guard = None;
    }

    let stdout_text = out_h.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr_text = err_h.and_then(|h| h.join().ok()).unwrap_or_default();
    let mut output = stdout_text;
    if !stderr_text.trim().is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&stderr_text);
    }

    let (status_str, exit_code) = match status {
        Some(st) if st.success() => ("done", st.code()),
        Some(st) => ("failed", st.code()),
        None => ("cancelled", None),
    };

    Ok(AgentRunResult {
        agent_id: id.as_str().to_string(),
        status: status_str.into(),
        exit_code,
        output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_ids() {
        assert_eq!(AgentId::parse("Hermes").unwrap(), AgentId::Hermes);
        assert_eq!(AgentId::parse("open-code").unwrap(), AgentId::Opencode);
        assert_eq!(AgentId::parse("antigravity").unwrap(), AgentId::Agy);
        assert_eq!(AgentId::parse("cursor-agent").unwrap(), AgentId::Cursor);
        assert!(AgentId::parse("agent").is_err());
        assert!(AgentId::parse("aider").is_err());
    }

    #[test]
    fn config_json_roundtrip() {
        let mut cfg = AgentConfig::default();
        cfg.enabled = true;
        cfg.default_agent = "opencode".into();
        cfg.workspace_dir = r"C:\Projects\demo".into();
        cfg.cursor.binary_path = r"C:\tools\cursor-agent.exe".into();
        cfg.cursor.model = "gpt-5".into();
        let raw = serialize_config(&cfg).unwrap();
        let loaded = parse_config(Some(&raw));
        assert!(loaded.enabled);
        assert_eq!(loaded.default_agent, "opencode");
        assert_eq!(loaded.workspace_dir, r"C:\Projects\demo");
        assert_eq!(loaded.cursor.binary_path, r"C:\tools\cursor-agent.exe");
        assert_eq!(loaded.cursor.model, "gpt-5");
    }

    #[test]
    fn find_binary_uses_custom_path() {
        let file = std::env::temp_dir().join(format!("lipi_fake_hermes_{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let found = find_binary(AgentId::Hermes, file.to_str().unwrap()).unwrap();
        assert_eq!(found, file);
        assert!(find_binary(AgentId::Hermes, file.with_extension("missing").to_str().unwrap()).is_none());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn dispatch_args() {
        let tmp = std::env::temp_dir();
        let hermes = build_dispatch_args(AgentId::Hermes, "hello \"world\"", Some(&tmp), true, false, false, "").unwrap();
        assert!(hermes.contains(&"--continue".into()));
        assert!(hermes.contains(&"-q".into()));
        assert!(!hermes.contains(&"--model".into()));
        assert_eq!(hermes.last().unwrap(), "hello \"world\"");

        let hermes_m = build_dispatch_args(
            AgentId::Hermes,
            "hello",
            Some(&tmp),
            false,
            false,
            false,
            "anthropic/claude-sonnet-4",
        )
        .unwrap();
        assert!(hermes_m.contains(&"--model".into()));
        assert!(hermes_m.contains(&"anthropic/claude-sonnet-4".into()));

        let oc = build_dispatch_args(AgentId::Opencode, "fix it", Some(&tmp), false, true, true, "openai/gpt-4o").unwrap();
        assert_eq!(oc[0], "run");
        assert!(oc.contains(&"--auto".into()));
        assert!(oc.contains(&"--model".into()));
        assert!(oc.contains(&"--dir".into()));

        let oc_no_dir = build_dispatch_args(AgentId::Opencode, "fix it", None, false, false, false, "").unwrap();
        assert!(!oc_no_dir.contains(&"--dir".into()));

        let agy = build_dispatch_args(AgentId::Agy, "summarize", Some(&tmp), true, false, false, "Gemini 3.1 Pro (High)").unwrap();
        assert_eq!(agy[0], "--model");
        assert_eq!(agy[1], "Gemini 3.1 Pro (High)");
        assert!(!agy.iter().any(|a| a.contains("dangerously")));

        let cur = build_dispatch_args(AgentId::Cursor, "refactor", Some(&tmp), false, true, false, "gpt-5").unwrap();
        assert!(cur.contains(&"--force".into()));
        assert!(cur.contains(&"--model".into()));
        assert!(cur.contains(&"gpt-5".into()));
    }

    #[test]
    fn missing_workspace_errors() {
        let err = build_dispatch_args(
            AgentId::Hermes,
            "hi",
            Some(Path::new("/definitely/missing/lipi-workspace-xyz")),
            false,
            false,
            false,
            "",
        )
        .unwrap_err();
        assert!(err.to_lowercase().contains("workspace"));

        assert!(build_dispatch_args(AgentId::Hermes, "hi", None, false, false, false, "").is_ok());
    }

    #[test]
    fn parse_cursor_and_opencode_model_lists() {
        let cursor = parse_listed_models(
            AgentId::Cursor,
            "Available models\n\nauto - Auto (default)\ngpt-5.3-codex - Codex 5.3\nTip: use --model <id>\n",
        );
        assert_eq!(cursor, vec!["auto", "gpt-5.3-codex"]);

        let oc = parse_listed_models(
            AgentId::Opencode,
            "anthropic/claude-sonnet-4\nopenai/gpt-4o\nUsage: opencode models\n",
        );
        assert_eq!(oc, vec!["anthropic/claude-sonnet-4", "openai/gpt-4o"]);

        let auth = parse_listed_models(
            AgentId::Cursor,
            "Authentication failed: your Cursor credentials or API key are invalid or expired.\n",
        );
        assert!(auth.is_empty());
        assert!(is_auth_failure(
            "Authentication failed: your Cursor credentials or API key are invalid or expired."
        ));
    }

    #[test]
    fn parse_agy_and_hermes_model_lists() {
        let agy = parse_listed_models(
            AgentId::Agy,
            "Gemini 3.5 Flash (High)\nGemini 3.1 Pro (High)\nClaude Opus 4.6 (Thinking)\n",
        );
        assert_eq!(
            agy,
            vec![
                "Gemini 3.5 Flash (High)",
                "Gemini 3.1 Pro (High)",
                "Claude Opus 4.6 (Thinking)"
            ]
        );

        let tabbed = parse_listed_models(
            AgentId::Agy,
            "gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\n",
        );
        assert_eq!(
            tabbed,
            vec!["Gemini 3.8 Flash (Low)", "Gemini 3.8 Flash (High)"]
        );

        let tmp = std::env::temp_dir();
        let args = build_dispatch_args(
            AgentId::Agy,
            "hi",
            Some(&tmp),
            false,
            false,
            false,
            "gemini-3.8-flash-low\tGemini 3.8 Flash (Low)",
        )
        .unwrap();
        assert_eq!(args[0], "--model");
        assert_eq!(args[1], "Gemini 3.8 Flash (Low)");

        let hermes = parse_listed_models(
            AgentId::Hermes,
            r#"{"providers":[{"id":"openrouter","models":[{"id":"anthropic/claude-sonnet-4"},{"name":"openai/gpt-5"}]}]}"#,
        );
        assert_eq!(hermes, vec!["anthropic/claude-sonnet-4", "openai/gpt-5"]);
    }
}
