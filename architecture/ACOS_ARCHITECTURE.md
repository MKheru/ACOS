# Architecture ACOS (Agent-Centric OS)

*Statut : Version 4.0 - La Fondation Redox OS & Le Protocole `mcp:`*

## 1. Vision Globale
ACOS est un système d'exploitation "AI-First" pur, éliminant la dette technique des noyaux monolithiques des années 90 (Linux/Windows). Il utilise un micro-noyau entièrement écrit en Rust, où la communication entre l'Intelligence Artificielle, le matériel et l'interface humaine se fait via un réseau sémantique (MCP) plutôt que par des fichiers POSIX traditionnels.

## 2. Le Noyau : Fork de Redox OS

Plutôt que d'éviscérer Linux (ce qui reste un compromis lourd) ou de créer un noyau de zéro (trop long), ACOS est basé sur un fork du projet **Redox OS**.

### 2.1. Les Avantages du Micro-noyau Redox
- **Sécurité et Isolation :** Seulement ~30 000 lignes de code s'exécutent avec des privilèges élevés. Les pilotes (drivers), le réseau et le système de fichiers tournent dans l'espace utilisateur. Si un composant plante, il redémarre sans faire crasher l'OS.
- **Mémoire Sûre :** Construit à 100% en Rust, éliminant par conception les failles de corruption mémoire.
- **La solution aux pilotes (Driver Sandbox) :** Pour le matériel très complexe (ex: GPU NVIDIA), ACOS utilise l'approche Redox d'isoler un mini-noyau Linux dans une machine virtuelle légère stricte (driver-domain) uniquement chargée d'exposer l'interface PCIe, sans polluer le reste du système.

## 3. L'IPC Sémantique : Le mariage des "Schemes" et du MCP

L'innovation centrale de Redox est son système IPC basé sur des URI (Uniform Resource Identifiers), appelés "Schemes". Par exemple, on accède au réseau via `tcp:` et à l'écran via `display:`.

### 3.1. Le schéma `mcp:` natif
ACOS étend ce paradigme en créant le schéma système `mcp:`.
- **Avant (Linux) :** L'agent IA devait lancer un processus bash, exécuter `cat /etc/config`, lire stdout.
- **Avec ACOS/Redox :** L'agent IA ouvre simplement le flux `mcp://system/config` au niveau du noyau. 

### 3.2. Tout le système est un graphe de connaissances
- L'interface utilisateur Web (WASM/Servo) s'enregistre auprès du noyau en tant que `mcp://ui/browser/tab/1`.
- Le gestionnaire de processus s'enregistre comme `mcp://system/processes`.
- Le Superviseur IA (qui tourne comme un service système principal) peut interroger, surveiller et modifier n'importe quel composant de l'ordinateur en utilisant le standard Model Context Protocol, car le noyau gère le routage MCP de manière matérielle.

## 4. Les Couches Supérieures (L'Espace Utilisateur)

L'architecture des processus est divisée en deux mondes, gérés par l'ordonnanceur de Redox :

### 4.1. Le Domaine Cognitif (The AI Supervisor)
- **Rôle :** C'est le cerveau de la machine. Un ou plusieurs modèles d'IA (ex: Llama 3 local, ou une connexion API distante) tournent en tâche de fond constante.
- **Capacités :** Il écoute les requêtes sur le bus `mcp:`. Il peut lire les logs du système, modifier des fichiers, générer du code. Il possède la capacité d'interagir avec les processus matériels (GPU) via des permissions strictes.

### 4.2. Le Domaine Humain (Le client léger WASM/Servo)
- **Rôle :** Fournir l'interface visuelle à l'utilisateur (Navigateur, IDE, Terminal).
- **Technologie :** ACOS intègre nativement **Servo** (le moteur de rendu web écrit en Rust). L'utilisateur n'exécute pas de binaires natifs classiques. Toutes les applications (comme un clone de VS Code) sont des WebApps s'exécutant dans le bac à sable de Servo.
- **L'Intégration IA :** Ce moteur Servo expose son arbre DOM via le schéma `mcp:`. L'utilisateur peut dire à voix haute : *"Ferme tous les onglets qui parlent de Python"*, le Superviseur IA comprend la requête, envoie une commande via `mcp://ui/servo/tabs`, et le navigateur s'exécute.

## 5. Résumé de l'Expérience Utilisateur
Sur un PC équipé d'ACOS, l'humain n'a plus besoin d'organiser ses fichiers dans des dossiers ou de scripter des tâches. L'ordinateur est un agent. L'humain utilise le navigateur (Servo) pour consommer du contenu, et le Superviseur IA (intégré au micro-noyau Redox via le bus `mcp:`) gère la complexité informatique sous-jacente en temps réel.
