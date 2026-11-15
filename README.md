# MesDrivers

Détecte le matériel Windows, liste les pilotes installés et les périphériques en erreur, puis propose et installe les pilotes plus récents disponibles sur **Windows Update** (sources officielles, signées Microsoft). Interface TUI dans le terminal.

## Installation (PowerShell)

```powershell
irm https://github.com/PhilDaiguille/mesdrivers/releases/latest/download/mesdrivers.exe -OutFile $env:USERPROFILE\mesdrivers.exe
& $env:USERPROFILE\mesdrivers.exe
```

Windows demande les droits administrateur au lancement (nécessaires pour installer un pilote). Pensez à créer un point de restauration avant d'installer.

## Touches

`Tab` onglet · `↑↓` naviguer · `Espace` cocher · `a` tout cocher · `i` installer · `r` rescanner · `q` quitter

## Publier une version

```sh
git tag v0.1.0 && git push --tags   # GitHub Actions compile et attache mesdrivers.exe à la release
```
