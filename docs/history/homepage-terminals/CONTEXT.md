# Homepage Terminals Tab — Context

**Feature slug:** homepage-terminals
**Date:** 2026-08-14
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | RUN

## Feature Boundary

Trang chủ có tab thứ ba **Terminals** bên cạnh Kanban và Projects: mở ra
một terminal agent đang chọn (màn hình sống, gõ được) kèm menu đổi sang
agent khác. Feature dừng ở trang chủ — trang terminal của từng project,
trang Transcript, và luồng tạo pane/agent giữ nguyên không đổi.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Thanh tab trang chủ thành ba mục: Kanban \| Projects \| Terminals. Terminals là liên kết thật `/?tab=terminals`, chỉ dựng phần đang chọn — cùng khuôn với hai tab hiện có. | — |
| D2 | Thân tab Terminals hiển thị **một** terminal đang chọn: màn hình sống giống trang terminal của project, phía trên là menu/dropdown liệt kê mọi terminal để đổi. Không phải danh sách phẳng, không phải lưới nhiều màn hình. | Người dùng theo dõi một agent tại một thời điểm; lưới làm chữ quá nhỏ để đọc. |
| D3 | Menu chỉ liệt kê pane **đang chạy agent** (kind khác `shell`), kể cả pane nằm ngoài mọi project đã đăng ký. Pane shell trống không xuất hiện. | Tab để theo dõi agent; shell trống là nhiễu. |
| D4 | Thứ tự trong menu và thứ tự chọn mặc định: `blocked` trước, rồi `working`, rồi phần còn lại; trong cùng nhóm giữ thứ tự ổn định (project, pane_id). | Agent đang chặn chờ người trả lời là việc cần người dùng nhất. |
| D5 | Pane đang chọn nằm trong URL: `/?tab=terminals&pane=<pane_id>`. Refresh, bookmark, nút back đều giữ đúng pane; đổi pane bằng menu là điều hướng thật, không phải state phía client. | Nhất quán với quyết định tab trang chủ là liên kết thật. |
| D6 | Tab Terminals cho gõ đầy đủ như trang terminal của project: nhập text, gửi phím, dán ảnh — dùng lại route `input`/`keys`/`attach` sẵn có, không dựng bản read-only. | Tránh nhảy trang chỉ để trả lời agent. |
| D7 | `?pane` trỏ tới pane không còn tồn tại: **không** tự nhảy sang pane khác. Thân tab báo không tìm thấy terminal đó; menu vẫn đầy đủ để người dùng tự chọn. | Tự đổi pane sẽ khiến người dùng gõ nhầm vào terminal khác. |
| D8 | Tab Terminals luôn hiện trên thanh tab, kể cả khi herdr tắt hoặc không có agent nào. Thân tab báo trạng thái rỗng tương ứng: "chưa có agent nào" và "herdr không chạy" là hai thông báo khác nhau. | Vị trí tab cố định; hai nguyên nhân rỗng khác nhau nên đọc ra khác nhau. |

### Agent's Discretion

- Nhãn từng dòng trong menu (gợi ý: `project / status kind` + title nếu có),
  kiểu điều khiển menu (`<select>` hay danh sách link), và cách bố trí khung
  màn hình trong tab — miễn giữ D2–D5.
- Dùng lại bao nhiêu code của trang terminal hiện có là quyết định của planning.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| terminal | Một pane herdr đang chạy agent. Pane shell trống không được gọi là terminal trong tab này (D3). |
| menu | Bộ chọn phía trên màn hình terminal, liệt kê mọi terminal theo thứ tự D4. |

## Specific Ideas And References

- Ảnh người dùng gửi: mũi tên chỉ vào chỗ trống bên phải "Kanban | Projects"
  trên thanh tab trang chủ, chú thích "Tab terminals" — tab mới nằm đúng
  hàng đó, sau Projects.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance/src/views.rs:139` — `HomeTab` enum (`Kanban`, `Projects`);
  cần thêm biến thể thứ ba.
- `crates/waggledance/src/views.rs:152` — `home_tab_strip`, nơi phát sinh hai
  link `/?tab=kanban` và `/?tab=projects`.
- `crates/waggledance/src/views.rs:192` — `home_page`, bộ lắp trang theo tab.
- `crates/waggledance/src/views.rs:925` — `TerminalPaneView` (pane_id, kind,
  name, status, title, cwd, workspace, tab) — struct hiển thị sẵn có.
- `crates/waggledance/src/server.rs:1498` — `terminal_page` / `terminal_page_for_pane`,
  trang terminal của project với đủ màn hình + ô nhập.
- `crates/waggledance/src/server.rs:1955` — `terminal_screen`, endpoint poll
  trả `{"text": html, "revision": n}`; 502 khi herdr chết.

### Established Patterns

- Không có template engine: HTML dựng bằng `format!` + `esc()` trong `views.rs`.
- Poll màn hình bằng JS trong `assets/app.js`, gọi endpoint `/screen`.
- `terminal_family_enabled(st)` (`server.rs:1602`) là công tắc chặn mọi lời gọi herdr.

### Integration Points

- `crates/waggledance/src/server.rs:590` — parse `?tab=` trong
  `RegisterFlagVisitor::visit_map`; giá trị lạ rơi về mặc định.
- `crates/waggledance/src/server.rs:608` — `index_page`, nơi lấy snapshot herdr
  và dựng `TerminalPaneView` cho từng project.
- `crates/waggledance/src/server.rs:2711` — `project_panes`, phép JOIN
  pane ↔ agent ↔ project; pane không khớp agent nhận `kind = "shell"`.
- `crates/waggledance/src/server.rs:2859` — `unassigned_panes`, pane ngoài mọi
  project đã đăng ký (D3 vẫn nhận nếu chúng có agent).

## Canonical References

- `crates/waggledance/src/herdr/wire.rs:22` — `AgentStatus` (`Working`,
  `Blocked`, `Done`, `Idle`, `Unknown`) — nguồn chuẩn cho thứ tự D4.

## Outstanding Questions

### Deferred To Planning

- [ ] Dùng lại `terminal_page` tới đâu (tách hàm dựng chung hay dựng riêng cho
      trang chủ) — đọc `server.rs:1498` và `views.rs` rồi quyết.
- [ ] Route `input`/`keys`/`attach` hiện gắn theo project (`/p/:id/_terminal/...`).
      Tab trang chủ có pane không thuộc project nào (D3) nên cần chọn: dùng lại
      cặp route `/_terminal/unassigned/...` sẵn có, hay mở route trang chủ theo
      pane_id. Planning quyết định, giữ nguyên D6.

## Deferred Ideas

- Lưới nhiều màn hình cùng lúc — người dùng loại ở vòng shaping này.
- Danh sách phẳng mọi pane kể cả shell — loại theo D3.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
