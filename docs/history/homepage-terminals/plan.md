# Homepage Terminals Tab — Plan

**Feature:** homepage-terminals · **Lane:** standard · **Route:** class=feature,
flags=1 (covered-contract-change), files=3
**Decisions:** `docs/history/homepage-terminals/CONTEXT.md` (D1–D8) — cited, never
reinterpreted.

## What gets built

Tab thứ ba trên trang chủ mở một terminal agent đang chọn, gõ được, kèm menu đổi
agent. Không có endpoint mới: mọi thao tác đọc/ghi đi qua route terminal đã có
(`/p/:id/_terminal/:pane/…` cho pane thuộc project, `/_terminal/unassigned/:pane/…`
cho pane ngoài project) — kèm theo cả hai guard sẵn có.

## Technical design

Ba quyết định kỹ thuật, đều suy ra từ D1–D8 + code hiện có:

1. **Không mở route mới.** `verify_pane_is_unassigned` (`server.rs:3160`) và guard
   in-boundary của route project là hai nửa phân hoạch trọn bộ pane. Trang chủ
   biết mỗi pane thuộc nhóm nào tại lúc dựng HTML, nên nó phát ra sẵn *base path*
   đúng cho từng pane. Mở route mới theo pane_id sẽ phải viết lại guard thứ ba —
   đắt hơn và là bề mặt bảo mật mới.
2. **`data-term-base` trên phần tử màn hình.** `assets/app.js:881` và `:1405` hiện
   dựng URL từ `data-project-id`, thứ trang chủ không có (pane có thể thuộc project
   bất kỳ hoặc không thuộc project nào — D3). Thêm một nhánh: có `data-term-base`
   thì dùng nó làm tiền tố, không có thì giữ nguyên đường cũ. Trang project và trang
   Unassigned không đổi hành vi.
3. **Kho pane của tab dựng riêng.** `index_page` (`server.rs:640`) chỉ tính
   `project_panes` từng project cho badge, chưa từng gọi `unassigned_panes`. Tab
   Terminals gọi thêm `unassigned_panes` rồi lọc `kind != "shell"` (D3), sắp theo
   D4, và mỗi mục nhớ base path của mình.

CSS dùng lại `PROJECT_TAB_STYLE` (`views.rs:552`) — inject vào trang chủ như
`terminal_page` đang làm; không hoisting sang `app.css` trong feature này.

## Slices

### Slice 1 — walking skeleton (cell 1)

Tab thật, menu thật, màn hình sống thật, chỉ chưa gõ được.

- `HomeTab::Terminals` + parse `?tab=terminals` + `?pane=` (D1, D5).
- Tab thứ ba trên `home_tab_strip`, luôn hiện (D8); 5 test tab-strip hiện có
  (`server.rs:14719`, `:14771`, `:14825`, `:14852`, `:14883`) cập nhật theo.
- Kho pane agent: `project_panes` mọi project + `unassigned_panes`, lọc bỏ
  `kind == "shell"` (D3), sắp `blocked > working > còn lại`, trong nhóm giữ thứ tự
  ổn định (D4).
- Chọn pane: `?pane` khớp thì lấy; không khớp thì báo "terminal này không còn"
  và **không** tự đổi (D7); không có `?pane` thì lấy đầu danh sách D4.
- Rỗng: "chưa có agent nào" và "herdr không chạy" là hai thông báo khác nhau (D8).
- Menu đổi pane = link thật `/?tab=terminals&pane=<id>` (D5).
- Màn hình sống: dựng lại khung `.term-screen` + `data-term-base`, `app.js` đọc
  base để poll `/screen`.

### Slice 2 — gõ được (cell 2)

- Ô nhập, nút phím, dán ảnh trong tab, POST qua base path (D6) — hoàn tất D2.
- `app.js` input/keys/attach dùng chung nhánh `data-term-base`.

## Test scoping

`commands.test` = `cargo test --workspace`, chạy ở mỗi lần cap.

Đã có sẵn (chỉ trích dẫn, không viết lại): tab-strip (5 test trên), pane strip và
reply bar của trang project (`server.rs:12373`, `views.rs:5166`), guard
unassigned (`server.rs:15672`).

Khoảng trống phải viết:
- `?tab=terminals` dựng thân Terminals, không dựng Kanban/Projects.
- Menu bỏ pane shell, giữ pane agent kể cả pane unassigned (D3).
- Thứ tự D4: blocked trước working trước phần còn lại.
- `?pane` trỏ pane đã chết: báo không tìm thấy, menu vẫn đủ, KHÔNG đổi pane (D7).
- Hai trạng thái rỗng khác nhau, tab vẫn hiện (D8).
- `data-term-base` đúng cho pane thuộc project và pane unassigned.
- Slice 2: reply bar/keys có mặt và trỏ đúng base (D6).

## Cost if the shape is wrong

Sai ở chỗ base path thì tab đọc/ghi nhầm pane — guard phía server vẫn chặn
(hai guard đã phân hoạch), nên hậu quả là 404, không phải gõ nhầm terminal.
Sai ở D4/D7 chỉ là chọn nhầm mặc định, sửa trong một cell.

## Rollback

Không có migration, không có state mới. Gỡ tab = trả `HomeTab` về hai biến thể
và bỏ nhánh `data-term-base` trong `app.js`; route và guard không hề đổi.
