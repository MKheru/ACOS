# Architecture ACOS (Agent-Centric OS)

*Statut : Version 2.1 - Affinée suite à l'audit matériel et Deep Dive Exokernel.*

## 1. Vision Globale
ACOS est une réinvention du système d'exploitation basée sur un postulat : **L'IA est le travailleur principal, l'humain est le superviseur.** L'architecture maximise les performances de l'IA (accès bas niveau) tout en garantissant la **souveraineté humaine** via des interruptions matérielles prioritaires.

## 2. L'Architecture : Exokernel Rust & WASM Ecosystem

### 2.1. Audit Critique et Pivot Matériel (Le Mur des Pilotes)
L'audit d'expertise (Round 8) a révélé que confier l'accès GPU "nu" aux agents IA sur un PC générique (avec des pilotes NVIDIA/AMD propriétaires) est un cauchemar d'ingénierie.
**Ajustement Stratégique :** Pour être développable, le prototype ACOS ne visera pas le PC générique fragmenté. Il ciblera un matériel unifié (Soc), comme les puces ARM avec NPU intégré, ou nécessitera un GPU open-source spécifique.

### 2.2. Plongée Technique : Que fait concrètement l'Exokernel Rust ?
Contrairement à Linux qui possède 30 millions de lignes de code, l'Exokernel ACOS ne dépassera pas les 50 000 lignes. Ses rôles exclusifs sont :

1.  **Boot & Initialisation (HAL) :** Démarrer le CPU, passer en mode 64 bits, configurer la table des interruptions (IDT/GDT).
2.  **Gestionnaire de RAM Brut :** Il ne gère pas de "mémoire virtuelle" complexe. Il tient juste un registre des pages physiques libres et les distribue (Page Allocator).
3.  **Multiplexeur PCIe & IOMMU :** C'est son rôle crucial. Il assigne une adresse matérielle (ex: le NPU ou la carte réseau) à un Agent spécifique. Grâce à l'IOMMU, il garantit matériellement que l'Agent A ne peut pas lire la RAM matérielle de l'Agent B.
4.  **Le Gardien du NMI (Non-Maskable Interrupt) :** Il gère la boucle d'interruption du clavier. Dès qu'une touche est pressée, l'Exokernel suspend l'horloge des processeurs assignés à l'IA.

**Ce qu'il NE FAIT PAS :** Pas de TCP/IP, pas de système de fichiers (Ext4/FAT), pas de fenêtres, pas de pilotes de clavier USB complexes (il utilise des protocoles polling très basiques).

### 2.3. Le "Cognitive Engine" (Library OS)
Les modèles d'IA intègrent directement (statiquement ou dynamiquement) les bibliothèques réseau (ex: une stack TCP/IP en Rust en espace utilisateur comme *smoltcp*) et les pilotes d'inférence (ex: llama.rs compilé pour attaquer directement l'adresse PCIe du GPU que l'exokernel lui a donné).

### 2.4. Le Sous-système Humain : WASM / WASI
L'environnement de l'utilisateur (Shell, IDE) tourne dans un runtime WebAssembly.
*Note de développement :* L'interface graphique WASI étant encore balbutiante, le prototype ACOS commencera par un "Agent Shell" purement textuel en WASM, avant d'intégrer un compositeur graphique complet.

## 3. Estimation de Développement (Phase Prototype)

Le développement d'un OS (même un exokernel) est la tâche d'ingénierie la plus complexe en informatique.

*   **Équipe cible :** 1 Ingénieur Système Senior (spécialiste Rust bare-metal) + 1 Spécialiste IA.
*   **Temps estimé pour un Prototype de Faisabilité (MVP) :** **6 à 9 mois**.

**Phases du MVP :**
1.  **Mois 1-2 (Boot & Memory) :** Bootloader UEFI, allocation de pages physiques en Rust, gestion des interruptions de base sur QEMU (émulateur).
2.  **Mois 3-4 (Isolation IOMMU & PCIe) :** Écriture du code d'énumération du bus PCIe. Configuration de l'IOMMU pour isoler un périphérique PCIe virtuel.
3.  **Mois 5-6 (Library OS & Inférence) :** Compilation d'un micro-modèle IA (type Llama 2 15M) capable de tourner sans Linux, juste avec les accès donnés par l'exokernel.
4.  **Mois 7-8 (Intégration WASM) :** Portage du runtime Wasmtime dans un Library OS pour faire tourner le shell de l'utilisateur. Validation de l'interruption matérielle d'urgence (NMI).
