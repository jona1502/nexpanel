# NexPanel Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Reproduzierbar baubare Grundgerüste für die Windows-App und die Astro-Website mit dokumentierter Installation erstellen.

**Architecture:** Ein Repository enthält einen Cargo-Workspace mit einer Desktop-Anwendung und eine eigenständige npm-Anwendung. Gemeinsame Dokumentation beschreibt Installation und Prüfungen; CI führt dieselben Prüfungen aus.

**Tech Stack:** Rust 1.96.1/MSVC, Slint, Astro, TypeScript strict, Node 24 LTS, npm.

**Spec:** `docs/superpowers/specs/2026-10-31-project-bootstrap-design.md`

## Global Constraints

- Windows 11 x64 für die Desktop-Anwendung.
- Rust wird auf die vorhandene Version 1.96.1 mit MSVC-Ziel festgelegt.
- Slint und slint-build erhalten dieselbe verifizierte Release-Version; Cargo.lock wird versioniert.
- Die vorhandene globale Node-25-Installation wird nicht entfernt oder ersetzt.
- Ein minimales Astro-Projekt verwendet npm, TypeScript strict und statische Ausgabe.
- Keine Pushes oder Veröffentlichungen.
- Bestehende fremde Änderungen bleiben erhalten.

## Review Focus

- Neue Shell: dokumentierte Aktivierung stellt dieselbe Node-Laufzeit bereit (Task 1 und 3).
- MSVC-Erkennung: ein vollständiger Cargo-Build muss erfolgreich linken (Task 2).
- Slint-Laufzeit: Fensterstart zusätzlich zur Kompilierung prüfen oder Einschränkung dokumentieren (Task 2).
- Lockfiles: saubere npm-ci-Installation und Cargo --locked müssen funktionieren (Task 2 und 3).
- Dokumentation und CI: Befehle und Laufzeiten müssen den geprüften lokalen Stand abbilden (Task 4).

---

### Task 1: Repository und Werkzeuge

**Files:** `.gitignore`, `.editorconfig`, `.gitattributes`, `rust-toolchain.toml`, `.node-version`, `scripts/use-node.ps1`, `docs/setup.md`, `README.md`.

**Interfaces:** Stellt Rust 1.96.1 mit rustfmt/clippy und Node 24 für spätere Tasks bereit. `scripts/use-node.ps1` wird dot-sourced und stellt Node nur für die aktuelle Shell bereit.

- [x] Bestehende Werkzeuge erneut prüfen; Node-Version und Download anhand offizieller Node- und Astro-Quellen verifizieren.
- [x] Gemeinsame Textformatierung und Ignore-Regeln für Cargo-Ausgabe, npm, Astro, lokale Werkzeuge und SDD-Artefakte einrichten.
- [x] Rust-Version mit rustfmt/clippy festlegen und fehlende Komponenten installieren.
- [x] Node 24 benutzerlokal oder unter ignoriertem `.tools` ablegen; offizielle SHA256-Prüfsumme vor Verwendung verifizieren. Kein globaler PATH-Umbau.
- [x] Node-Aktivierung und Setup dokumentieren; in neuer PowerShell Version und npm-Aufruf prüfen. Windows-npm-Aufrufe verwenden `npm.cmd`.
- [x] `git diff --check` und Werkzeugversionen prüfen. Commit: `chore: configure repository and development tooling`.

### Task 2: Desktop-Grundgerüst

**Files:** `Cargo.toml`, `Cargo.lock`, `apps/desktop/Cargo.toml`, `apps/desktop/build.rs`, `apps/desktop/src/main.rs`, `apps/desktop/ui/main.slint`.

**Interfaces:** Workspace-Mitglied `apps/desktop`, Binary-Paket `nexpanel`; `cargo run -p nexpanel` öffnet das Setup-Fenster.

- [x] Aktuelle kompatible Slint-Version anhand offizieller Dokumentation und Registry prüfen; identische Version für slint und slint-build verwenden.
- [x] Root-Workspace und ein Binary-Paket erstellen. UI über build.rs kompilieren; generierten Code in main.rs einbinden.
- [x] Ein kleines Fenster mit NexPanel-Titel, Setup-Beschriftung und spezifizierten Farben erstellen. Schließen beendet die App.
- [x] `cargo fmt --all -- --check`, `cargo check --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings` und `cargo build --workspace --locked` erfolgreich ausführen.
- [x] Fensterstart prüfen; Prozess danach gezielt beenden. Visuelle Einschränkungen und Build-Ergebnis dokumentieren.
- [x] Commit: `chore(desktop): scaffold Rust and Slint application`.

### Task 3: Website-Grundgerüst

**Files:** `apps/website/package.json`, `package-lock.json`, `astro.config.mjs`, `tsconfig.json`, `src/pages/index.astro`.

**Interfaces:** `npm.cmd run dev`, `npm.cmd run check` und `npm.cmd run build` im Website-Verzeichnis, mit Node aus Task 1.

- [x] Minimales Astro-Projekt ohne zusätzliches Framework erstellen. `@astrojs/check` und TypeScript als devDependencies ergänzen.
- [x] TypeScript strict sowie statische Ausgabe und Skripte für dev, check, build und preview konfigurieren.
- [x] Neutrale NexPanel-Setup-Seite mit deutschem Text, `lang="de"`, HTML-Titel und Viewport erstellen.
- [x] `npm.cmd ci`, `npm.cmd run check` und `npm.cmd run build` erfolgreich ausführen.
- [x] Lokalen Server versteckt starten, Seite per HTTP auf Titel und Sprache prüfen und Server anschließend gezielt stoppen.
- [x] Commit: `chore(website): scaffold Astro website`.

### Task 4: CI und Abschlussdokumentation

**Files:** `.github/workflows/ci.yml`, `README.md`, `docs/setup.md`, dieser Plan.

**Interfaces:** CI verwendet die bereits geprüften lokalen Befehle; Windows-Job baut Desktop, Website-Job prüft und baut Astro.

- [x] Windows-CI für Rust mit Version 1.96.1, rustfmt/clippy und --locked einrichten.
- [x] Website-CI mit Node 24, npm ci, check und build einrichten.
- [x] README und Setup mit exakten verwendeten Versionen, Voraussetzungen, Startbefehlen und gemessenen Prüfergebnissen vervollständigen. Keine ungeprüften Performance-Zusagen.
- [x] Workflow auf korrekte Arbeitsverzeichnisse, Lockfile-Pfade und Übereinstimmung mit lokalen Befehlen prüfen. Remote-CI bleibt ohne Push ungeprüft.
- [x] `git diff --check` ausführen; Task-Häkchen anhand tatsächlich erbrachter Nachweise aktualisieren.
- [x] Commit: `ci: add desktop and website build checks`.

## Ausführung

Der Nutzer hat Subagents und den Start der Umsetzung ausdrücklich angefordert. Die Koordination erstellt Commits nach Prüfung; implementierende Subagents bearbeiten jeweils nur ihren Task. Ein Arbeitsbranch im vorhandenen Checkout hält die Änderungen im sichtbaren IDE-Projekt. Ein zusätzlicher Worktree ist für dieses neue Repository nicht erforderlich; es wird nicht auf main implementiert.

Setup-Schritte werden mit den tatsächlichen Build- und Startprüfungen abgenommen. Zusätzliche Tests ohne Fachverhalten werden nicht erfunden. Jede Task-Änderung wird unabhängig geprüft, abschließend erfolgt ein Gesamtreview.
