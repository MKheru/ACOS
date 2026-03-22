# Architecture ACOS (Agent-Centric OS)

*Statut : Version 2.0 - Validée après 7 itérations (Correction suite à l'audit de sécurité).*

## 1. Vision Globale (Corrigée)
ACOS est une réinvention du système d'exploitation basée sur un postulat : **L'IA est le travailleur principal, l'humain est le superviseur.** L'architecture doit maximiser les performances de l'IA (accès bas niveau, gestion de la mémoire tensorielle) tout en garantissant de manière absolue la **souveraineté et le contrôle de l'humain**, même lorsque le système exécute des applications de productivité complexes (IDE, Navigateur).

## 2. L'Architecture Viable : Exokernel Rust & WASM Ecosystem

L'approche de la virtualisation lourde (VM) ou des micro-noyaux à capacités pures ayant échoué face à la réalité technique, ACOS adopte une architecture en trois couches distinctes :

### 2.1. La Fondation : L'Exokernel Rust "Naked"
L'Exokernel (noyau minimaliste qui multiplexe le matériel de manière brute, sans imposer d'abstractions) est développé en Rust.
- **Zéro Abstraction Système :** L'Exokernel ne connaît ni le concept de "fichier", ni de "socket réseau". Il ne gère que l'allocation des pages mémoire physiques et l'accès sécurisé aux bus de données (PCIe pour le GPU).
- **Le NMI Humain (Non-Maskable Interrupt) :** C'est la garantie de sécurité. Les ports d'entrée (Clavier/Souris) sont câblés (logiciellement ou matériellement) sur une interruption non-masquable. **L'humain a la priorité absolue.** Au moindre mouvement de souris, l'Exokernel préempte instantanément les ressources de calcul de l'IA pour garantir un rendu fluide de l'interface utilisateur.

### 2.2. Le "Cognitive Engine" (Library OS)
Les modèles d'IA locaux et les agents ne tournent pas comme des "programmes", mais utilisent un système d'exploitation sous forme de bibliothèque (Library OS) directement lié à l'Exokernel.
- Ils gèrent eux-mêmes leur propre mémoire virtuelle et leurs accès GPU, éliminant tout appel système (syscall) coûteux lors de l'inférence.
- C'est l'équivalent d'un environnement "Unikernel" dédié uniquement au calcul neuronal et à la gestion vectorielle.

### 2.3. L'Interface Humaine Standardisée : Le Sous-système WASM / WASI
C'est la solution élégante au problème de compatibilité des logiciels humains (VS Code, Navigateurs) sur un OS minimaliste.
- **Abandon des binaires natifs :** ACOS n'essaie pas de faire tourner des binaires ELF (Linux) ou PE (Windows). 
- **L'environnement de l'utilisateur (le Shell, l'IDE, le Navigateur) est entièrement exécuté dans un runtime WebAssembly (WASM) couplé à l'interface WASI.**
- **Avantages :** 
  - *Sécurité maximale :* WASM est un bac à sable parfait par conception. Un navigateur compromis ne peut pas sortir de son runtime pour toucher au "Cognitive Engine" de l'IA.
  - *Légèreté :* Pas besoin de réécrire des millions de lignes de code POSIX (système de fichiers Linux, etc.). Le runtime WASI fournit juste ce qu'il faut.
  - *Compatibilité :* VS Code est déjà massivement compatible avec le web et WASM.

## 3. Optimisation Matérielle PC (Propositions Mises à Jour)
Pour soutenir cette architecture Exokernel + WASM :
- **Clavier/Souris avec puce de gestion d'interruptions d'urgence :** Un périphérique d'entrée capable d'envoyer un signal matériel prioritaire sur le bus, forçant l'ordonnanceur CPU à suspendre les tâches tensorielles massives.
- **Mémoire NVM (Non-Volatile Memory) unifiée :** L'effacement de la frontière entre la RAM et le SSD (via des technologies type CXL ou Optane de nouvelle génération) pour permettre à l'Exokernel de charger des bases de données vectorielles massives sans temps d'I/O de disque.