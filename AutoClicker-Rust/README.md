# Wardogs Autoklicker (Rust)

Rust-Port des Python-Autoklickers. Gleiche Oberfläche, gleiche Funktionen
(Wardogs-Teamsuche, Autoklicker, Minecraft Fast Bridge, eigene Hotkeys).

## EXE bauen

1. Einmalig Rust installieren: <https://rustup.rs> (Windows: dabei die "Visual Studio Build Tools" mit
   "Desktopentwicklung mit C++" installieren, wenn rustup danach fragt).
2. `build.bat` doppelklicken -> `dist\WardogsAutoklicker.exe`.

Zum schnellen Testen: `start.bat` (oder `cargo run --release`).

Ohne lokale Installation: Projekt auf GitHub pushen, der Workflow `.github/workflows/build.yml`
baut die EXE auf einem Windows-Server (Actions -> Artefakt "WardogsAutoklicker").

Voraussetzung zum Ausführen: Windows 10/11 mit der Edge-**WebView2-Runtime** (auf Windows 11 vorinstalliert).

## Aufbau

| Datei | Aufgabe |
|---|---|
| `src/main.rs` | Fenster, WebView, Verbindung zur Oberfläche |
| `src/engine.rs` | Autoklicker, Fast Bridge, Wardogs-Suche, Hotkeys |
| `src/matcher.rs` | Bilderkennung (Ersatz für OpenCV), parallel & grob-zu-fein |
| `src/capture.rs` | Screenshot per GDI (Ersatz für mss) |
| `src/input.rs` | Maus/Tastatur senden + globale Hotkeys (Ersatz für pynput) |
| `src/overlay.rs` | Countdown-Rahmen (Ersatz für Tkinter) |
| `ui/index.html` | Oberfläche (unverändertes Design) |
| `assets/` | Logos + Hintergrund, werden in die EXE einkompiliert |

Eigenes Hintergrundbild: `assets\bg.jpg` neben die EXE legen (überschreibt das eingebaute).
