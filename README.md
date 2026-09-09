# Magy v2: The Minimalist Autonomous Engine

Magy is a high-integrity, synchronized AI engineering workbench. It provides a direct, API-less bridge between your local project and ChatGPT, allowing you to collaborate on code across multiple devices (PC and Phone) with a single authoritative state.

Magy operates on a single, uncompromising principle:
> **One Brain, Many Windows.**

## The "One Magy" Architecture

Magy is built on a **Single Authority** model. Unlike traditional sync apps that try to manage multiple independent states, Magy uses a single Rust process as the source of truth.

*   **The Engine (Rust Backend):** Owns the state machine, manages the chat history, and handles the low-level connection to ChatGPT.
*   **The Renderers (PC & Phone):** Connected UIs act as pure puppets. If you switch a tab on your phone, the PC follows. If the agent pops on the PC, the phone pops too.

## Core Features

### 🚀 Zero-API Bridge
Magy uses a specialized **Local Relay** to communicate with ChatGPT through a debugged Chrome instance. This bypasses the need for costly API keys while maintaining a persistent, high-reasoning connection.

### 📏 Automatic Message Splitting
To ensure stability and reliability, the Relay automatically chunks long instructions into manageable **750-character parts**. It uses UI-state detection to handshake with ChatGPT piece-by-piece, ensuring your entire engineering mission is delivered without truncation.

### 📶 Unified Wireless Link
Magy is built for the "Multi-Screen" developer.
*   **PC:** Run the native desktop window for a focused workspace.
*   **Phone:** Connect wirelessly via your local IP (e.g., `http://192.168.x.x:3000`) to manage your project from anywhere on your Wi-Fi.

### 🧼 Human-Centric UI
Stripped of bloat, the Magy interface focuses on the **Interaction Loop**. It features a synchronized chat feed, custom dark-mode scrollbars, and real-time "Brain Health" checkmarks to verify your links are hot.

## The Magy Manifesto

Every Magy instance is initialized with a core directive:
1.  **Truth:** Never claim an action succeeded unless verified.
2.  **Unity:** PC and Phone are two windows into one process.
3.  **Safety:** Operates strictly within the project boundary.

## Quick Start

1.  **Fire the Brain:** Launch Chrome with the debug port open (Port 9222).
2.  **Launch the Engine:** Run `magy-app.exe`.
3.  **Link the Phone:** Open your PC's local IP on port 3000 in your phone's browser.
4.  **Work:** Type your mission and listen for the **Magy POP**.

---
*Created by Zack for the community. API-less local engineering is here.*
