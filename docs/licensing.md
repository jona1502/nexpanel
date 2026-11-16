# Lizenzen und Abhängigkeiten

Stand: Entwurf, keine Rechtsberatung. Verbindlich zu prüfen vor der ersten öffentlichen Distribution (siehe ADR-009 in `docs/superpowers/specs/2026-11-16-v02-windows-integration.md`).

## Slint

Gewählter Pfad: **Slint Royalty-free License 2.0**. Bedingung: das `AboutSlint`-Widget ist in einem erreichbaren „Über NexPanel“-Bereich eingebunden. Umsetzung: Einstellungen → „Über NexPanel“ (`apps/desktop/ui/about.slint`). Die Lizenzbedingungen sind vor Veröffentlichung gegen den aktuellen Slint-Lizenztext abzugleichen.

Alternativen: GPLv3 (würde NexPanel selbst unter GPLv3 zwingen) oder kostenpflichtige Slint-Lizenz.

## Lizenz von NexPanel selbst

**Offene Entscheidung.** Es ist keine Lizenz gewählt und es gibt keine `LICENSE`-Datei. Bis dahin gilt kein Nutzungsrecht für Dritte.

## Wichtige direkte Abhängigkeiten

Aus `apps/desktop/Cargo.toml`. Lizenzen nach bestem Wissen, nicht per Tooling verifiziert; transitive Abhängigkeiten sind nicht erfasst (z. B. mit `cargo-deny`/`cargo-about` prüfen).

| Crate | Lizenz (erwartet) | Status |
|---|---|---|
| slint, slint-build | Royalty-free 2.0 / GPLv3 / kommerziell | siehe oben |
| serde, serde_json | MIT OR Apache-2.0 | prüfen |
| log | MIT OR Apache-2.0 | prüfen |
| image | MIT OR Apache-2.0 | prüfen |
| tempfile | MIT OR Apache-2.0 | prüfen |
| windows | MIT OR Apache-2.0 | prüfen |
| wmi | MIT OR Apache-2.0 | prüfen |
| winreg | MIT | prüfen |
| embed-resource | MIT | prüfen |

Transitive Abhängigkeiten von Slint (Renderer, Winit, Schriften) bringen weitere Lizenzen mit; eine Drittlizenz-Übersicht für das Release-Paket steht aus.
