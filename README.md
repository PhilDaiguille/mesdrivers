# MesDrivers

Outil de gestion des pilotes Windows dans le terminal (TUI). Il remplace « Mes Drivers » de TousLesDrivers, figé depuis 2016 :

- **Résumé** du PC : Windows, CPU, RAM, carte mère, BIOS, cartes graphiques, disques.
- **Matériel** : tous les pilotes installés, les périphériques en erreur en rouge.
- **Mises à jour** depuis deux sources officielles et signées :
  - **Windows Update** ;
  - **Catalogue Microsoft Update**, souvent plus récent : .cab vérifié par SHA-1, contrôle de l'architecture, installation via `pnputil`.
- **DriverStore** : repère les anciennes versions de pilotes qui prennent de la place (souvent plusieurs Go) et les supprime sans risque.
- **Sauvegarde / restauration** de tous les pilotes, idéal avant de réinstaller Windows.
- **Mise à jour automatique** de MesDrivers depuis GitHub (exe vérifié par SHA-256).

## Installation (PowerShell)

```powershell
irm https://github.com/PhilDaiguille/mesdrivers/releases/latest/download/mesdrivers.exe -OutFile $env:USERPROFILE\mesdrivers.exe
& $env:USERPROFILE\mesdrivers.exe
```

Windows demande les droits administrateur au lancement (nécessaires pour installer ou supprimer un pilote).

## Sécurité

- Point de restauration Windows créé avant chaque installation ou restauration. S'il échoue, l'appli demande confirmation.
- Pilotes installés un par un, avec la progression affichée. Windows refuse de lui-même un pilote non signé.
- DriverStore : seuls les paquets **inutilisés et remplacés par une version plus récente en service** sont proposés. La suppression se fait sans `/force`, donc Windows refuse un pilote encore utilisé.
- Chaque action est notée dans `%LOCALAPPDATA%\mesdrivers\journal.txt`.

## Touches

| Touche | Action |
|---|---|
| `Tab` / `1`–`5` | changer d'onglet |
| `↑↓` `PgUp/PgDn` | naviguer |
| `/` | filtrer (Échap efface) |
| `Espace` / `a` | cocher / tout cocher |
| `i` | installer les mises à jour cochées |
| `d` | supprimer les paquets DriverStore cochés |
| `Entrée` | lancer une sauvegarde / restauration |
| `m` | afficher/masquer les pilotes génériques Microsoft |
| `o` | ouvrir le Catalogue Microsoft pour la ligne sélectionnée |
| `r` | rescanner |
| `U` | mettre à jour MesDrivers |
| `q` | quitter |

## Ligne de commande

```powershell
mesdrivers --scan                                        # analyse complète sans interface, avec les durées
mesdrivers --catalog "PCI\VEN_8086&DEV_51F1"             # dernière version publiée au Catalogue
mesdrivers --catalog-get "PCI\VEN_8086&DEV_51F1" C:\tmp  # télécharge + vérifie + extrait, sans installer
mesdrivers --backup D:\Pilotes                           # exporte tous les pilotes tiers
mesdrivers --restore D:\Pilotes                          # réinstalle une sauvegarde
```

## Publier une version

```sh
git tag v0.3.0 && git push --tags   # la CI teste (dont des tests réels sur Windows), compile et publie l'exe + son .sha256
```
