---
name: macOS test report
about: Tried PromptFlow on a Mac? Tell us what worked (takes ~15 minutes)
title: "macOS test: <Mac model>, macOS <version>"
labels: macos
---

**Mac:** (e.g. MacBook Air M2, 16 GB)
**macOS version:** (Apple menu → About This Mac)
**Python:** (python.org / Homebrew, version)

### Checklist
Tick what worked; for anything that didn't, write what happened below.

- [ ] `bash install.command` finished (how long did it take? ___ min)
- [ ] macOS asked for **Microphone**, **Input Monitoring** and **Accessibility**, and each request said "PromptFlow"
- [ ] The orange banner in the window disappeared after allowing them
- [ ] The microphone icon showed up in the menu bar; its menu has Show / Pause / Quit
- [ ] Holding **Right Command** showed the "Listening" pill at the bottom of the screen
- [ ] The text was pasted into **Notes**
- [ ] ... into **a browser** (Chrome / Safari)
- [ ] ... into **VS Code / Cursor** or another editor
- [ ] The pill never took the focus away (the text never went to the wrong window)
- [ ] **Clean**, **Prompt** and **Raw** modes all worked (Prompt / Clean need Ollama)
- [ ] Right Command + C still copies normally (the dictation is cancelled)
- [ ] **Start at login** worked after logging out and back in
- [ ] Opening PromptFlow from Spotlight / Launchpad worked

**Speed:** about ___ seconds from releasing the key to the text appearing (with mode: ___)

### What went wrong
(Describe it, and attach `promptflow.log` from the PromptFlow folder.)
