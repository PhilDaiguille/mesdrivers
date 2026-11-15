# FAQ

**Pourquoi des pilotes Intel datés de 1968 ?**
C'est voulu par Intel. Les INF « chipset » (10.1.x : Host Bridge, PCI Express Root Port, SMBus…) portent `DriverVer=07/18/1968` pour ne jamais passer devant un vrai pilote dans le classement de Windows. MesDrivers les affiche « non datée ». Pour eux, seule la version compte.

**Pourquoi « · 6 périphériques » sur une mise à jour ?**
Plusieurs composants (par exemple 6 contrôleurs Serial IO I2C) utilisent le même pilote. Une seule installation les met tous à jour.

**Pourquoi aucune mise à jour Windows Update ?**
Windows a déjà installé ce qu'il propose. Le Catalogue Microsoft a souvent des versions plus récentes que Windows Update ne pousse pas automatiquement : elles apparaissent avec la source « Catalogue ».

**Le Catalogue, c'est sûr ?**
C'est le site officiel de Microsoft (catalog.update.microsoft.com). Les fabricants y publient des pilotes signés par Microsoft (WHQL). MesDrivers vérifie l'empreinte SHA-1 du fichier, l'architecture du pilote, et Windows refuse un pilote non signé. Un point de restauration est créé avant l'installation.

**« Point de restauration impossible » ?**
La protection du système est désactivée sur le disque C:. Pour l'activer : Panneau de configuration → Récupération → Configurer la Restauration du système → C: → Configurer → Activer. Sans elle, MesDrivers demande confirmation avant de continuer.

**Que supprime l'onglet DriverStore ?**
Seulement les anciennes versions d'un pilote dont une version plus récente est en service. Un pilote seul (imprimante débranchée par exemple) est conservé. Windows refuse de toute façon de supprimer un pilote utilisé.

**Comment revenir en arrière après une installation ?**
Par le Gestionnaire de périphériques → le périphérique → Pilote → Version précédente. Sinon, restaurez le point de restauration créé par MesDrivers (« MesDrivers »).

**Où est le journal ?**
`%LOCALAPPDATA%\mesdrivers\journal.txt` : une ligne par installation, suppression, export ou restauration.

**Windows SmartScreen bloque l'exe ?**
L'exe n'est pas signé (un certificat de signature est payant). Téléchargé avec `irm`, il n'est normalement pas bloqué. Sinon : « Informations complémentaires » → « Exécuter quand même ». L'empreinte SHA-256 de chaque release est publiée à côté de l'exe.

**Ça marche sur quelles versions de Windows ?**
Windows 10 et 11, x64 (ARM64 non publié pour l'instant). Il faut un compte administrateur.
