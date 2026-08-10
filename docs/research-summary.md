# Research Summary: Community Ham Radio Logging Best Practices

**Date**: January 31, 2026  
**Research Period**: Session analyzing top 10 GitHub ham radio logger projects  
**Key Deliverable**: YAHAML positioned to avoid re-inventing wheels and adopt proven patterns

---

## What We Learned

### The Community has Solved These Problems

We analyzed **5 major open-source projects**:
- **FDLog_Enhanced** (16 ⭐) - Distributed field day logger, 40+ year history
- **Hamledger** (9 ⭐) - Modern TypeScript/Electron everyday logger
- **dolphinlog** (8 ⭐) - Minimalist Python + SQLite + ADIF export
- **pycslog** (3 ⭐) - Client-server contest logger
- **logXchecker** (14 ⭐) - Contest log validator & cross-checker

### Common Patterns Across All Projects

1. **Deduplication is a core feature, not an afterthought**
   - All use some form of `(call, band, date, mode)` unique key
   - FDLog_Enhanced: Distributed dupe checking with fallback sync
   - pycslog: Server-side dupe list broadcast to clients

2. **Multi-source ingest is essential**
   - WSJT-X auto-logging (FT8, FT4)
   - Fldigi integration (PSK, RTTY, etc.)
   - Hamlib rig control (real-time frequency/mode)
   - Manual UI entry as fallback
   - **Result**: One QSO logged in many ways; dedup handles conflicts

3. **Export formats matter**
   - ADIF-3 (.adi): eQSL, LoTW, QRZ compatibility
   - CABRILLO (.log): ARRL, CQ contest submission
   - Without these, users can't escape vendor lock-in

4. **Real-time updates are expected**
   - FDLog: Network sync across nodes
   - Hamledger: Instant UI updates
   - **Problem**: Polling is slow; broadcast is expected

5. **Raw data preservation is crucial**
   - Store original format of each source (TCP, UDP, WSJT-X, etc.)
   - Enables audit, dispute resolution, format conversion
   - FDLog_Enhanced & Hamledger both do this

---

## YAHAML's Current Position

### What We're Doing Right ✅

1. **LogEntry model** is already ahead of industry standard
   - Mode-aware dedup (not just band)
   - Contest/club scoping (not just global)
   - Multi-source tracking (`source` field)
   - Raw payload storage

2. **Multi-source ingest** is architected correctly
   - TCP relay (N3FJP protocol)
   - UDP broadcast listener
   - UI form entry
   - Hamlib placeholder (ready for integration)

3. **Dedup logic** is sound
   - Hash-based dedupeKey
   - Unique constraint prevents duplicates
   - Handles contest/club scoping

### What We're Missing 🎯

| Gap | Impact | Effort | Timeline |
|-----|--------|--------|----------|
| No export (ADIF-3, CABRILLO) | Users locked in YAHAML | Medium | Week 3 |
| No merge/conflict handling | Disputed dupes have no audit trail | Medium | Week 2 |
| No aggregation tables | Scoreboard requires DB scan | Medium | Week 4 |
| No real-time push | UI polling inefficient | Medium-High | Week 5 |
| No contest templates | Must build from scratch each time | High | Week 8 |
| No WSJT-X / Hamlib auto-logging | Manual entry only | High | Weeks 6-8 |

---

## 3 Documents We Created

### 1. **LOGGING_BEST_PRACTICES.md** (8 KB)
   - Deep analysis of 5 major projects
   - Data model patterns (canonical QSO fields)
   - Dedup strategies & conflict resolution
   - Multi-source ingest architecture
   - Real-time event propagation
   - ADIF-3 & CABRILLO export standards
   - Common pitfalls & how to avoid them

### 2. **implementation-roadmap.md** (9 KB)
   - 8 phases of development (now → V2)
   - Phase 1: Foundation (current, mostly done)
   - Phase 2: Data Quality & Audit (merge handling)
   - Phase 3: Standard Export Formats (ADIF-3, CABRILLO)
   - Phase 4: Real-Time Aggregation (scoreboard, stats)
   - Phase 5: Real-Time Push (SSE, WebSocket)
   - Phase 6: Integration Modules (WSJT-X, Hamlib, Fldigi)
   - Phase 7: Contest Templates (ARRL, CQ, WPX rules)
   - Phase 8: Distributed Logging (multi-site sync)
   - Priority matrix & quick wins

### 3. **CODE_RECOMMENDATIONS.md** (12 KB)
   - Gap analysis vs. industry patterns
   - 6 priority code additions with full examples
   - Priority 1: Merge status tracking in schema
   - Priority 2: Merge API endpoint
   - Priority 3: ADIF-3 export endpoint
   - Priority 4: LogAggregate table for performance
   - Priority 5: Real-time scoreboard API
   - Priority 6: Server-Sent Events (SSE) stream
   - React hook for live log updates
   - Reference implementations from GitHub projects

---

## Key Insights

### 1. **We're Not Reinventing Ham Radio Logging**

FDLog_Enhanced has been in production for **40+ years** with distributed databases. Hamledger is **modern TypeScript/Electron**. Both solve the same core problems:
- Multi-operator simultaneous logging
- Dupe detection in real-time
- Export for international QSL services
- Integration with digital mode software

**Our advantage**: We can borrow their patterns without committing to their tech stack.

### 2. **The Dedup Key is Your Single Source of Truth**

Every logger uses some form of:
```
HASH(callsign, band, mode, date, time, [optional context])
```

FDLog: `(call, band, date, mode, gota_flag)`  
Hamledger: `(call, band, date, mode)`  
**YAHAML**: `(call, band, mode, date, time, contestId, clubId)` ← **More granular, better**

Why? Contests are scoped (same QSO in two contests = different entries). Clubs are scoped (same QSO logged by club A vs. club B = different). We got this right.

### 3. **Export Formats Are Non-Negotiable**

Without ADIF-3 export, users can't:
- Submit to eQSL.cc (no QSL card confirmation)
- Upload to LoTW (ARRL Logbook of the World)
- Share logs with QRZ.com profile
- Import into competitor loggers

**This is table stakes.** Users expect it.

### 4. **Real-Time is Expected, Polling is Legacy**

FDLog uses network broadcasts. Hamledger uses event streams. The community moved past polling.

Our current UI polls every 2 seconds. That's fine for **hobby use**, but for **contests** (especially Field Day where 50+ operators are active), real-time is non-negotiable.

**Solution**: Server-Sent Events (SSE) or WebSocket. Both are simple to add; no framework change needed.

### 5. **Merge Tracking Enables Dispute Resolution**

When the same QSO is entered:
- Manually via UI (operator types call)
- Via TCP relay (remote system sends it)
- Via WSJT-X auto-logging

Do we keep 1 or 3 entries? 

**FDLog_Enhanced answer**: Mark as `duplicate_of`, keep audit trail, preserve raw payloads. This allows post-contest review: "Was this a legitimate second contact (different mode/band)?" or "Accidental duplicate?"

We started this with `dedupeKey`, but we need `merge_status` to complete it.

---

## Immediate Actions (Next 4 Weeks)

### Week 1: Foundation Completion
- [ ] Run Prisma migration for merge status fields
- [ ] Verify dedupeKey algorithm includes mode
- [ ] Test TCP relay ingest with dedup
- [ ] Confirm server is stable

**Deliverable**: Migration merged to `main`

### Week 2: Data Quality
- [ ] Implement `/api/logs/merge` endpoint
- [ ] Write conflict resolution tests
- [ ] Document merge API
- [ ] UI for viewing/resolving conflicts (optional)

**Deliverable**: Merge endpoint in production

### Week 3: Portability
- [ ] Implement ADIF-3 export endpoint
- [ ] Test export with eQSL format validator
- [ ] Document ADIF fields
- [ ] Implement CABRILLO export (optional, can defer)

**Deliverable**: `GET /api/export/adif` endpoint live

### Week 4: Performance
- [ ] Add LogAggregate model
- [ ] Implement hourly aggregation trigger
- [ ] Add scoreboard endpoints
- [ ] Wire UI to scoreboard data

**Deliverable**: Live scoreboard working

---

## Decision Points for You

### Q1: WSJT-X Integration Timeline
**Option A** (V1): TCP/UDP/UI only; WSJT-X in V2  
**Option B** (V1): Include WSJT-X auto-logging  

*Recommendation*: Option A. Get export & aggregation solid first.

### Q2: Contest Scope
**Option A**: Support generic contest (any fields)  
**Option B**: Start with ARRL Field Day only  
**Option C**: Support 3-5 major contests (ARRL, CQ WW, etc.)  

*Recommendation*: Option B for V1, plan for C in V2.

### Q3: Distribution
**Option A**: Single server (cloud or on-site)  
**Option B**: LAN-only (like FDLog_Enhanced)  
**Option C**: Hybrid (cloud + offline sync)  

*Recommendation*: Option A for now. FDLog model (Option B) is Phase 8.

---

## Success Criteria for V1 (8 weeks)

By end of Q1, YAHAML should:

✅ **Core**: Handle multi-source ingest (TCP, UDP, UI) with robust dedupe  
✅ **Quality**: Merge conflicting entries with audit trail  
✅ **Portability**: Export ADIF-3 format (eQSL compatible)  
✅ **Performance**: Live scoreboard without DB scans  
✅ **UX**: Real-time dupe alerts (no polling)  

By these metrics, we're production-ready for:
- Club-level contests (50-200 operators)
- Multi-venue coordinated events
- Solo operator tracking

---

## Resources & References

### GitHub Projects
- FDLog_Enhanced: https://github.com/scotthibbs/FDLog_Enhanced
- Hamledger: https://github.com/valibali/hamledger
- dolphinlog: https://github.com/xaratustrah/dolphinlog
- pycslog: https://github.com/neilmb/pycslog
- logXchecker: https://github.com/ciorceri/logXchecker

### Standards
- ADIF-3 Spec: http://adif.org/
- CABRILLO Format: http://www.arrl.org/cabrillo
- Hamlib Docs: https://hamlib.github.io/
- N3FJP Protocol: (Documented in FDLog_Enhanced source)

### Community
- ARRL (American Radio Relay League): https://www.arrl.org/
- eQSL: https://www.eqsl.cc/
- Logbook of the World (LoTW): https://lotw.arrl.org/

---

## Next Steps

1. **Review these 3 documents** with the team
2. **Approve Phase 1-3 roadmap** (foundation, merge, export)
3. **Start Phase 2** (merge status) this week
4. **Assign owners** for each priority code change
5. **Schedule Phase 3** sprint planning (ADIF-3)

---

**Compiled by**: Research Agent  
**Research Period**: 2026-01-31 evening  
**Confidence Level**: High (multi-project consensus)  
**Next Review**: After Phase 2 completion  

---

## One-Page Summary for Leadership

**Problem**: YAHAML is building a ham radio logger. What should we learn from established projects?

**Solution**: Analyzed top 5 open-source ham radio loggers (FDLog, Hamledger, dolphinlog, pycslog, logXchecker).

**Key Findings**:
1. YAHAML's LogEntry model is already ahead of industry (mode-aware, contest-scoped dedup)
2. Missing: Export formats (ADIF-3), merge/conflict handling, aggregation, real-time push
3. These are solved problems; we should borrow solutions vs. reinvent

**Deliverables**: 3 documents (LOGGING_BEST_PRACTICES, IMPLEMENTATION_ROADMAP, CODE_RECOMMENDATIONS) with full code examples and timeline.

**Timeline**: 8 weeks to production-ready system (V1).

**Next Action**: Prioritize Phase 1-3 (merge handling → ADIF-3 export) in next sprint.

---

**End of Research Summary**
