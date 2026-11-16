# NexPanel: Entwicklungsgrundlage

Datum: 2026-10-31
Status: Vom Nutzer am 2026-10-31 zur Umsetzung freigegeben
Grundlage: `NexPanel.md`

## Ziel und Umfang

Eine reproduzierbare Entwicklungsgrundlage für die Windows-Anwendung und die Astro-Website schaffen. Beide Projekte sollen sich lokal bauen und starten lassen. Installation, Startbefehle, Prüfungen und Entscheidungen werden dokumentiert. Die Umsetzung erfolgt mit Subagents und getrennten, überprüften Commits.

Dieser Schritt umfasst Grundgerüste, benötigte Werkzeuge, Lockfiles und Build-Prüfungen. Tray-Verhalten, Monitorsteuerung, Audio, Timer, Installer und die gestaltete öffentliche Website folgen später. Es werden keine Platzhalter-Services, Plugin-Systeme oder ungenutzten Frameworks angelegt. Veröffentlichung und Deployment gehören nicht zum Setup.

## Betrachtete Ansätze und Entscheidung

1. Gemeinsames Repository mit `apps/desktop`, `apps/website` und gemeinsamer Dokumentation: einfachere Abstimmung und gemeinsame Änderungshistorie; beide Projekte teilen die Repository-Verwaltung.
2. Getrennte Repositories: unabhängigere Verwaltung und Veröffentlichungen, dafür zusätzliche Abstimmung und doppelte Grundkonfiguration.

Der Nutzer hat Ansatz 1 gewählt. Rust und Astro behalten getrennte Paketverwaltung und Lockfiles. Zusätzliche Monorepo-Werkzeuge sind nicht erforderlich.

## Struktur

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
.gitignore
.editorconfig
.gitattributes
README.md
NexPanel.md
apps/
  desktop/
    Cargo.toml
    build.rs
    src/main.rs
    ui/main.slint
  website/
    package.json
    package-lock.json
    astro.config.mjs
    tsconfig.json
    public/
    src/pages/index.astro
docs/
  setup.md
  superpowers/specs/
  superpowers/plans/
.github/workflows/
```

Leere Verzeichnisse werden nicht künstlich versioniert. Weitere Module entstehen mit ihrer ersten Funktion.

## Desktop

Ein Cargo-Workspace enthält zunächst nur das Binary-Paket `nexpanel` in `apps/desktop`. Slint kompiliert `ui/main.slint` über `build.rs`. Das Grundgerüst zeigt ein kleines NexPanel-Fenster mit den bereits spezifizierten Farben und einer erkennbaren Setup-Beschriftung. Normales Schließen beendet den Prozess; das spätere Tray-Popup wird damit nicht vorweggenommen.

Rust wird auf die vorhandene Version 1.96.1 mit MSVC-Ziel festgelegt. Fehlende Komponenten `rustfmt` und `clippy` werden ergänzt. Slint und slint-build erhalten dieselbe verifizierte Release-Version; Cargo.lock wird versioniert. Vorhandene MSVC-Werkzeuge und Windows SDK werden durch einen tatsächlichen Build geprüft. Kein Qt wird zusätzlich installiert.

## Website

Ein minimales Astro-Projekt verwendet npm, TypeScript strict und statische Ausgabe. Die Startseite enthält nur eine neutrale NexPanel-Setup-Seite mit HTML-Titel, Sprache und Viewport. Zusätzliche UI-Frameworks und Integrationen werden erst bei konkretem Bedarf eingeführt.

Node 24 LTS wird bevorzugt projektbezogen oder benutzerlokal bereitgestellt. Die vorhandene globale Node-25-Installation wird nicht entfernt oder ersetzt. Die genaue verfügbare Version und ihre Astro-Kompatibilität werden vor Installation anhand offizieller Quellen geprüft. README und Setup-Dokumentation erklären, wie dieselbe Laufzeit in einer neuen Shell aktiviert wird. `package-lock.json` wird versioniert; `@astrojs/check` und TypeScript erlauben eine eigenständige Typprüfung.

## Dokumentation und Commits

Jeder Commit enthält eine zusammenhängende Änderung und wird erst nach seinen relevanten Prüfungen erstellt. Keine Pushes oder Veröffentlichungen. Die bestehende, noch unversionierte `NexPanel.md` wird unverändert in den Dokumentationscommit aufgenommen.

1. `docs: document project architecture and bootstrap plan`: Spezifikation, freigegebener Entwurf und Umsetzungsplan.
2. `chore: configure repository and development tooling`: gemeinsame Einstellungen, Ignore-Regeln, Laufzeitkonfiguration und Setup-Anleitung.
3. `chore(desktop): scaffold Rust and Slint application`: startfähiges Desktop-Grundgerüst und Cargo.lock.
4. `chore(website): scaffold Astro website`: prüfbares Website-Grundgerüst und npm-Lockfile.
5. `ci: add desktop and website build checks`: Windows-Prüfung für Rust, separate Website-Prüfung und abschließende Dokumentation.

Implementierende Subagents bearbeiten getrennte Bereiche; die Koordination prüft Änderungen und erstellt die Commits. Bestehende fremde Änderungen bleiben erhalten. Ein frischer Reviewer prüft das Gesamtergebnis.

## Abnahme

- `cargo fmt --all -- --check`, `cargo check --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings` und `cargo build --workspace --locked` sind erfolgreich.
- Das Desktop-Fenster startet lokal und lässt sich schließen. Falls eine visuelle Prüfung nicht möglich ist, wird diese Einschränkung ausdrücklich dokumentiert.
- Im Website-Verzeichnis funktionieren `npm ci`, `npm run check` und `npm run build` mit der dokumentierten Node-Laufzeit.
- Die Startseite wird lokal per HTTP geprüft. CI-Konfiguration verwendet dieselben Versionen und Prüfungen.
- README und `docs/setup.md` enthalten Voraussetzungen, Installation, Start, Prüfung und den tatsächlich erreichten Stand. Performance- und Hardwareziele gelten noch nicht als nachgewiesen.

## Entscheidungen der Umsetzung

Die Koordination wählt kompatible konkrete Paketversionen, die benutzerlokale Node-Bereitstellung und minimale CI-Konfiguration. Die verwendeten Versionen und Entscheidungen werden dokumentiert. Projektlizenz, Slint-Veröffentlichungsbedingungen, Hosting und Installer werden vor einer späteren Veröffentlichung geklärt; dieses Setup legt keine Lizenz fest.
