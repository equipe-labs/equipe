# CHANGELOG

One line each: version · date · what · why · platforms affected. Version rules are in `README.md`'s governance.

- **1.0.0 (2026-11-05)** — Count rule (`components.md` §5): one filled `wait` pill for the total, the breakdown as `muted` numbers + status-colour dots. No new tokens. Why: when nearly every project was waiting, the sidebar turned all orange. Affects: the equipe desktop sidebar.
- **1.0.0 (2026-11-03)** — Pictures in the conversation (T-336): in `visuals/`, a Mermaid theme (card style A, light·dark, the renderer's `Theme` values·generated Rust) and three HTML cards (status board·comparison table·number summary) + a contrast table. No new tokens. Affects: the equipe desktop (T-338), daemon rendering (T-337), mobile (T-339).
- **1.0.0 (2026-10-19)** — Moved into the repository's `design/` (T-152): T-145's sources·scripts carried over, generating the `tokens.json` the screen reads added (light·dark), the T-144 guidelines as `guidelines.md`, logo SVGs (`assets/logo/`). Values unchanged. Affects: nothing (no screen uses the generated code yet).
- **1.0.0 (2026-10-16)** — First version (T-145). Three-layer tokens, light·dark, motion·reduce motion, generators for 4 platforms, 116 contrast pairs, sheets for 9 components. Grounds: T-144 guidelines v1 + the T-145 audit.
