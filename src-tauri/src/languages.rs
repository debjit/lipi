use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LanguageOption {
    pub code: String,
    pub name: String,
    pub native: String,
}

struct Lang(&'static str, &'static str, &'static str);

/// Whisper's multilingual language set (99 codes).
const WHISPER: &[Lang] = &[
    Lang("en", "English", "English"),
    Lang("zh", "Chinese", "中文"),
    Lang("de", "German", "Deutsch"),
    Lang("es", "Spanish", "Español"),
    Lang("ru", "Russian", "Русский"),
    Lang("ko", "Korean", "한국어"),
    Lang("fr", "French", "Français"),
    Lang("ja", "Japanese", "日本語"),
    Lang("pt", "Portuguese", "Português"),
    Lang("tr", "Turkish", "Türkçe"),
    Lang("pl", "Polish", "Polski"),
    Lang("ca", "Catalan", "Català"),
    Lang("nl", "Dutch", "Nederlands"),
    Lang("ar", "Arabic", "العربية"),
    Lang("sv", "Swedish", "Svenska"),
    Lang("it", "Italian", "Italiano"),
    Lang("id", "Indonesian", "Bahasa Indonesia"),
    Lang("hi", "Hindi", "हिन्दी"),
    Lang("fi", "Finnish", "Suomi"),
    Lang("vi", "Vietnamese", "Tiếng Việt"),
    Lang("he", "Hebrew", "עברית"),
    Lang("uk", "Ukrainian", "Українська"),
    Lang("el", "Greek", "Ελληνικά"),
    Lang("ms", "Malay", "Bahasa Melayu"),
    Lang("cs", "Czech", "Čeština"),
    Lang("ro", "Romanian", "Română"),
    Lang("da", "Danish", "Dansk"),
    Lang("hu", "Hungarian", "Magyar"),
    Lang("ta", "Tamil", "தமிழ்"),
    Lang("no", "Norwegian", "Norsk"),
    Lang("th", "Thai", "ไทย"),
    Lang("ur", "Urdu", "اردو"),
    Lang("hr", "Croatian", "Hrvatski"),
    Lang("bg", "Bulgarian", "Български"),
    Lang("lt", "Lithuanian", "Lietuvių"),
    Lang("la", "Latin", "Latina"),
    Lang("mi", "Maori", "Māori"),
    Lang("ml", "Malayalam", "മലയാളം"),
    Lang("cy", "Welsh", "Cymraeg"),
    Lang("sk", "Slovak", "Slovenčina"),
    Lang("te", "Telugu", "తెలుగు"),
    Lang("fa", "Persian", "فارسی"),
    Lang("lv", "Latvian", "Latviešu"),
    Lang("bn", "Bengali", "বাংলা"),
    Lang("sr", "Serbian", "Српски"),
    Lang("az", "Azerbaijani", "Azərbaycan"),
    Lang("sl", "Slovenian", "Slovenščina"),
    Lang("kn", "Kannada", "ಕನ್ನಡ"),
    Lang("et", "Estonian", "Eesti"),
    Lang("mk", "Macedonian", "Македонски"),
    Lang("br", "Breton", "Brezhoneg"),
    Lang("eu", "Basque", "Euskara"),
    Lang("is", "Icelandic", "Íslenska"),
    Lang("hy", "Armenian", "Հայերեն"),
    Lang("ne", "Nepali", "नेपाली"),
    Lang("mn", "Mongolian", "Монгол"),
    Lang("bs", "Bosnian", "Bosanski"),
    Lang("kk", "Kazakh", "Қазақ"),
    Lang("sq", "Albanian", "Shqip"),
    Lang("sw", "Swahili", "Kiswahili"),
    Lang("gl", "Galician", "Galego"),
    Lang("mr", "Marathi", "मराठी"),
    Lang("pa", "Punjabi", "ਪੰਜਾਬੀ"),
    Lang("si", "Sinhala", "සිංහල"),
    Lang("km", "Khmer", "ខ្មែរ"),
    Lang("sn", "Shona", "chiShona"),
    Lang("yo", "Yoruba", "Yorùbá"),
    Lang("so", "Somali", "Soomaali"),
    Lang("af", "Afrikaans", "Afrikaans"),
    Lang("oc", "Occitan", "Occitan"),
    Lang("ka", "Georgian", "ქართული"),
    Lang("be", "Belarusian", "Беларуская"),
    Lang("tg", "Tajik", "Тоҷикӣ"),
    Lang("sd", "Sindhi", "سنڌي"),
    Lang("gu", "Gujarati", "ગુજરાતી"),
    Lang("am", "Amharic", "አማርኛ"),
    Lang("yi", "Yiddish", "ייִדיש"),
    Lang("lo", "Lao", "ລາວ"),
    Lang("uz", "Uzbek", "Oʻzbek"),
    Lang("fo", "Faroese", "Føroyskt"),
    Lang("ht", "Haitian Creole", "Kreyòl ayisyen"),
    Lang("ps", "Pashto", "پښتو"),
    Lang("tk", "Turkmen", "Türkmen"),
    Lang("nn", "Nynorsk", "Nynorsk"),
    Lang("mt", "Maltese", "Malti"),
    Lang("sa", "Sanskrit", "संस्कृतम्"),
    Lang("lb", "Luxembourgish", "Lëtzebuergesch"),
    Lang("my", "Myanmar", "မြန်မာ"),
    Lang("bo", "Tibetan", "བོད་སྐད"),
    Lang("tl", "Tagalog", "Tagalog"),
    Lang("mg", "Malagasy", "Malagasy"),
    Lang("as", "Assamese", "অসমীয়া"),
    Lang("tt", "Tatar", "Татар"),
    Lang("haw", "Hawaiian", "ʻŌlelo Hawaiʻi"),
    Lang("ln", "Lingala", "Lingála"),
    Lang("ha", "Hausa", "Hausa"),
    Lang("ba", "Bashkir", "Башҡорт"),
    Lang("jw", "Javanese", "Basa Jawa"),
    Lang("su", "Sundanese", "Basa Sunda"),
];

fn option(lang: &Lang) -> LanguageOption {
    LanguageOption {
        code: lang.0.to_string(),
        name: lang.1.to_string(),
        native: lang.2.to_string(),
    }
}

fn is_english_only(engine_mode: &str, model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    let marked = id.contains(".en") || id.ends_with("-en") || id.contains("english-only");
    if engine_mode == "local" {
        // Bundled sizes (tiny, base, small, medium, turbo_q8) are multilingual.
        id.contains(".en")
    } else {
        marked
    }
}

const PARAKEET: &[&str] = &[
    "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt",
    "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
];

pub fn languages_for(engine_mode: &str, model_id: &str) -> Vec<LanguageOption> {
    if model_id == "parakeet" {
        let mut list: Vec<LanguageOption> = WHISPER
            .iter()
            .filter(|lang| PARAKEET.contains(&lang.0))
            .map(option)
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        return list;
    }
    if is_english_only(engine_mode, model_id) {
        return WHISPER.iter().filter(|lang| lang.0 == "en").map(option).collect();
    }
    let mut list: Vec<LanguageOption> = WHISPER.iter().map(option).collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// A language code Whisper can be asked to use for this model. Empty and unknown codes become auto-detect.
pub fn for_asr<'a>(engine_mode: &str, model_id: &str, code: Option<&'a str>) -> Option<&'a str> {
    let code = code.map(str::trim).filter(|c| !c.is_empty())?;
    let allowed = languages_for(engine_mode, model_id);
    if allowed.iter().any(|lang| lang.code == code) {
        Some(code)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_catalog_has_99_languages_including_bengali() {
        assert_eq!(WHISPER.len(), 99);
        let codes: Vec<&str> = WHISPER.iter().map(|lang| lang.0).collect();
        assert!(codes.contains(&"bn"));
        assert!(codes.contains(&"en"));
        let mut unique = codes.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 99);
    }

    #[test]
    fn bundled_local_models_keep_the_full_list() {
        let langs = languages_for("local", "base");
        assert_eq!(langs.len(), 99);
        assert!(langs.iter().any(|lang| lang.code == "bn" && lang.native == "বাংলা"));
    }

    #[test]
    fn english_only_model_ids_shrink_the_list() {
        assert_eq!(languages_for("cloud", "whisper-tiny.en").len(), 1);
        assert_eq!(languages_for("cloud", "whisper-tiny.en")[0].code, "en");
        assert_eq!(languages_for("local", "tiny.en").len(), 1);
        assert_eq!(languages_for("cloud", "whisper-large-v3").len(), 99);
    }

    #[test]
    fn unknown_codes_fall_back_to_auto_detect() {
        assert_eq!(for_asr("local", "base", Some("bn")), Some("bn"));
        assert_eq!(for_asr("local", "base", Some("")), None);
        assert_eq!(for_asr("local", "base", Some("zz")), None);
        assert_eq!(for_asr("cloud", "whisper-tiny.en", Some("bn")), None);
        assert_eq!(for_asr("cloud", "whisper-tiny.en", Some("en")), Some("en"));
    }

    #[test]
    fn parakeet_lists_its_twenty_five_languages() {
        let langs = languages_for("local", "parakeet");
        assert_eq!(langs.len(), 25);
        assert!(!langs.iter().any(|lang| lang.code == "bn" || lang.code == "hi"));
        assert_eq!(for_asr("local", "parakeet", Some("bn")), None);
        assert_eq!(for_asr("local", "parakeet", Some("en")), Some("en"));
    }
}
