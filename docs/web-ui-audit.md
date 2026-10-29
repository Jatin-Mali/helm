# Web console audit — 2026-10-29

Scope: embedded dashboard, before the refine-and-polish pass. Operate mode: Linux operators investigating incidents. Evidence: release binary, isolated XDG state and stub systemctl, Webcmd Chromium at 1440 and 390 pixels. Screenshots are local Webcmd artifacts, deliberately untracked.

| Dimension | Before / 4 | Finding |
|---|---:|---|
| Accessibility | 2 | Global Enter handler overrides native buttons; search inputs lack names; focus lost when content is replaced. |
| Performance | 3 | One self-contained document; unnecessary background grid/blur and decorative metric sparklines. |
| Responsive | 2 | Queue expands to the whole incident count before content on mobile; console requires several screens of scrolling before actions. |
| Theming | 1 | Dark-only, many hard-coded translucent backgrounds, no reduced-motion handling. |
| Integrity | 2 | Rounded nested cards compete with evidence, 39 inline-style sites, unrelated metric history appears under status cards. |
| Total | 10/20 | Significant work needed. |

P1: preserve native keyboard activation and modal focus; keep pending actions busy across snapshot replacement; fix filter focus loss.
P2: bound the incident queue on narrow screens, improve contrast/theming, expose the plan action beside its incident and explicit generation state.
P3: flatten nested containers, use consistent spacing, remove decorative chart/history associations.

Direction: dense operator console with neutral solid surfaces, restrained teal selection, semantic severity colors, compact operational summary, incident evidence followed by verify/fix sections. System sans for controls, mono for commands and measurements. No external assets or build dependencies.

Ruling: design-taste-frontend explicitly excludes dashboards and its framework/package defaults conflict with the plan. Apply its audit-first discipline only; the existing self-contained HTML constraint and Impeccable Operate guidance govern the implementation. No new product features or invented claims.

## Final pass (Claude, after Codex's implementation)

Evidence: debug build on the real host (40 live findings), Webcmd Chromium at 1440×900 and 390×844, light and dark. No horizontal overflow at either width.

Fixed in this pass:
- Live findings showed `open n/a · seen n/a` everywhere: they have no persisted history. Last seen now falls back to the observation time (server, `live_findings_carry_last_seen`); unknown first-seen says so instead of `n/a`.
- Evidence summary was a bare number (`331`); it now pairs the value with its note.
- Status chips were styled like the adjacent button; they are now borderless inline labels with dividers, so the one real action (refresh) is the only button in the strip.
- Removed the header's leftover `Plan for selected incident Alt+G` hint (the Generate plan control lives in the incident detail) and the static "What changed before it broke" card, which described the console rather than the incident.
- Phones: the incident queue now precedes the metric summary, so the first screen shows incidents instead of five metric cards.

Detector: `impeccable detect` reports only `cramped-padding` on the status chips. Intentional exception: they are borderless inline labels, not boxes.

| Dimension | After / 4 |
|---|---:|
| Accessibility | 3 |
| Performance | 3 |
| Responsive | 3 |
| Theming | 3 |
| Integrity | 3 |
| Total | 15/20 |

Not verified: physical touch devices and screen readers.
