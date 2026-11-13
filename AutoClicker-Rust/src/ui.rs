//! Oberfläche: index.html und alle Bilder sind in die EXE einkompiliert.
//! Es wird nichts entpackt und nichts von der Festplatte nachgeladen -> sofortiger Start.

const HTML: &str = include_str!("../ui/index.html");
const BG: &[u8] = include_bytes!("../assets/bg.jpg");
const MANTICORE: &[u8] = include_bytes!("../assets/teams/Manticore.png");
const VALKYRA: &[u8] = include_bytes!("../assets/teams/Valkyra.png");
const LONESTAR: &[u8] = include_bytes!("../assets/teams/Lonestar.png");

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = ((chunk[0] as u32) << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn data_uri(mime: &str, data: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64(data))
}

/// Fertige Seite mit eingebetteten Bildern.
pub fn build_html() -> String {
    HTML.replace("{{IMG_BG}}", &data_uri("image/jpeg", BG))
        .replace("{{IMG_MANTICORE}}", &data_uri("image/png", MANTICORE))
        .replace("{{IMG_VALKYRA}}", &data_uri("image/png", VALKYRA))
        .replace("{{IMG_LONESTAR}}", &data_uri("image/png", LONESTAR))
}

/// Eigenes Hintergrundbild: `assets/bg.jpg` (oder bg.png) neben der EXE überschreibt das eingebaute.
/// Wird erst nach dem Laden per Skript gesetzt (WebView2 erlaubt für die Startseite nur ~2 MB).
pub fn custom_background_script() -> Option<String> {
    let dir = std::env::current_exe().ok()?.parent()?.join("assets");
    for (name, mime) in [("bg.jpg", "image/jpeg"), ("bg.png", "image/png")] {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            if !bytes.is_empty() {
                return Some(format!("window.__setBg('{}')", data_uri(mime, &bytes)));
            }
        }
    }
    None
}
