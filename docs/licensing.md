# Lizenzen und Abhängigkeiten

Stand: Entwurf, keine Rechtsberatung. Verbindlich zu prüfen vor der ersten öffentlichen Distribution (siehe ADR-009 in `docs/superpowers/specs/2026-11-16-v02-windows-integration.md`).

## Slint

Gewählter Pfad: **Slint Royalty-free License 2.0**. Bedingung: das `AboutSlint`-Widget ist in einem erreichbaren „Über NexPanel“-Bereich eingebunden. Umsetzung: Einstellungen → „Über NexPanel“ (`apps/desktop/ui/about.slint`). Die Lizenzbedingungen sind vor Veröffentlichung gegen den aktuellen Slint-Lizenztext abzugleichen.

Alternativen: GPLv3 (würde NexPanel selbst unter GPLv3 zwingen) oder kostenpflichtige Slint-Lizenz.

## Lizenz von NexPanel selbst

**Offene Entscheidung.** Es ist keine Lizenz gewählt und es gibt keine `LICENSE`-Datei. Bis dahin gilt kein Nutzungsrecht für Dritte.

## Abhängigkeiten (verifiziert)

Quelle: `cargo metadata --format-version 1 --locked` (Stand `Cargo.lock` auf `main`, Slint 1.18.1), ausgewertet für das Ziel `x86_64-pc-windows-msvc`. Es wurde nichts kompiliert. Maßgeblich ist das `license`-Feld der jeweiligen `Cargo.toml`; die Lizenztexte der Crates selbst wurden nicht einzeln geprüft. Keine Rechtsberatung.

Hinweis: Die Auflösung ist Feature-vereinigt und Cargo-metadata-basiert. Sie kann Crates enthalten, die im fertigen Binary nicht gelinkt sind (z. B. `i-slint-backend-testing`, `i-slint-renderer-skia`, `i-slint-compiler`). Die Zahlen sind eine konservative Obergrenze. Für die Release-Übersicht das Tooling unten gegen das gebaute Ziel laufen lassen.

### Direkte Abhängigkeiten `apps/desktop`

| Crate | Version | SPDX (`license`) | Art |
|---|---|---|---|
| slint | 1.18.1 | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | normal, siehe Slint |
| slint-build | 1.18.1 | wie slint | build |
| serde | 1.0.229 | `MIT OR Apache-2.0` | normal |
| serde_json | 1.0.154 | `MIT OR Apache-2.0` | normal |
| log | 0.4.35 | `MIT OR Apache-2.0` | normal |
| image | 0.25.10 | `MIT OR Apache-2.0` | normal |
| tempfile | 3.27.0 | `MIT OR Apache-2.0` | normal |
| wmi | 0.18.4 | `MIT OR Apache-2.0` | normal (Windows) |
| winreg | 0.55.0 | `MIT` | normal (Windows) |
| windows | 0.62.2 | `MIT OR Apache-2.0` | normal (Windows) |
| windows-core | 0.62.2 | `MIT OR Apache-2.0` | normal (Windows) |
| embed-resource | 3.0.12 | `MIT` | build |

### Direkte Abhängigkeiten `spikes/ddc`

| Crate | Version | SPDX |
|---|---|---|
| windows | 0.62.2 | `MIT OR Apache-2.0` |

Transitiv (normal, Windows-Ziel): 15 Crates, davon 14 `MIT OR Apache-2.0` und `unicode-ident` (`(MIT OR Apache-2.0) AND Unicode-3.0`). Der Spike wird nicht ausgeliefert.

### Transitive Abhängigkeiten `apps/desktop` (normal, Windows-Ziel, ohne Workspace-Crates)

344 Crates, nach SPDX-Ausdruck:

| Anzahl | SPDX |
|---|---|
| 161 | `MIT OR Apache-2.0` |
| 53 | `MIT` |
| 44 | `Apache-2.0 OR MIT` |
| 27 | `Unicode-3.0` |
| 11 | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` (alle `slint`/`i-slint-*`) |
| 9 | `Apache-2.0` (accesskit_winit, codespan-reporting, gethostname, glutin, glutin_egl_sys, glutin_wgl_sys, spirv, unicode-linebreak, winit) |
| 6 | `MIT OR Apache-2.0 OR Zlib` |
| 5 | `MIT/Apache-2.0` (alte Schreibweise für `MIT OR Apache-2.0`) |
| 3 | `Zlib` (foldhash, slotmap, zlib-rs) |
| 3 | `Zlib OR Apache-2.0 OR MIT` |
| 2 | `BSD-3-Clause OR Apache-2.0` (moxcms, pxfm) |
| 2 | `BSD-3-Clause OR MIT OR Apache-2.0` |
| 2 | `MIT OR Zlib OR Apache-2.0` |
| 2 | `BSD-3-Clause` (tiny-skia, tiny-skia-path) |
| 2 | `BSL-1.0` (clipboard-win, error-code) |
| 2 | `BSD-2-Clause OR Apache-2.0 OR MIT` |
| 2 | `Unlicense OR MIT` (byteorder-lite, memchr) |
| je 1 | `Apache-2.0/MIT` (rustc-hash 1.1.0), `0BSD OR MIT OR Apache-2.0` (adler2), `ISC` (libloading), `BSD-2-Clause` (arrayref), `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` (rustix), `Apache-2.0 AND MIT` (dpi), `CC0-1.0 OR Apache-2.0` (imgref), `(MIT OR Apache-2.0) AND Unicode-3.0` (unicode-ident) |

Build-only: `embed-resource` (MIT) und `slint-build`. Deren eigene transitive Build-Abhängigkeiten (u. a. Slint-Compiler) gehen nicht ins Binary und sind hier nicht aufgelistet.

### Auffälligkeiten und Pflichten

- Nicht gefunden: Crates ohne `license`, Crates nur mit `license_file`, GPL/LGPL/AGPL/MPL/EUPL in der Windows-Normal-Auflösung (außer der Slint-Wahlmöglichkeit unten).
- **Slint (11 Crates, `slint` und `i-slint-*`)**: Dreifachlizenz `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`. Für proprietäre Distribution nur über Royalty-free 2.0 (Bedingung: `AboutSlint`, siehe oben) oder die kostenpflichtige Lizenz; GPL-3.0 wäre Copyleft. Der Royalty-free-Text ist kein SPDX-Standard und vor Release gegen den aktuellen Text zu prüfen.
- **Unicode-3.0 (27 + `unicode-ident`)**: Daten-/ICU-Crates (`icu_*`). Permissiv, verlangt aber Copyright- und Erlaubnishinweis in der Dokumentation bzw. den Drittlizenzhinweisen.
- **Apache-2.0-only (9 Crates)**: Lizenztext und NOTICE-Dateien (falls vorhanden) beilegen; keine MIT-Alternative.
- **BSL-1.0 (clipboard-win, error-code)**: Hinweis nur bei Quellcode-Weitergabe nötig, bei Binärverteilung keine Pflicht. Trotzdem im Hinweis führen.
- **BSD-2/3-Clause, ISC, Zlib, 0BSD, CC0, Unlicense**: BSD/ISC verlangen Copyright-Hinweis und Lizenztext auch bei Binärverteilung; Zlib verlangt nichts bei Binärverteilung. Bei `OR`-Ausdrücken ist die Wahl der MIT/Apache-Alternative zulässig.
- **MIT / Apache-2.0**: Copyright-Hinweis und Lizenztext im Release-Paket (z. B. „Über NexPanel“ oder `THIRD-PARTY-NOTICES`).
- **MPL**: im Windows-Normalpfad nicht vorhanden; keine dateibezogenen Copyleft-Pflichten. Bei Änderung der Features/Plattformen erneut prüfen.
- **Schriften**: Es sind Font-Parser-Crates (`fontdb`, `ttf-parser`, `fontique`, …) vorhanden, aber keine eingebetteten Schriftdateien über Crates. Slint-eigene Schriften und die Windows-Systemschriften sind vor Release gesondert zu klären.
- Zwei Versionen von `winreg` im Lockfile (0.55.0 direkt, 0.56.0 transitiv), beide MIT.

Blocker für Closed-Source-Distribution: keiner aus den Abhängigkeiten, sofern der Slint-Royalty-free-Pfad (Bedingungen oben) erfüllt wird. Offen bleibt die Lizenz von NexPanel selbst und die Drittlizenz-Datei im Release-Paket.

### Neu erzeugen

```
cargo metadata --format-version 1 --locked > meta.json
```

Auswertung: `packages[].license` / `license_file` je Crate, gefiltert über `resolve.nodes[].deps[].dep_kinds` (`kind`, `target`) auf `x86_64-pc-windows-msvc`. Alternativ Tooling (nicht Teil des Repos, nicht installiert):

- `cargo install cargo-deny`, dann `cargo deny --target x86_64-pc-windows-msvc check licenses` mit einer `deny.toml` (Allowlist der oben aufgeführten SPDX-Ausdrücke).
- `cargo install cargo-about`, dann `cargo about generate` mit `about.toml` für eine vollständige Drittlizenz-Datei inklusive Lizenztexte.
