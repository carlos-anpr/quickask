use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
pub const SCREEN_PHRASES: &[&str] = &[
    "pantalla",
    "esta ventana",
    "esta pagina",
    "esta web",
    "lo que pone",
    "lo que sale",
    "lo que veo",
    "lo que ves",
    "lo que aparece",
    "que pone aqui",
    "esto que",
    "este error",
    "este mensaje",
    "screen",
    "captura",
    "lo que tengo delante",
    "que estoy viendo",
];
pub fn needs_screen(text: &str) -> bool {
    let normalized: String = text
        .to_lowercase()
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect();
    SCREEN_PHRASES.iter().any(|p| normalized.contains(p))
}
pub fn greeting(hour: u32) -> String {
    let s = match hour {
        6..=12 => "Buenos días",
        13..=20 => "Buenas tardes",
        _ => "Buenas noches",
    };
    format!("{s}, ¿qué quieres preguntar hoy?")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positives() {
        for s in [
            "Mira mi PANTALLA",
            "en pantalla",
            "esta ventana",
            "esta página",
            "esta web",
            "lo que pone ahí",
            "lo que sale",
            "lo que veo",
            "lo que ves",
            "lo que aparece",
            "¿Qué pone aquí?",
            "esto que sale",
            "este error",
            "este mensaje",
            "screen",
            "captura",
            "lo que tengo delante",
            "qué estoy viendo",
        ] {
            assert!(needs_screen(s), "{s}");
        }
    }
    #[test]
    fn negatives() {
        for s in [
            "Hola",
            "Cuánto es 2+2",
            "Consulta mis correos",
            "Fecha de hoy",
            "Escribe un poema",
            "Qué es Rust",
            "investiga Tokio",
            "resume el documento",
        ] {
            assert!(!needs_screen(s), "{s}");
        }
    }
    #[test]
    fn greetings() {
        for h in 0..24 {
            assert!(greeting(h).starts_with(if (6..13).contains(&h) {
                "Buenos días"
            } else if (13..21).contains(&h) {
                "Buenas tardes"
            } else {
                "Buenas noches"
            }));
        }
    }
}
