---
created: 2026-09-08
updated: 2026-09-08
type: bug
reporter: jari
status: fixed
priority: normal
provenance: chat
source_ref: chat:2026-09-08/cleanshot-media_FMVjctLtFI
lane: unlaned
commits:
- hash: 23a91be
  summary: enforce bundled skill description limit
- hash: 1c4ab95
  summary: align skill limit test with Pi
closed: 2026-09-08
---

# Bundled shipshape-changelog description exceeds Pi limit

## Description

/Users/jari/Library/Application Support/CleanShot/media/media_FMVjctLtFI/CleanShot 2026-09-08 at 12.04.44@2x.png Tällaista tulee. Voisi fixata.

Tästä ehkä oma pikkuisse tai sitten korjaat suioraan. Joka tapausess pitäsii sitten ajaa /skill:triage-unlaned-issues ja /skill:stint-start kun tirage on valmis. Ton pikkuissen voi joka tpaiusessa korjata smana tien

## Resolution

### 2026-09-08T09:31:14Z · @issuectl

Shortened the rendered shipshape-changelog description to 853 UTF-16 code units and added a catalog-wide rendered-frontmatter regression test for Pi's 1024-unit limit. The exact repository green gate and required LLM review/assessment passed.
