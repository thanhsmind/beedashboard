# Spec nguồn: bee PM Dashboard (bản người dùng cung cấp, 06/08/2026)

> Đây là bản gốc do chủ dự án đưa vào, giữ nguyên văn làm tham chiếu. Nó được viết
> cho một tool Node độc lập đọc bee CLI; công việc thật của mdview là **bee-board-pm** —
> áp dụng thông tin kiến trúc và luật chú ý của spec này lên trang `/p/<project>/_bee`
> sẵn có. Chỗ nào spec này lệch với CONTEXT.md của bee-board-pm thì CONTEXT.md thắng.

---

**Phiên bản:** 1.0 · 06/08/2026
**Trạng thái:** Đã có prototype chạy thật (artifact `bee-pm-dashboard`); spec này chuẩn hoá để implement thành công cụ lâu dài.
**Đối tượng đọc:** Người implement (dev). Thuật ngữ bee giữ nguyên tiếng Anh trong code; nhãn UI tiếng Việt.

---

## 1. Mục tiêu & phạm vi

### 1.1 Mục tiêu

Một dashboard **một trang** cho người quản lý dự án (PM) nhìn vào repo dùng bee harness và trả lời được ngay 4 câu:

1. **Đã xây những gì** — backlog đã xong, đầu việc (cell) đã hoàn tất có kiểm chứng.
2. **Đang làm gì** — hạng mục (feature/lane) đang thực thi, tiến độ từng cell, ai/model nào làm.
3. **Kế tiếp là gì** — bước kế tiếp đã ghi của từng lane, PBI đang chờ.
4. **Đang kẹt ở đâu** — handoff tạm dừng, phiên gián đoạn, review tồn, nợ tri thức, chi phí model.

Người đọc **không cần biết thuật ngữ bee** — mọi khái niệm dịch sang ngôn ngữ PM (xem bảng §6.1).

### 1.2 Ngoài phạm vi (non-goals)

- **Không ghi** vào bee: dashboard là read-only tuyệt đối. Không gọi lệnh mutating, không sửa `.bee/*`.
- Không thay thế `bee status`/`bee orient` cho agent — đây là công cụ cho người.
- Không realtime đa người dùng; một máy, một repo, làm mới theo yêu cầu hoặc theo watch.
- Không authentication (chạy local).

### 1.3 Ràng buộc nền tảng

- Node.js ≥ 20, không dependency ngoài (chỉ stdlib) — collector và server đều vậy.
- Chạy được trên Windows (binary là `.bee/bin/bee.exe`) lẫn POSIX (`.bee/bin/bee`).
- Output HTML tự chứa 100%: không CDN, không webfont ngoài, CSS/JS inline (để có thể publish thành artifact hoặc mở `file://`).

---

## 2. Kiến trúc

```
┌──────────────┐   exec (read-only)   ┌────────────┐   inject JSON   ┌───────────────────┐
│ bee CLI +    │ ───────────────────► │ collector   │ ──────────────► │ renderer (template │
│ .bee/*.jsonl │                      │ collect.mjs │    data.json    │ HTML + JS thuần)   │
└──────────────┘                      └────────────┘                 └───────────────────┘
                                                                        │
                                             ┌──────────────────────────┼─────────────┐
                                             ▼                          ▼             ▼
                                      (a) file tĩnh            (b) serve.mjs     (c) artifact
                                      bee-dash.html            localhost + SSE   publish lại
```

Ba chế độ giao hàng, chung một collector và một template:

| Chế độ | Lệnh | Dùng khi |
|---|---|---|
| **(a) Snapshot tĩnh** | `bee-dash build` | Chụp trạng thái một lần, mở file hoặc gửi đi |
| **(b) Server local + auto-refresh** | `bee-dash serve` | Treo màn hình theo dõi khi swarm đang chạy |
| **(c) Artifact** | build rồi publish | Chia sẻ link đọc từ xa |

Module:

```
tools/bee-dash/
├── collect.mjs      # gom dữ liệu -> data.json (thuần Node, không dep)
├── build.mjs        # data.json + template -> bee-dash.html
├── serve.mjs        # chế độ (b): HTTP + watch + SSE   [tuỳ chọn, giai đoạn 2]
├── template.html    # skeleton + CSS tokens + render JS
└── cli.mjs          # entry: build | serve | collect
```

> **Vị trí trong repo:** nếu đưa vào repo beehive, việc này chạm mã nguồn → đi qua chuỗi bee
> (shape → gate → cells). Nếu để ngoài repo (thư mục tool riêng), không cần.

---

## 3. Nguồn dữ liệu (Data Sources)

### 3.1 Lệnh bee (tất cả read-only)

| # | Lệnh | Lấy gì | Ghi chú |
|---|---|---|---|
| S1 | `bee status --json` | phase, gates, handoff, cells summary, review, recovery, scribing_debt, capture_queue, pbi counts, tier_mix, ceiling_scarcity, workers, reservations, recent_decisions, staleness_warnings, recommended_next | Nguồn chính |
| S2 | `bee state lanes --json` | Mảng lane: feature, phase, mode, approved_gates, next_action, created_at | **Không chứa lane đang hoạt động** (xem D2) |
| S3 | `bee cells list --json` | Mảng cell: id, feature, lane, title, status, tier, behavior_change, deps, files… | Chỉ kho hiện hành (cell đã archive không có) |
| S4 | `bee state session list --json` | Mảng session: id, started_at, last_heartbeat, transcript_path, workspace_id | Gồm cả session cũ đã chết |

### 3.2 File đọc trực tiếp (read-only)

| # | File | Lấy gì | Cách đọc |
|---|---|---|---|
| F1 | `.bee/backlog.jsonl` | PBI chi tiết (id, title, status) | Fold sự kiện, xem §4.3 |
| F2 | `docs/history/<feature>/promote-proposals.md` | Đề xuất tài liệu chưa áp dụng | Chỉ kiểm tra **tồn tại file**, đếm theo feature |

### 3.3 Quirk bắt buộc xử lý (đã kiểm chứng)

1. **Dòng timing:** mọi lệnh bee in thêm `[bee] <cmd> <n>ms` **sau** JSON (stdout).
   → Lọc mọi dòng bắt đầu bằng `[bee] ` trước khi `JSON.parse`.
2. **Windows:** spawn `bee.exe` trực tiếp (`execFileSync(".bee/bin/bee.exe", args)`).
   Spawn qua `.cmd` sẽ EINVAL trên Node ≥ 18; qua shell thì phải quote — tránh.
3. **`bee backlog pbi list`** kén flag (`--status <x>` bị từ chối với mọi giá trị thử) —
   **không dùng**; fold `.bee/backlog.jsonl` thay thế (F1).
4. `maxBuffer` cho execFileSync đặt ≥ 64MB (cells list + lanes có thể lớn).
5. Mọi lệnh chạy với `cwd` = gốc repo.

---

## 4. Data model — `data.json`

Collector xuất **một object duy nhất**. Mọi trường string tự do đều bị cắt (slice) tại nguồn để file gọn (giới hạn ghi cạnh từng trường).

### 4.1 Schema

```jsonc
{
  "generated_at": "ISO8601",          // thời điểm chụp — hiển thị ở header và footer
  "bee_version": "2.2.2",             // status.onboarding.bee_version
  "phase": "swarming",                // phase của lane đang hoạt động
  "mode": "standard",
  "feature": "exec-speed",            // lane đang hoạt động
  "gates": { "context": true, "shape": true, "execution": true, "review": false },
  "gate_bypass_level": "off",         // "off" = cổng dừng đúng luật

  "route": {                          // null nếu chưa route
    "class": "feature", "lane": "standard",
    "flags": ["public-contracts", "multi-domain"],
    "product_files": 14,
    "rationale": "…"                  // slice 400
  },

  "handoff": {                        // null nếu không có
    "feature": "work-language-guard",
    "phase": "exploring",
    "kind": "pause",                  // mặc định "pause" nếu thiếu
    "written_at": "ISO8601",
    "next_action": "…"                // slice 700
  },

  "pbi": {
    "proposed": 52, "in_flight": 6, "done": 64,
    "in_flight_items":  [{ "id": "P76", "title": "…" }],   // title slice 160
    "proposed_recent":  [{ "id": "p-…", "title": "…" }]    // 6 cái mới nhất theo ts
  },

  "cells_summary": {                  // status.cells nguyên trạng
    "open": 0, "claimed": 0, "capped": 66, "blocked": 0,
    "archived": { "capped": 40, "dropped": 2, "total": 42 }
  },

  "active_feature": {
    "name": "exec-speed",
    "cells": [{ "id": "es-1", "title": "…", "status": "capped",
                "tier": "ceiling", "behavior_change": true, "lane": "standard" }],
    "scribing_debt_cells": ["es-1", "…"],
    "recommended_next": "…"           // status.recommended_next, slice 400
  },

  "lanes": [{                         // S2 + một dòng tổng hợp cho active feature (D2)
    "feature": "advisor-gate-port",
    "phase": "swarming",              // exploring|planning|swarming|compounding|compounding-complete
    "mode": "high-risk",
    "gates": { "context": false, "shape": true, "execution": true, "review": false },
    "next_action": "…",               // slice 220
    "created_at": "ISO8601",
    "cells": { "capped": 2, "open": 0, "claimed": 0, "blocked": 0, "dropped": 0, "total": 2 },
    "scribing_debt": 2,               // từ status.scribing_debt.orphaned.features
    "promote_pending": true,          // tồn tại docs/history/<f>/promote-proposals.md
    "cell_list": [{ "id": "agp-1", "title": "…", "status": "capped", "tier": null }]  // title slice 100
  }],

  "review": {                         // status.review nguyên trạng
    "candidates": { "total": 89, "unreviewed": 44, "in_review": 23, "reviewed": 0, "stale": 22 },
    "open_sessions": ["…"],
    "high_risk_unreviewed": 24
  },

  "sessions": [{                      // tối đa 12, sort last_heartbeat desc — xem D1
    "id": "7d5d4cfd",                 // 8 ký tự đầu
    "started_at": "ISO8601", "last_heartbeat": "ISO8601",
    "workspace": "main",
    "state": "active",                // active | interrupted | recent
    "lane": "hook-teeth"              // chỉ có ở state=interrupted, từ recovery
  }],

  "debt": {
    "scribing": 5,                    // cell của feature hiện tại chưa capture
    "orphaned": 29,                   // cell mồ côi (feature chưa từng scribing-run)
    "orphaned_features": [{ "feature": "hook-teeth", "cells": 6 }],
    "capture_queue": 31,              // stub chờ flush
    "promote_unapplied_features": ["advisor-gate-port", "…"]   // 15 feature
  },

  "tier_mix": { "counts": { "extraction": 0, "generation": 3, "ceiling": 4, "untiered": 5 },
                "tiered": 7, "ceilingShare": 0.57 },
  "ceiling":  { "pct": 57, "ceiling": 4, "tiered": 7 },        // null nếu không có

  "workers": [],                      // status.workers nguyên trạng
  "reservations": [],                 // status.active_reservations nguyên trạng
  "recent_decisions": [{ "ts": "ISO8601|null", "text": "…" }], // tối đa 5, slice 260
  "staleness_warnings": []
}
```

### 4.2 Quy tắc suy diễn (Derivations)

**D1 — Phân loại session.** Với mỗi session, `ageMin = (now − last_heartbeat)/60000`:

| Điều kiện (xét theo thứ tự) | state |
|---|---|
| id ∈ `status.recovery.candidates[].session_id` | `interrupted` |
| ageMin < 60 | `active` |
| ageMin < 1440 (24h) | `recent` |
| còn lại | loại bỏ khỏi output |

Session `interrupted` gắn thêm `lane` lấy từ recovery candidate cùng id (có thể `null`).

**D2 — Lane đang hoạt động phải được tổng hợp.** `bee state lanes` **không** chứa feature
đang hoạt động (nó là state toàn cục, không phải lane phụ). Nếu `status.feature` không có
trong S2 → unshift một dòng lane tự dựng: `phase/mode/gates` lấy từ status toàn cục,
`next_action` = `status.recommended_next`, `created_at` = `status.route.updated_at`.
Bỏ bước này là bug đã gặp: cột "Thực thi" thiếu chính hạng mục đang làm.

**D3 — Cell stats theo lane.** Group S3 theo `feature`, đếm theo `status`. Feature đã
`compounding-complete` thường có `cell_list` rỗng (cell đã archive) — UI phải chịu được
mảng rỗng (§6.7).

**D4 — Tiến độ active feature.** `live = cells.filter(s => s.status !== "dropped")`;
tiến độ = `capped/live.length`. **Không** tính cell `dropped` vào mẫu số.

**D5 — Nợ đúc kết theo lane.** Map từ `status.scribing_debt.orphaned.features[]`.

### 4.3 Fold PBI từ `.bee/backlog.jsonl`

File là JSONL nhiều loại sự kiện. Chỉ quan tâm dòng `kind === "pbi"`:

```
{ kind:"pbi", event:"add",    id:"p-…", title:"…", status:"proposed", ts:"…" }
{ kind:"pbi", event:"status", id:"p-…", status:"in_flight", ts:"…" }
{ kind:"pbi", event:"amend",  id:"p-…", title:"…", ts:"…" }
```

Thuật toán: quét tuần tự, `Map<id, {id,title,status,ts}>` — `add` tạo, sự kiện sau
cập nhật đè trường có mặt. Dòng parse lỗi → bỏ qua (không throw).
Đối chiếu tổng: counts sau fold phải khớp `status.pbi` (proposed/in_flight/done);
lệch thì **vẫn hiển thị số của `status.pbi`** (nguồn chân lý) và log warning ra console.

---

## 5. Quy tắc "Cần chú ý" (Attention Engine)

Danh sách sinh tự động, **xếp theo mức nặng giảm dần**, tối đa ~7 mục. Mỗi rule độc lập:

| # | Điều kiện | Mức | Tiêu đề (mẫu) | Hành động gợi ý |
|---|---|---|---|---|
| A1 | `handoff != null` | 🔴 critical | `{feature} đang tạm dừng` | "Chờ quyết định của bạn: tiếp tục hay gác lại" — kèm ngày ghi + trích `next_action` (180 ký tự) |
| A2 | `review.high_risk_unreviewed > 0` | 🟠 serious | `{n} thay đổi rủi ro cao chưa review` | "Chạy một phiên review gộp trước bản phát hành kế" |
| A3 | tồn tại session `interrupted` | 🟠 serious | `{n} phiên làm việc bị gián đoạn` | Liệt kê id + lane + thời điểm im lặng; "khôi phục hoặc đóng phiên" |
| A4 | `debt.orphaned + debt.scribing > 0` | 🟡 warning | "Nợ tri thức đang phình" | Tổng nợ + capture_queue + số feature có promote chưa áp |
| A5 | `ceiling.pct > 40` | 🟡 warning | `{pct}% đầu việc dùng model đắt nhất` | "Hạ bậc việc thường quy xuống model tiêu chuẩn" (ngưỡng 40% là luật của bee, decision 0012) |
| A6 | `cells_summary.blocked > 0` | 🔴 critical | `{n} đầu việc kẹt đỏ` | "Mỗi việc đỏ là một việc sửa-trước" |
| A7 | mỗi phần tử `staleness_warnings` (tối đa 2) | 🟡 warning | "Cảnh báo dữ liệu cũ" | Hiện nguyên văn |

Rỗng → một dòng "Không có gì cần chú ý."

KPI "Đang kẹt / chờ quyết định" = `(handoff ? 1 : 0) + cells_summary.blocked`.

---

## 6. UI Spec

### 6.1 Từ điển thuật ngữ (bắt buộc dùng nhất quán)

| bee | Nhãn UI |
|---|---|
| cell | đầu việc |
| capped (test xanh) | Xong, test xanh |
| claimed / open / blocked / dropped | Đang làm / Chờ làm / Kẹt / Bỏ |
| lane / feature | hạng mục |
| gate (context/shape/execution/review) | cổng duyệt (Bối cảnh / Định hình / Thực thi / Review) |
| phase exploring/planning/swarming/compounding/compounding-complete | Khám phá / Kế hoạch / Thực thi / Đúc kết / Hoàn tất |
| handoff kind=pause | bàn giao tạm dừng |
| session | phiên làm việc |
| scribing/capture debt | nợ ghi chép tri thức / nợ đúc kết |
| tier extraction/generation/ceiling/review | model rẻ / model thường / model đắt nhất / model review |
| PBI | mục backlog |
| worker | worker (giữ nguyên) |

### 6.2 Layout (desktop ≥ 900px)

```
┌ Header: logo lục giác + tiêu đề + "beehive · bee vX · dữ liệu {dt}" | chips trạng thái ┐
├ [1] Stepper vòng đời hạng mục đang làm (6 bước)                                        ┤
├ [2] KPI row: 5 tile                                                                    ┤
├ [3] Grid 1.9fr/1.1fr:  Đang làm gì (trái)  |  Cần chú ý (phải)                         ┤
├ [4] Kanban 6 cột (scroll ngang)                                                        ┤
├ [5] Grid 3 cột: Phiên làm việc | Backlog & review (bars) | Sức khỏe quy trình          ┤
└ Footer: nguồn dữ liệu + cách refresh                                                   ┘
```

Mobile < 900px: mọi grid về 1 cột; kanban giữ scroll ngang trong container riêng
(`overflow-x: auto` — body không bao giờ scroll ngang).

### 6.3 Design tokens

Palette hai theme, token-level (light mặc định; dark qua cả `@media (prefers-color-scheme: dark)`
với guard `:root:where(:not([data-theme="light"]))` **và** `:root[data-theme="dark"]` —
toggle của viewer phải thắng OS cả hai chiều):

| Token | Light | Dark | Vai trò |
|---|---|---|---|
| `--page` | `#f7f5f0` | `#121110` | nền trang (neutral ấm — chủ ý theo accent mật ong) |
| `--surface` | `#fefdfa` | `#1b1a17` | nền card |
| `--surface-2` | `#f1eee6` | `#232119` | nền cột kanban, track bar |
| `--ink` / `--ink-2` / `--ink-3` | `#171512` / `#5c584f` / `#8d897e` | `#f2efe8` / `#beb9ac` / `#8d897e` | chữ 3 bậc |
| `--line` / `--line-strong` | `#e3e0d6` / `#c9c5b8` | `#2e2c26` / `#44413a` | hairline |
| `--accent` | `#a06b06` | `#f0a92e` | mật ong — thương hiệu, KHÔNG dùng cho ngữ nghĩa |
| `--good` | `#0ca30c` | `#0ca30c` | text đi kèm: `#006300` light / `#4ec24e` dark |
| `--warning` | `#fab219` | `#fab219` | text: `#7a5200` / `#fcc95c` |
| `--serious` | `#ec835a` | `#ec835a` | text: `#9a3d14` / `#f2a284` |
| `--critical` | `#d03b3b` | `#d03b3b` | text: `#b02525` / `#ea7070` |
| `--info` | `#2a78d6` | `#3987e5` | text: `#1c5cab` / `#86b6ef` |

Luật màu: **màu ngữ nghĩa không bao giờ đứng một mình** — luôn kèm nhãn chữ hoặc icon
(chip có dot + text). Số liệu trong bảng dùng `font-variant-numeric: tabular-nums`.
Font: system stack (`system-ui, -apple-system, "Segoe UI", sans-serif`); id/mã dùng
`ui-monospace`.

### 6.4 Thành phần

**[1] Stepper** — 6 bước: Khám phá, Định hình, Kế hoạch, Thực thi, Ghi chép, Review độc lập.

- `done` khi gate tương ứng approved (bước 1←context, 2←shape, 3←execution, 6←review).
- Bước "now" theo map `{exploring:0, shaping:1, planning:2, swarming:3, compounding:4}` (mặc định 3).
- Note từng bước: bước 4 = `{capped}/{live} đầu việc hoàn tất`; bước 5 = `{debt.scribing} đầu việc chờ đúc kết` hoặc "Không nợ"; bước 6 = "Chạy khi bạn yêu cầu" (review là bước user-invoked — không bao giờ hiện như việc tự động).
- Visual: done = tick tròn xanh; now = nền `--accent-soft`, mark `▶`; chưa tới = số thứ tự xám.

**[2] KPI tiles** (5 cái):

| Tile | Giá trị | Phụ đề | Cảnh báo |
|---|---|---|---|
| Backlog sản phẩm | proposed+in_flight+done | "{done} đã xong · {in_flight} đang làm · {proposed} đề xuất" | — |
| Đầu việc hoàn tất có kiểm chứng | capped + archived.capped | "{capped} hiện hành + {arch} lưu trữ · {open} đang mở, {blocked} kẹt" | — |
| Đang kẹt / chờ quyết định | công thức §5 | tên feature handoff | viền trái đỏ khi > 0 |
| Tồn đọng review | unreviewed | "{high_risk} rủi ro cao · {in_review} đang review · {stale} đã cũ" | viền trái vàng khi > 0 |
| Nợ ghi chép tri thức | orphaned+scribing | "… · {capture_queue} ghi chú chờ gộp" | viền trái vàng khi > 0 |

**[3-trái] Card "Đang làm gì"** — header: tên feature + chip "Đang thực thi" + chip
`{n}/3 cổng đã duyệt` (xanh khi 3/3). Body: rationale của route (slice 400) → progress bar
(D4) → bảng cell (cột: Đầu việc | Nội dung | Model | Trạng thái; mọi cell kể cả dropped,
map nhãn §6.1) → hộp accent "Bước kế tiếp: {recommended_next}".

**[3-phải] "Cần chú ý"** — render output §5; mỗi mục: vạch dọc 4px màu mức độ + tiêu đề đậm
+ mô tả + dòng hành động (prefix ⏸/▲/●).

**[4] Kanban 6 cột:**

| Cột | Nguồn | Thẻ |
|---|---|---|
| Đề xuất | pbi | 1 thẻ tổng "Backlog sản phẩm ({proposed} ý tưởng)" + tối đa 3 thẻ PBI in-flight, note "+n khác" |
| Khám phá | lanes phase=exploring | tất cả |
| Kế hoạch | phase=planning | tất cả |
| Thực thi | phase=swarming, **active feature xếp đầu** | tối đa 5 + "+n khác" |
| Đúc kết | phase=compounding | tối đa 5 + "+n khác" |
| Hoàn tất | phase=compounding-complete, sort created_at desc | tối đa 4 + "+n khác"; header đếm kèm "{archived.total} cell lưu trữ" |

Thẻ lane: tên feature + meta line (`{capped}/{total−dropped} đầu việc`, `📝 nợ {n}`,
`📄 đề xuất chưa áp`, `⏸ tạm dừng`, `phiên này`). Border trái: accent nếu active,
đỏ nếu là feature của handoff. Thẻ focusable (`tabindex=0`, Enter/Space mở drawer).

**[5a] Phiên làm việc** — tối đa 6 dòng theo D1 (icon: ● xanh active, ◍ cam interrupted,
○ xám recent) + 1 dòng ⏸ cho handoff nếu có. Mỗi dòng: nhãn trạng thái + id mono + lane
(nếu có) / thời gian bắt đầu, nhịp tim cuối, workspace.

**[5b] Backlog & review** — 2 nhóm horizontal bars (giá trị lớn nhất = 100% track):
PBI (Đã xong=good, Đang làm=info, Đề xuất=line-strong) và Review (Chưa review=serious,
Đang review=info, Đã cũ=line-strong). Chú thích dưới: nhấn mạnh high_risk_unreviewed.
Label + số luôn hiển thị cạnh bar (không tooltip-only).

**[5c] Sức khỏe quy trình** — bảng key-value: cổng duyệt (gate_bypass_level: "off" →
"Dừng đúng luật — không tự duyệt"; khác → cảnh báo "Đang nới: {level}"), đầu việc kẹt đỏ,
xung đột tài nguyên (reservations.length), worker đang chạy, phân bậc model (4 số),
chi phí model (⚠ khi pct>40), nợ tri thức (⚠ khi >0), phiên bản bee. Dưới cùng:
3 quyết định gần nhất (text thuần).

### 6.5 Drawer chi tiết (side panel phải, 460px, ESC/veil để đóng)

3 loại nội dung theo `data-kind` của thẻ:

- **lane**: Trạng thái (giai đoạn, chế độ, 4 cổng dạng `✓/○`, ngày bắt đầu, nợ đúc kết,
  đề xuất tài liệu) → nếu là feature handoff: mục "Vì sao kẹt" = `handoff.next_action`
  nguyên văn → "Việc kế tiếp đã ghi" = `lane.next_action` → bảng `cell_list`
  (id | title | chip trạng thái); rỗng → "Chưa có trong kho hiện hành (có thể đã lưu trữ)."
- **backlog**: tổng quan 3 số → danh sách in_flight_items đầy đủ → proposed_recent.
- **pbi**: id + title đầy đủ.

`prefers-reduced-motion: reduce` → tắt transition trượt.

### 6.6 An toàn nội dung

Mọi chuỗi từ dữ liệu đi qua `esc()` (escape `& < > "`) trước khi vào innerHTML — title
cell/PBI là văn bản tự do. Khi inject `data.json` vào template: thay marker
`/*__DATA__*/null`, và **escape `<` thành `<`** trong JSON để chuỗi chứa
`</script>` không phá thẻ script.

### 6.7 Trạng thái rỗng / thiếu

| Tình huống | Hành vi |
|---|---|
| `handoff = null` | Không chip đỏ header, KPI kẹt = blocked, không mục A1, không dòng ⏸ |
| `route = null` | Card đang làm bỏ đoạn rationale |
| `cell_list` rỗng | Thông báo "có thể đã lưu trữ" (§6.5) |
| `ceiling = null` | Bỏ A5 và dòng chi phí model |
| Cột kanban rỗng | Cột vẫn hiện, min-height 120px |
| Attention rỗng | "Không có gì cần chú ý." |
| Lệnh bee fail (exit ≠ 0 / parse lỗi) | Collector **fail toàn phần** với message nêu lệnh hỏng — không render dashboard nửa dữ liệu |

---

## 7. Chế độ (b): serve + auto-refresh — giai đoạn 2

- `serve.mjs`: HTTP server stdlib, port mặc định 8791 (`--port`).
  - `GET /` → build on-the-fly (template + data mới nhất trong RAM).
  - `GET /api/data` → data.json hiện tại (`Content-Type: application/json`).
  - `GET /events` → SSE; đẩy `event: refresh` khi dữ liệu mới.
- Watch: `fs.watch` trên `.bee/state.json`, `.bee/cells.jsonl` (hoặc store cell tương ứng),
  `.bee/backlog.jsonl`, `.bee/decisions.jsonl` — **debounce 2s** (bee ghi theo cụm),
  rồi chạy lại collector. Collector lỗi → giữ data cũ, đẩy `event: stale` kèm message;
  client hiện banner vàng "Dữ liệu tạm cũ, thu thập lại thất bại: …".
- Client: `new EventSource("/events")` → on refresh: fetch `/api/data` và re-render
  (mọi hàm render phải là hàm thuần nhận `D` — không đọc DOM cũ).
- Header thêm dot "live" (xanh khi SSE mở, xám khi mất kết nối) + giờ dữ liệu.

---

## 8. CLI

```
bee-dash collect [--repo <path>] [--out data.json]
bee-dash build   [--repo <path>] [--out bee-dash.html]     # collect + inject
bee-dash serve   [--repo <path>] [--port 8791]             # giai đoạn 2
```

- `--repo` mặc định: cwd. Xác định binary: `<repo>/.bee/bin/bee.exe` (win32) hoặc
  `…/bee` (POSIX); không tồn tại → lỗi rõ ràng "repo này chưa cài bee".
- Exit code ≠ 0 khi collect fail. Thời gian collect kỳ vọng < 1s (đo thật: ~0.5s).

---

## 9. Tiêu chí nghiệm thu (Acceptance)

Chạy trên repo beehive thật:

1. `build` tạo file HTML **một file tự chứa**, mở `file://` hiển thị đủ 7 khu vực §6.2, không lỗi console.
2. Cột "Thực thi" chứa feature đang hoạt động, xếp đầu, border accent (bẫy D2).
3. Tiến độ active feature không đếm cell `dropped` (bẫy D4).
4. Chip cổng hiện `3/3 cổng đã duyệt` khi context+shape+execution đều approved dù review=false.
5. Có handoff → chip đỏ header, mục A1 đứng đầu "Cần chú ý", thẻ lane tương ứng border đỏ, drawer có "Vì sao kẹt".
6. Số PBI trên KPI khớp `bee status --json` → `pbi`.
7. Mọi title chứa `<`, `>`, `&`, `"` render như văn bản (không vỡ HTML); JSON chứa `</script>` không phá trang.
8. Dark/light: đổi OS theme và đổi `data-theme` trên root — cả hai chiều đều đúng token.
9. Thu nhỏ 375px: không scroll ngang toàn trang; kanban scroll trong container của nó.
10. Keyboard: Tab đến thẻ kanban, Enter mở drawer, ESC đóng; focus có viền thấy được.
11. Không có lệnh mutating nào được gọi (soát: collector chỉ dùng 4 lệnh §3.1 + đọc 2 file §3.2).
12. (Giai đoạn 2) `bee cells finish` chạy ở terminal khác → dashboard tự refresh trong ≤ 5s.

---

## 10. Rủi ro & lưu ý version

- **Contract JSON của bee không phải API công khai** — cấu trúc `status --json` có thể đổi
  giữa các bản bee. Collector nên phòng thủ: mọi truy cập sâu dùng optional chaining +
  default; thiếu khối nào thì khu vực UI tương ứng ẩn thay vì crash. Ghi `bee_version`
  vào data.json để debug lệch schema.
- Dòng `[bee] …ms` là format hiện tại (v2.2.x) — lọc theo prefix `[bee] `, đừng regex chặt hơn.
- `.bee/backlog.jsonl` là append-only nhưng **schema sự kiện lẫn lộn** (nhiều `kind`/`type`
  lịch sử) — fold chỉ tin dòng `kind === "pbi"`, mọi dòng khác bỏ qua im lặng.
- Nếu publish artifact: dữ liệu chứa tên feature/title cell nội bộ — mặc định artifact
  private; cân nhắc trước khi share ra ngoài.
