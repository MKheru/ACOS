# Architecture ACOS (Agent-Centric OS)

*Statut : Version 3.0 - Le "Cheat Code" Linux/Rust & Le Bus MCP.*

## 1. Vision Globale
ACOS est le premier système d'exploitation où le **Model Context Protocol (MCP)** n'est pas un outil utilisateur, mais le **bus système natif (IPC)**. L'IA est le citoyen de première classe. L'humain interagit via un sous-système web/WASM, qui est entièrement pilotable par l'IA.

## 2. Le Noyau : Le pragmatisme "Rust-for-Linux" (RFL)

Coder un Exokernel de zéro pour un PC moderne est une impasse (le "mur des pilotes"). ACOS utilise une approche hybride radicale :

### 2.1. La Fondation Linux "Éviscérée"
ACOS boot sur un noyau Linux standard, mais extrêmement minimaliste (custom kernel build). Son seul but est d'apporter le support matériel gratuit (pilotes GPU propriétaires, USB, PCIe, gestion d'énergie). L'espace utilisateur GNU/Linux classique (systemd, bash, X11) est totalement supprimé.

### 2.2. L'Hyper-Module Rust ACOS
Toute l'intelligence "Agent-Centric" est implémentée sous forme de **modules noyau écrits en Rust** (via l'infrastructure *Rust-for-Linux*). 
- Ce module Rust prend le contrôle de l'ordonnanceur Linux (via eBPF ou des hooks profonds).
- Il garantit l'interruption de sécurité humaine (NMI) : il surveille les interruptions des périphériques d'entrée pour suspendre les calculs GPU de l'Agent.

## 3. Le Bus Système : MCP (Model Context Protocol) au niveau OS

Sur Linux, les processus se parlent via des pipes, des sockets ou D-Bus. Sur ACOS, **les processus se parlent en MCP**.

### 3.1. Tout est un Serveur MCP
- Le système de fichiers n'est pas monté classiquement, il est exposé par le noyau comme un "Serveur MCP Fichier".
- Le GPU est exposé comme un "Serveur MCP Calcul".
- Le Navigateur Web/IDE (tournant dans l'espace humain) expose son état interne (DOM, onglets ouverts) via un "Serveur MCP UI".

### 3.2. Le Superviseur IA (Le Cerveau)
L'Agent IA principal tourne comme le processus init (PID 1) de l'ACOS.
Puisque le Navigateur expose ses contrôles via MCP, l'Agent IA peut nativement et instantanément :
- "Lis l'onglet 2 du navigateur."
- "Clique sur ce bouton dans le DOM."
- "Ouvre VS Code et tape ce code."
L'Agent contrôle l'interface humaine comme une simple API, sans avoir besoin de simuler des clics de souris hasardeux.

## 4. L'Interface Humaine : Le Bac à Sable WebApps/WASM

L'humain n'a pas accès au système de fichiers sous-jacent. Son environnement visuel est un "Thin Client" (Client Léger) :

- **Le Compositeur :** Un compositeur Wayland ultra-basique écrit en Rust, géré par le module noyau ACOS.
- **L'Exécuteur :** Une WebView/WASM engine (type Tauri/Servo) en plein écran.
- **L'Expérience :** L'utilisateur utilise des WebApps (VS Code Web, Navigateurs). Lorsqu'il a besoin d'une action, il demande à l'Agent IA. L'Agent utilise le bus MCP pour "piloter" la WebApp devant les yeux de l'utilisateur, ou travaille en tâche de fond.

## 5. Nouveau Chronogramme de Développement (Réaliste)
Grâce à l'utilisation de la base Linux et de Rust-for-Linux :
- **Mois 1 :** Création du noyau Linux minimal et implémentation du "Bus MCP" en espace noyau via Rust.
- **Mois 2 :** Intégration du superviseur IA (PID 1) et de la communication avec le bus MCP noyau.
- **Mois 3 :** Lancement du compositeur Wayland minimal et de la WebView pour afficher l'interface humaine. Validation du contrôle de la WebView par l'Agent via MCP.
