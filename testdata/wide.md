---
source: docs/appendix-a.xlsx
origin: NAS /mnt/server/Upgrade/
tags:
  - hardware
  - wishlist
date: 2025-12-22
---

# Wide table

A normal paragraph stays at a comfortable reading width in the default mode, even when the window is very wide, because long lines are hard to read.

| Id | Feature | Description | Type | Priority | Reasoning | Coordination | Notes | Owner | Status |
|----|---------|-------------|------|----------|-----------|--------------|-------|-------|--------|
| 3 | Badge reader | Read employee badge for login/logoff or debug access | Hardware | High | Removes need for a CPU key to restrict access | No. Include in OCU replacement. | Part of new board, reader antenna to enclosure | Team A | Open |
| 4 | Common local data bus | Implement an existing standard so additional components can be dropped in | Hardware | High | Allows easier expansion and replacements | No. Full access to backplane. | Update to core would remove this entirely | Team B | Open |
