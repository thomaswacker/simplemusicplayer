# Simple Music Player

Ein kleiner, ordnerbasierter Musikspieler für den Linux-Desktop (KDE Plasma), geschrieben in Rust mit egui.

- Startordner wählen, Unterordner durchblättern, Titel anklicken: der Ordner wird zur Wiedergabeliste
- Play/Pause, vorheriger/nächster Titel, Spulen, Lautstärke, Stummschalten
- Nach dem Ende eines Titels geht es automatisch mit dem nächsten weiter
- Beim Beenden merkt er sich Titel und Stelle; beim nächsten Start steht der Titel pausiert dort bereit
- Medientasten und das Plasma-Medienwidget funktionieren über MPRIS, auch wenn das Fenster minimiert ist
- Formate: MP3, FLAC, Ogg Vorbis, Opus, M4A/AAC, WAV, AIFF

## Installation

```sh
./install.sh             # baut ein Release und installiert nach ~/.local
./install.sh --uninstall # entfernt Programm, Icon und Startmenü-Eintrag
```

Voraussetzungen zum Bauen (Debian Trixie): `cargo`, `pkg-config`, `libasound2-dev`, `libopus-dev`, `librsvg2-bin` (für die Icon-Größen).

Danach liegt die App im Startmenü unter Multimedia. Sie lässt sich auch direkt starten, mit optionalem Startordner:

```sh
simplemusicplayer ~/Musik
```

## Bedienung

| Taste                         | Aktion                                  |
|-------------------------------|-----------------------------------------|
| Leertaste                     | Abspielen / Pause                       |
| N  oder  Strg+→               | Nächster Titel                          |
| P  oder  Strg+←               | Vorheriger Titel (nach 3 s: Titelanfang) |
| ← / →                         | 10 s zurück / vor                       |
| + / −  oder  ↑ / ↓            | Lauter / leiser                         |
| M                             | Stummschalten                           |
| Rücktaste                     | Übergeordneter Ordner                   |
| F5                            | Ordner neu einlesen                     |
| Medientasten (Play, ⏭, ⏮, Stop) | systemweit über MPRIS                 |

Die Lautstärketasten der Tastatur regeln in KDE die Systemlautstärke. Die Player-Lautstärke lässt sich im Fenster und im Plasma-Medienwidget einstellen.

Die Einstellungen (Startordner, letzter Ordner, Lautstärke) liegen in `~/.config/simplemusicplayer/config.json`, die Wiedergabeposition in `~/.local/state/simplemusicplayer/resume.json`.

## Entwicklung

```sh
cargo run -- ~/Musik
cargo test
# Wiedergabe-Integrationstest (stumm, braucht ein Audiogerät):
SMP_TEST_DIR=/pfad/zu/kurzen/titeln SMP_TEST_LONG_DIR=/pfad/zu/titeln/ab/5s \
  cargo test -- --ignored --test-threads=1
```

Aufbau:

- `src/player.rs`: Wiedergabe-Thread, dem alle Audiozustände gehören. Die Oberfläche schickt nur Befehle und liest einen Snapshot. Deshalb laufen Auto-Weiter und Medientasten auch ohne sichtbares Fenster. Geht das Audiogerät verloren, wird es neu geöffnet.
- `src/media_keys.rs`: MPRIS (D-Bus) über souvlaki/zbus
- `src/opus.rs`: Opus-Dekodierung über libopus (rodio kann kein Opus)
- `src/library.rs`: Ordnerliste mit natürlicher Sortierung („2“ vor „10“)
- `src/app.rs`, `src/icons.rs`: Oberfläche und Vektor-Icons
