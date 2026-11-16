# NexPanel 0.1 – Windows 11 x64

Dieses lokal gebaute Paket enthält nexpanel.exe und PowerShell-Skripte. Es ist nicht codesigniert. Eine Projektlizenz wurde noch nicht gewählt; vor Weitergabe die Projekt- und Abhängigkeitslizenzen klären.

## Installation

ZIP vollständig in einen eigenen Ordner entpacken. PowerShell dort öffnen:

```powershell
./install.ps1
```

Standardziel: %LOCALAPPDATA%\NexPanel\bin. Keine Administratorrechte nötig. Startmenü-Verknüpfung wird angelegt. Die Anwendung startet nicht automatisch; Autostart bleibt unverändert. Falls PowerShell die Skriptausführung beschränkt, zuerst Skript und Herkunft prüfen und die lokale Richtlinie beachten.

Eigenes Ziel ohne Startmenü-Verknüpfung:

```powershell
./install.ps1 -InstallDirectory 'C:\MeinOrdner\NexPanel' -NoShortcut
```

NexPanel danach per Startmenü oder nexpanel.exe öffnen. Tray-Klick schaltet das Panel um; Esc und Schließen blenden aus. Beenden erfolgt im Tray-Menü. --background startet versteckt; --help zeigt Diagnose- und Aufnahmeoptionen.

## Deinstallation

NexPanel im Tray beenden. Das installierte uninstall.ps1 im Installationsordner ausführen (bei Installation mit -NoShortcut ebenfalls -NoShortcut übergeben). Entfernt nur bekannte Paketdateien, die eigene passende Verknüpfung und einen exakt auf diese Installation zeigenden NexPanel-Autostarteintrag. Keine Prozesse werden erzwungen beendet. %LOCALAPPDATA%\NexPanel\settings.json und fremde Dateien bleiben erhalten.

## Grenzen

Helligkeit benötigt DDC/CI oder interne WMI-Unterstützung; nicht jedes Display oder jeder Adapter unterstützt das. Keine Software-Abdunkelung. Timer laufen im Tray weiter, werden bei App-Neustart zurückgesetzt und folgen der Systemzeit. Native Interaktion, Hardware-Schreibzugriffe, Schlaf/Hotplug und andere Rechner benötigen weitere praktische Tests; Performance ist ungemessen.

Quellcode, Voraussetzungen und Teststand: https://github.com/jona1502/nexpanel
