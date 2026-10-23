import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface LanguageOption {
  code: string;
  name: string;
  native: string;
}

const RECENT_KEY = "lipi.language.recent";

export function LanguagePicker({
  value,
  engineMode,
  modelId,
  onChange,
}: {
  value: string;
  engineMode: string;
  modelId: string;
  onChange: (code: string) => void;
}) {
  const [options, setOptions] = useState<LanguageOption[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [recent, setRecent] = useState(() => localStorage.getItem(RECENT_KEY) || "");

  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    invoke<LanguageOption[]>("list_languages", { engineMode, modelId })
      .then((list) => {
        if (!cancelled) setOptions(list);
      })
      .catch(() => {
        if (!cancelled) setOptions([]);
      })
      .finally(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [engineMode, modelId]);

  useEffect(() => {
    if (!loaded || options.length === 0) return;
    if (value && !options.some((lang) => lang.code === value)) {
      onChange("");
    }
  }, [loaded, options, value]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return options;
    return options.filter((lang) =>
      lang.name.toLowerCase().includes(q) ||
      lang.native.toLowerCase().includes(q) ||
      lang.code.toLowerCase().includes(q)
    );
  }, [options, query]);

  function choose(code: string) {
    if (code) {
      localStorage.setItem(RECENT_KEY, code);
      setRecent(code);
    }
    setQuery("");
    setOpen(false);
    onChange(code);
  }

  const chips = [
    { code: "", label: "Auto-detect" },
    { code: "en", label: "English" },
    { code: "bn", label: "Bengali (বাংলা)" },
    { code: "hi", label: "Hindi (हिन्दी)" },
  ].filter((chip) => chip.code === "" || options.some((lang) => lang.code === chip.code));
  if (recent && recent !== "en" && recent !== "bn" && recent !== "hi" && options.some((lang) => lang.code === recent)) {
    const lang = options.find((item) => item.code === recent);
    chips.push({ code: recent, label: lang ? lang.name : recent });
  }

  const englishOnly = loaded && options.length === 1 && options[0]?.code === "en";

  return (
    <>
      <div className="form-group">
        <label className="form-label">Spoken Language</label>
        <div className="language-search">
          <input
            type="text"
            className="form-input"
            placeholder={englishOnly ? "English" : "Search languages"}
            value={open ? query : value ? labelFor(options, value) : ""}
            onFocus={() => setOpen(true)}
            onChange={(e) => {
              setQuery(e.target.value);
              setOpen(true);
            }}
            onBlur={() => setOpen(false)}
          />
          {open && (
            <div className="language-menu" role="listbox">
              <button type="button" className="language-option" onMouseDown={() => choose("")}>
                Auto-detect
              </button>
              {filtered.map((lang) => (
                <button
                  key={lang.code}
                  type="button"
                  className={`language-option ${value === lang.code ? "active" : ""}`}
                  onMouseDown={() => choose(lang.code)}
                >
                  <span>{lang.name}</span>
                  <span className="language-option-meta">
                    {lang.native !== lang.name ? lang.native : lang.code}
                  </span>
                </button>
              ))}
              {filtered.length === 0 && <div className="language-empty">No matching language</div>}
            </div>
          )}
        </div>
        <span className="form-hint">
          {englishOnly
            ? "This model transcribes English."
            : "Choosing a language skips Whisper's detection step. The list is limited to languages this model can transcribe."}
        </span>
      </div>
      <div className="language-quick-chips">
        {chips.map((lang) => (
          <button
            key={lang.code || "auto"}
            type="button"
            className={`provider-chip ${value === lang.code ? "active" : ""}`}
            style={{ fontSize: "11px", padding: "3px 8px" }}
            onClick={() => choose(lang.code)}
          >
            {lang.label}
          </button>
        ))}
      </div>
    </>
  );
}

function labelFor(options: LanguageOption[], code: string): string {
  const lang = options.find((item) => item.code === code);
  if (!lang) return code;
  return lang.native !== lang.name ? `${lang.name} (${lang.native})` : lang.name;
}
