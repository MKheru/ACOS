# Architecture ACOS (Agent-Centric OS)

*Statut : Validé après 4 itérations d'Auto-Évolution (Phase 1).*

## 1. Vision Globale
ACOS est un système d'exploitation conçu de zéro en Rust, renversant le paradigme traditionnel. L'utilisateur principal du système n'est plus l'humain via une interface graphique, mais un **Superviseur d'Agents IA**. L'interface humaine est reléguée au rang de "périphérique optionnel et isolé".

## 2. L'Architecture Hybride Validée : CBM & Context Buffers
Après plusieurs itérations visant à équilibrer légèreté, sécurité et compatibilité avec des outils comme VS Code, l'architecture suivante a été validée :

### 2.1. Le Micro-noyau à Capacités (CBM - Capability-Based Microkernel)
Le cœur du système est un micro-noyau écrit en Rust, fortement inspiré de seL4.
- **Rôle strictement limité :** Il ne contient AUCUN pilote de périphérique complexe (ni réseau, ni affichage, ni système de fichiers). Il gère uniquement la mémoire (MMU), le temps CPU (ordonnancement) et les communications inter-processus (IPC).
- **Sécurité par Capacités :** Chaque ressource (mémoire, accès matériel) est un jeton cryptographique (une capacité). Sans la capacité explicite, un processus ne peut rien voir ni toucher.

### 2.2. Le "Agent Supervisor Domain" (Ring 1)
C'est l'espace utilisateur privilégié où vivent les modèles d'IA (via CLI ou orchestration locale).
- **Accès Bare-Metal :** Ce domaine possède la capacité exclusive d'accéder directement au matériel de calcul intensif (GPU via PCIe pass-through, NPU) sans passer par une couche d'abstraction graphique (type OpenGL/Vulkan/DirectX).
- **Mémoire Contextuelle :** Utilisation agressive de la RAM pour maintenir des bases vectorielles et le contexte des agents en direct.

### 2.3. Le "Human UI Sandbox" (Ring 3)
C'est ici que tournent votre navigateur Web et VS Code.
- **Isolation Totale :** Ce sous-système est un "bac à sable" hermétique. Il contient un compositeur Wayland ultra-léger et les outils nécessaires.
- **Suspension Intelligente :** L'ordonnanceur de l'ACOS alloue moins de 5% du temps CPU à ce domaine lorsque l'utilisateur n'interagit pas physiquement (souris/clavier inactifs), redirigeant 95% de l'énergie vers les agents IA en arrière-plan.

### 2.4. La Révolution IPC : "Zero-Copy Context Buffers"
Pour résoudre le goulot d'étranglement de la communication entre l'Agent (qui génère du code/texte) et l'UI humaine (qui doit l'afficher), ACOS n'utilise pas de sockets ou de pipes traditionnels.
- L'OS alloue des "Context Buffers" (zones de mémoire partagée).
- L'Agent IA écrit directement ses résultats dans ce buffer.
- Le domaine UI obtient une capacité "Lecture Seule" sur ce mapping mémoire.
- **Résultat :** L'interface graphique affiche les résultats de l'IA à la vitesse de la RAM, sans aucune surcharge de copie CPU.

## 3. Propositions d'Optimisation Matérielle PC
Pour tirer pleinement parti de cette architecture, le PC de demain devrait évoluer :
- **Asymétrie Mémoire (RAM-C) :** Création de barrettes de RAM dédiées au "Contexte" (optimisées pour la bande passante aléatoire des LLM), séparées de la RAM système classique.
- **GPU sans affichage natif :** Les GPU/NPU ne devraient plus gérer de sorties vidéo physiques. La sortie vidéo doit être gérée par un simple chipset sur la carte mère, le GPU devenant un pur co-processeur de calcul asynchrone interconnecté avec les *Context Buffers*.