use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
/// Modelo seleccionable, tal y como lo describe `GET /api/model` de OpenCode.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub label: String,
    pub free: bool,
    pub vision: bool,
    pub variants: Vec<String>,
}
/// Modelo gratuito de OpenCode Zen: funciona sin suscripción ni clave propia.
pub const DEFAULT_MODEL: &str = "opencode/deepseek-v4.1-flash-free";
/// Sin relevo por defecto.
pub const DEFAULT_FALLBACK: &str = "";
/// Variables de entorno que mandan sobre `config.toml` (ver README).
pub const ENV_MODEL: &str = "QUICKASK_MODEL";
pub const ENV_FALLBACK: &str = "QUICKASK_FALLBACK_MODEL";
pub const ENV_PROVIDERS: &str = "QUICKASK_MODEL_PROVIDERS";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub shortcut: String,
    pub model: String,
    pub fallback_model: String,
    /// Proveedores cuyo catálogo completo sale en el selector. El modelo y el
    /// relevo configurados se añaden siempre, aunque su proveedor no esté aquí.
    pub model_providers: Vec<String>,
    pub variant: String,
    pub workdir: String,
    pub hide_on_blur: bool,
    pub notify_when_hidden: bool,
    pub screen_auto: bool,
    pub theme: String,
    pub ask_permissions: Vec<String>,
    /// Salida de seguridad del turno (ver watchdog.rs).
    pub turn_timeout_secs: u64,
    pub stall_timeout_secs: u64,
    pub max_tool_calls: usize,
    pub max_step_repeats: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            shortcut: "<Control><Alt>space".into(),
            model: DEFAULT_MODEL.into(),
            fallback_model: DEFAULT_FALLBACK.into(),
            model_providers: vec!["opencode".into()],
            variant: String::new(),
            workdir: "~/".into(),
            hide_on_blur: true,
            notify_when_hidden: true,
            screen_auto: true,
            theme: "system".into(),
            ask_permissions: vec![],
            turn_timeout_secs: 300,
            stall_timeout_secs: 180,
            max_tool_calls: 40,
            max_step_repeats: 3,
        }
    }
}
/// Nombre corto para avisos: la parte tras el proveedor (`deepseek-v4.1-flash`).
pub fn model_label(model: &str) -> &str {
    model.split_once('/').map(|(_, id)| id).unwrap_or(model)
}
/// Formato `proveedor/modelo`; la existencia real se comprueba contra el catálogo de OpenCode.
pub fn validate_model(model: &str) -> Result<()> {
    match model.split_once('/') {
        Some((p, m)) if !p.is_empty() && !m.is_empty() => Ok(()),
        _ => bail!("Modelo no disponible"),
    }
}
impl Config {
    pub fn load() -> Result<Self> {
        Self::load_path(&Self::path()?)
    }
    pub fn path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("No se encuentra el directorio de configuración")?
            .join("quickask/config.toml"))
    }
    pub fn validate_variant(variant: &str) -> Result<()> {
        // Cada modelo declara sus propias variantes (none, minimal, xhigh, thinking…);
        // aquí solo se valida el formato y el catálogo decide si existe.
        if variant.len() > 16 || !variant.chars().all(|c| c.is_ascii_lowercase()) {
            bail!("Esfuerzo inválido");
        }
        Ok(())
    }
    pub fn variant_document(source: &str, variant: &str) -> Result<String> {
        Self::validate_variant(variant)?;
        Self::string_document(source, "variant", variant)
    }
    pub fn model_document(source: &str, model: &str, fallback: &str) -> Result<String> {
        validate_model(model)?;
        if !fallback.is_empty() {
            validate_model(fallback)?;
        }
        if model == fallback {
            bail!("El modelo de respaldo debe ser distinto del principal");
        }
        let source = Self::string_document(source, "model", model)?;
        Self::string_document(&source, "fallback_model", fallback)
    }
    pub fn string_document(source: &str, key: &str, value: &str) -> Result<String> {
        let mut doc: toml_edit::Document = source.parse().context("config.toml inválido")?;
        let mut replacement = toml_edit::value(value);
        if let (Some(old), Some(new)) = (
            doc.get(key).and_then(toml_edit::Item::as_value),
            replacement.as_value_mut(),
        ) {
            *new.decor_mut() = old.decor().clone();
        }
        doc[key] = replacement;
        Ok(doc.to_string())
    }
    pub fn save_variant_at(path: &Path, variant: &str) -> Result<()> {
        let updated = Self::variant_document(&std::fs::read_to_string(path)?, variant)?;
        Self::write_at(path, &updated)
    }
    pub fn write_at(path: &Path, content: &str) -> Result<()> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let parent = path.parent().context("Ruta de configuración inválida")?;
        let temp = parent.join(format!(".config-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
    pub fn load_path(path: &Path) -> Result<Self> {
        Self::load_path_with(path, |key| std::env::var(key).ok())
    }
    /// `env` resuelve las variables `QUICKASK_*`; se inyecta para poder probarlo.
    pub fn load_path_with(path: &Path, env: impl Fn(&str) -> Option<String>) -> Result<Self> {
        if !path.exists() {
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(path, toml::to_string_pretty(&Self::default())?)?;
        }
        let source = std::fs::read_to_string(path)?;
        let mut c: Self = toml::from_str(&source).context("config.toml inválido")?;
        c.apply_env(env);
        c.model_parts()?;
        if !c.fallback_model.is_empty() {
            Self::parts(&c.fallback_model)?;
        }
        Self::validate_variant(&c.variant)?;
        for provider in &c.model_providers {
            if provider.is_empty() || provider.contains('/') {
                bail!("model_providers debe contener nombres de proveedor sin '/'");
            }
        }
        if !["system", "dark", "light"].contains(&c.theme.as_str()) {
            bail!("theme debe ser system, dark o light");
        }
        if c.turn_timeout_secs == 0
            || c.stall_timeout_secs == 0
            || c.max_tool_calls == 0
            || c.max_step_repeats < 2
        {
            bail!("límites del turno inválidos (tiempos > 0, max_tool_calls > 0, max_step_repeats >= 2)");
        }
        c.workdir = expand_home(&c.workdir)?.to_string_lossy().into_owned();
        if !Path::new(&c.workdir).is_absolute() || !Path::new(&c.workdir).is_dir() {
            bail!("workdir debe ser un directorio absoluto existente");
        }
        Ok(c)
    }
    fn apply_env(&mut self, env: impl Fn(&str) -> Option<String>) {
        let set = |key| env(key).map(|v: String| v.trim().to_owned());
        if let Some(model) = set(ENV_MODEL).filter(|m| !m.is_empty()) {
            self.model = model;
        }
        // Vacía = sin relevo, así que aquí sí se respeta la cadena vacía.
        if let Some(fallback) = set(ENV_FALLBACK) {
            self.fallback_model = fallback;
        }
        if let Some(providers) = set(ENV_PROVIDERS).filter(|p| !p.is_empty()) {
            self.model_providers = providers
                .split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect();
        }
        if self.fallback_model == self.model {
            self.fallback_model.clear();
        }
    }
    pub fn parts(model: &str) -> Result<(&str, &str)> {
        match model.split_once('/') {
            Some((p, m)) if !p.is_empty() && !m.is_empty() => Ok((p, m)),
            _ => bail!("Modelo inválido: se espera proveedor/modelo"),
        }
    }
    pub fn model_parts(&self) -> Result<(&str, &str)> {
        Self::parts(&self.model)
    }
}
fn expand_home(value: &str) -> Result<PathBuf> {
    if value == "~" || value.starts_with("~/") {
        Ok(dirs::home_dir()
            .context("No se encuentra HOME")?
            .join(value.trim_start_matches('~').trim_start_matches('/')))
    } else {
        Ok(PathBuf::from(value))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_partial() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let c = Config::load_path_with(&p, |_| None).unwrap();
        assert_eq!(
            c.model_parts().unwrap(),
            ("opencode", "deepseek-v4.1-flash-free")
        );
        assert_eq!(c.fallback_model, "");
        assert_eq!(c.model_providers, ["opencode"]);
        assert!(c.hide_on_blur);
        std::fs::write(&p, "screen_auto = false\nmodel='a/b/c'\n").unwrap();
        let c = Config::load_path_with(&p, |_| None).unwrap();
        assert!(!c.screen_auto);
        assert_eq!(c.model_parts().unwrap(), ("a", "b/c"));
        assert!(Path::new(&c.workdir).is_absolute());
        // Lista explícita y validación de nombres de proveedor.
        std::fs::write(&p, "model_providers = ['opencode-go']\n").unwrap();
        assert_eq!(
            Config::load_path_with(&p, |_| None)
                .unwrap()
                .model_providers,
            ["opencode-go"]
        );
        std::fs::write(&p, "model_providers = ['opencode-go/x']\n").unwrap();
        assert!(Config::load_path_with(&p, |_| None).is_err());
    }
    #[test]
    fn env_overrides_model_fallback_and_providers() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "model = 'a/b'\nfallback_model = 'c/d'\n").unwrap();
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let c = Config::load_path_with(&p, env(&[])).unwrap();
        assert_eq!(
            (c.model.as_str(), c.fallback_model.as_str()),
            ("a/b", "c/d")
        );
        let c = Config::load_path_with(
            &p,
            env(&[
                (ENV_MODEL, " ollama-cloud/glm-5.3 "),
                (ENV_FALLBACK, ""),
                (ENV_PROVIDERS, "ollama-cloud, opencode-go"),
            ]),
        )
        .unwrap();
        assert_eq!(c.model, "ollama-cloud/glm-5.3");
        assert_eq!(c.fallback_model, "");
        assert_eq!(c.model_providers, ["ollama-cloud", "opencode-go"]);
        // Un modelo vacío no pisa el del fichero; un relevo igual al modelo se descarta.
        let c = Config::load_path_with(&p, env(&[(ENV_MODEL, ""), (ENV_FALLBACK, "a/b")])).unwrap();
        assert_eq!((c.model.as_str(), c.fallback_model.as_str()), ("a/b", ""));
        assert!(Config::load_path_with(&p, env(&[(ENV_MODEL, "sin-proveedor")])).is_err());
        // El fichero no se reescribe con lo que venga del entorno.
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "model = 'a/b'\nfallback_model = 'c/d'\n"
        );
    }
    #[test]
    fn model_document_rewrites_both_keys_and_keeps_the_rest() {
        let source = "# preferencias\nmodel = 'opencode/deepseek-v4.1-flash-free' # principal\nfallback_model = \"opencode-go/deepseek-v4.1-flash\"\n[extra]\nvalue = 1\n";
        let updated = Config::model_document(
            source,
            "opencode-go/deepseek-v4.1-flash",
            "opencode/deepseek-v4.1-flash-free",
        )
        .unwrap();
        assert_eq!(
            updated,
            source
                .replace(
                    "model = 'opencode/deepseek-v4.1-flash-free' # principal",
                    "model = \"opencode-go/deepseek-v4.1-flash\" # principal"
                )
                .replace(
                    "fallback_model = \"opencode-go/deepseek-v4.1-flash\"",
                    "fallback_model = \"opencode/deepseek-v4.1-flash-free\""
                )
        );
        assert_eq!(
            Config::model_document("theme = 'dark'\n", "opencode/deepseek-v4.1-flash-free", "").unwrap(),
            "theme = 'dark'\nmodel = \"opencode/deepseek-v4.1-flash-free\"\nfallback_model = \"\"\n"
        );
        for (model, fallback) in [
            ("sin-proveedor", "opencode-go/deepseek-v4.1-flash"),
            ("opencode/deepseek-v4.1-flash-free", "roto/"),
            (
                "opencode/deepseek-v4.1-flash-free",
                "opencode/deepseek-v4.1-flash-free",
            ),
        ] {
            assert!(Config::model_document("", model, fallback).is_err());
        }
    }
    #[test]
    fn model_labels_and_formats() {
        assert_eq!(
            model_label("opencode/deepseek-v4.1-flash-free"),
            "deepseek-v4.1-flash-free"
        );
        assert_eq!(model_label("sinproveedor"), "sinproveedor");
        assert!(validate_model("opencode-go/kimi-k3").is_ok());
        for bad in ["", "a", "/b", "a/"] {
            assert!(validate_model(bad).is_err(), "{bad}");
        }
        for ok in ["", "none", "minimal", "xhigh", "thinking", "max"] {
            assert!(Config::validate_variant(ok).is_ok(), "{ok}");
        }
        for bad in ["High", "x-high", "muy alto", "aaaaaaaaaaaaaaaaa"] {
            assert!(Config::validate_variant(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn invalid_model() {
        for m in ["", "foo", "/bar", "foo/"] {
            let c = Config {
                model: m.into(),
                ..Default::default()
            };
            assert!(c
                .model_parts()
                .unwrap_err()
                .to_string()
                .contains("Modelo inválido"));
        }
    }
    #[test]
    fn variant_rewrite_preserves_other_values_comments_and_tables() {
        let source = "# preferencias\nmodel = 'provider/model'\nvariant = \"\" # esfuerzo\nworkdir = '~/ruta con espacios'\nask_permissions = ['shell']\n[extra]\nvalue = 123 # futuro\n";
        let updated = Config::variant_document(source, "high").unwrap();
        assert_eq!(
            updated,
            source.replace("variant = \"\"", "variant = \"high\"")
        );
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, source).unwrap();
        Config::save_variant_at(&p, "high").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), updated);
        Config::save_variant_at(&p, "").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), source);
        assert!(Config::save_variant_at(&p, "In valid").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), source);
    }
    #[test]
    fn variant_can_be_added_to_partial_config_without_touching_tables() {
        let source = "theme = 'dark'\n[extra]\nvariant = 'keep'\n";
        let updated = Config::variant_document(source, "max").unwrap();
        let doc: toml::Value = toml::from_str(&updated).unwrap();
        assert_eq!(doc["variant"].as_str(), Some("max"));
        assert_eq!(doc["extra"]["variant"].as_str(), Some("keep"));
        assert!(updated.contains("theme = 'dark'"));
    }
}
