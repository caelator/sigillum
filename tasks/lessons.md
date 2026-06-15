# Lessons

- Validate embedded daemon UI changes in a real browser after CSP or interaction changes; passing tests alone does not prove the page is actually clickable.
- Update `scripts/check-architecture.sh` in the same commit as any restructuring it encodes; the script fails fast, so one stale entry can mask every violation behind it (the operator-console rebuild left four line-cap violations and fourteen missing-file references undetected until the next full release-gate run).
- Hide operator tools during first-run setup so the interface only shows the next meaningful action instead of overwhelming the user.
- When the setup flow hits hardware-key failures, show recovery guidance in user terms instead of surfacing raw CTAP error text.
- Do not surface raw CTAP or HID error strings in the daemon UI; FIDO2 failures need direct recovery guidance such as unplugging and reinserting the key when PIN auth is temporarily blocked.
- If the hardware stack can resolve a first-run device onboarding blocker locally, build that recovery path into the daemon wizard instead of sending users out to vendor tooling.
- Distinguish “daemon still has unlocked compartments” from “this browser tab still has a valid session”; otherwise the UI can report locked while unlock routes report already unlocked and strand the user.
- Preset names and thresholds must stay truthful through onboarding; if a plan implies two or three enrolled keys, the wizard must either collect them or explicitly warn that the higher-threshold lanes are not usable yet.
- Every first-run and locked-out state needs an obvious recovery path in the same screen; snapshot restore and destructive reset cannot be buried only in post-unlock operator surfaces.
- Persistent navigation in the daemon UI must stay subordinate to the main surface; a workspace map should never become a dominant rail that covers the background treatment or competes with the hero and primary workflow.
- Embedded daemon UI auth cannot rely solely on `sessionStorage`; keep an in-memory session-token fallback so unlock still works in webviews or privacy-restricted browsers where storage access is unavailable.
- Multi-key FIDO flows cannot bind to the first HID device in enumeration order; resolve existing authenticators by credential across attached devices and fail with explicit target-key guidance when a new-key step is still ambiguous.
- Sigillum UI passes should bias toward minimalist surfaces, intuitive task flow, and depth from layering and contrast; if a redesign feels loud or self-conscious, strip it back before shipping.
- Treat Sigillum's north star as local-on-your-computer software, not an eventual internet-facing or remote-service product; audits and readiness reviews must judge it against that local-only boundary.
- Navigation that only scrolls a long operator page is not enough for Sigillum; major workflows need distinct, self-explanatory sections with clear purpose, usage guidance, and visible "what do I do next?" affordances so operators can understand the system they built without reading code.
- Do not force FIDO2 PIN entry when trusted hardware-key possession is already the intended boundary; Sigillum should default to touch-only flows and ask for a current PIN only when a specific authenticator actually requires one.
